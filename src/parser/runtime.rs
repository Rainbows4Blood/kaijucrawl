use crate::domain::*;
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, Read, Write},
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Semaphore,
};
use tokio_util::sync::CancellationToken;

#[derive(Serialize, Deserialize)]
struct Header {
    id: String,
    api_url: String,
    length: usize,
}
#[derive(Serialize, Deserialize)]
struct Reply {
    product: Option<Product>,
    failure: Option<Failure>,
}

/// Private pipe protocol; never a user-facing operational command.
pub fn worker() -> Result<()> {
    let mut input = std::io::BufReader::new(std::io::stdin().lock());
    let mut line = String::new();
    (&mut input).take(8193).read_line(&mut line)?;
    if line.len() > 8192 {
        return Err(Error::Config("Invalid worker header".into()));
    }
    let h: Header = serde_json::from_str(&line)?;
    if h.length > PRODUCT_LIMIT {
        return Err(Error::Config("Worker input exceeds limit".into()));
    }
    let mut body = vec![0; h.length];
    input.read_exact(&mut body)?;
    let reply = match super::extract::parse(&body, &h.id, &h.api_url) {
        Ok(p) => Reply {
            product: Some(p),
            failure: None,
        },
        Err(Error::Job(f)) => Reply {
            product: None,
            failure: Some(f),
        },
        Err(_) => Reply {
            product: None,
            failure: Some(Failure::new(FailureKind::Schema, "Parser rejected input")),
        },
    };
    serde_json::to_writer(std::io::stdout().lock(), &reply)?;
    std::io::stdout().flush()?;
    Ok(())
}
#[derive(Clone)]
pub struct Runtime {
    slots: Arc<Semaphore>,
    deadline: Duration,
    executable: std::path::PathBuf,
}
impl Runtime {
    pub fn active(&self, capacity: usize) -> usize {
        capacity.saturating_sub(self.slots.available_permits())
    }
    pub fn new(slots: usize, deadline: Duration) -> Result<Self> {
        Ok(Self {
            slots: Arc::new(Semaphore::new(slots)),
            deadline,
            executable: std::env::current_exe()?,
        })
    }
    pub fn with_executable(
        slots: usize,
        deadline: Duration,
        executable: std::path::PathBuf,
    ) -> Self {
        Self {
            slots: Arc::new(Semaphore::new(slots)),
            deadline,
            executable,
        }
    }
    pub async fn parse(
        &self,
        body: Vec<u8>,
        id: String,
        api_url: String,
        cancel: &CancellationToken,
    ) -> Result<Product> {
        let permit = tokio::select! {_ = cancel.cancelled()=>return Err(Error::Cancelled), p=self.slots.clone().acquire_owned()=>p.map_err(|_|Error::Cancelled)?};
        if body.len() > PRODUCT_LIMIT {
            return Err(Failure::new(FailureKind::Schema, "Product exceeds parser limit").into());
        }
        let mut command = tokio::process::Command::new(&self.executable);
        command
            .arg("--internal-parser-worker")
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        {
            if let Some(root) = std::env::var_os("SystemRoot") {
                command.env("SystemRoot", root);
            }
            command.creation_flags(0x08000000);
        }
        let mut child = command.spawn()?;
        let mut stdin = child.stdin.take().ok_or(Error::Cancelled)?;
        let stdout = child.stdout.take().ok_or(Error::Cancelled)?;
        let mut header = serde_json::to_vec(&Header {
            id,
            api_url,
            length: body.len(),
        })?;
        header.push(b'\n');
        let operation = async {
            let writing = async {
                stdin.write_all(&header).await?;
                stdin.write_all(&body).await?;
                stdin.shutdown().await?;
                drop(stdin);
                Ok::<(), Error>(())
            };
            let reading = async {
                let mut output = Vec::new();
                stdout
                    .take(WORKER_OUTPUT_LIMIT as u64 + 1)
                    .read_to_end(&mut output)
                    .await?;
                if output.len() > WORKER_OUTPUT_LIMIT {
                    return Err(
                        Failure::new(FailureKind::Schema, "Parser output exceeds limit").into(),
                    );
                }
                Ok(output)
            };
            let (_, output) = tokio::try_join!(writing, reading)?;
            let status = child.wait().await?;
            if !status.success() {
                return Err(Failure::new(
                    FailureKind::Schema,
                    "Parser process exited unsuccessfully",
                )
                .into());
            }
            let reply: Reply = serde_json::from_slice(&output)?;
            match (reply.product, reply.failure) {
                (Some(p), None) => {
                    p.validate()?;
                    Ok(p)
                }
                (_, Some(f)) => Err(f.into()),
                _ => Err(Failure::new(FailureKind::Schema, "Invalid parser reply").into()),
            }
        };
        let result = tokio::select! {_ = cancel.cancelled()=>Err(Error::Cancelled),result=tokio::time::timeout(self.deadline,operation)=>result.unwrap_or_else(|_|Err(Failure::new(FailureKind::ParseTimeout,"Isolated parser exceeded deadline").into()))};
        if child.try_wait()?.is_none() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        drop(permit);
        result
    }
}
