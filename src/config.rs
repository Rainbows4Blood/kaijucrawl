use crate::domain::{Error, Result};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, time::Duration};
use zeroize::{Zeroize, ZeroizeOnDrop};

pub fn logical_cpu_count() -> usize {
    std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
}

/// Internal runtime policy. Never deserialized from user settings.
#[derive(Clone, Debug, Serialize)]
pub struct RuntimeSettings {
    pub workers: usize,
    pub connect_secs: u64,
    pub read_secs: u64,
    pub total_secs: u64,
    pub attempts: i32,
    pub user_agent: String,
    pub robots_agent: String,
    pub parsers: usize,
    pub parse_secs: u64,
    pub drain_secs: u64,
    pub claim_batch: i64,
    pub save_batch: usize,
    pub flush_ms: u64,
}
impl Default for RuntimeSettings {
    fn default() -> Self {
        let workers = logical_cpu_count();
        Self {
            workers,
            connect_secs: 15,
            read_secs: 45,
            total_secs: 300,
            attempts: 5,
            user_agent: crate::targets::drivethru::USER_AGENT.into(),
            robots_agent: concat!("KaijuCrawl/", env!("CARGO_PKG_VERSION")).into(),
            parsers: workers,
            parse_secs: 30,
            drain_secs: 5,
            claim_batch: 20,
            save_batch: 10,
            flush_ms: 500,
        }
    }
}
impl RuntimeSettings {
    pub fn validate(&self) -> Result<()> {
        if self.workers == 0
            || self.parsers == 0
            || !(1..=1000).contains(&self.attempts)
            || !(1..=1000).contains(&self.claim_batch)
            || !(1..=1000).contains(&self.save_batch)
        {
            return Err(Error::Config(
                "Internal runtime worker counts must be positive; attempts/batches must be 1–1000"
                    .into(),
            ));
        }
        if [
            self.connect_secs,
            self.read_secs,
            self.total_secs,
            self.parse_secs,
            self.drain_secs,
        ]
        .iter()
        .any(|x| *x == 0 || *x > 86400)
            || self.total_secs < self.connect_secs.max(self.read_secs)
            || self.flush_ms > 86400000
        {
            return Err(Error::Config("Durations must be positive and at most one day; total must cover connect/read timeouts".into()));
        }
        if self.user_agent.trim().is_empty()
            || self.user_agent.len() > 512
            || self.user_agent.chars().any(char::is_control)
        {
            return Err(Error::Config(
                "User-Agent must be a nonempty single line".into(),
            ));
        }
        Ok(())
    }
    pub fn parse_deadline(&self) -> Duration {
        Duration::from_secs(self.parse_secs)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub selected: bool,
    #[serde(skip)]
    pub runtime: RuntimeSettings,
    pub browser: Option<crate::auth::BrowserProfile>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            selected: true,
            runtime: RuntimeSettings::default(),
            browser: None,
        }
    }
}
#[derive(Clone, Default, Zeroize, ZeroizeOnDrop, Serialize, Deserialize)]
pub struct Secrets {
    #[serde(default)]
    pub website_cookie: String,
    #[serde(default)]
    pub api_cookie: String,
}
impl Config {
    pub fn path() -> Result<PathBuf> {
        Ok(crate::paths::LocalPaths::current()?.settings())
    }
    pub fn load() -> Result<Self> {
        let paths = crate::paths::LocalPaths::current()?;
        let legacy = directories::ProjectDirs::from("", "", "kaijucrawl")
            .map(|p| p.config_dir().join("settings.toml"));
        Self::load_at(&paths, legacy.as_deref())
    }
    #[doc(hidden)]
    pub fn load_at(
        paths: &crate::paths::LocalPaths,
        legacy: Option<&std::path::Path>,
    ) -> Result<Self> {
        match std::fs::read_to_string(paths.settings()) {
            Ok(text) => toml::from_str(&text).map_err(|_| {
                Error::Config("Invalid settings.toml; repair settings or move it aside".into())
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let value = if let Some(legacy) = legacy.filter(|p| p.is_file()) {
                    toml::from_str(&std::fs::read_to_string(legacy)?).map_err(|_| {
                        Error::Config("Cannot read previous application settings".into())
                    })?
                } else {
                    Self::default()
                };
                value.save_at(paths)?;
                Ok(value)
            }
            Err(e) => Err(e.into()),
        }
    }
    pub fn save(&self) -> Result<()> {
        self.save_at(&crate::paths::LocalPaths::current()?)
    }
    #[doc(hidden)]
    pub fn save_at(&self, paths: &crate::paths::LocalPaths) -> Result<()> {
        self.runtime.validate()?;
        let text = toml::to_string_pretty(self)
            .map_err(|_| Error::Config("Cannot encode settings".into()))?;
        crate::paths::atomic_write(&paths.settings(), text.as_bytes())
    }
}
impl Secrets {
    pub fn clear_persisted_cookies() -> Result<()> {
        Self::default().persist()
    }
    pub fn load() -> Result<Self> {
        let paths = crate::paths::LocalPaths::current()?;
        if paths.secrets().exists() {
            return Self::load_at(&paths);
        }
        // One-time compatibility read only: old cloud credentials are discarded
        // by deserialization. All future persistence is inside ~/.kaijucrawl.
        let legacy = keyring::Entry::new("kaijucrawl", "credentials")
            .ok()
            .and_then(|e| e.get_password().ok());
        let saved = if let Some(mut text) = legacy {
            let value = serde_json::from_str::<Self>(&text);
            text.zeroize();
            value.map_err(|_| {
                Error::Config(
                    "Cannot migrate remembered cookies; re-enter them in Authentication".into(),
                )
            })?
        } else {
            Self::default()
        };
        saved.persist_at(&paths)?;
        Ok(saved)
    }
    #[doc(hidden)]
    pub fn load_at(paths: &crate::paths::LocalPaths) -> Result<Self> {
        let bytes = match std::fs::read(paths.secrets()) {
            Ok(bytes) => zeroize::Zeroizing::new(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e.into()),
        };
        serde_json::from_slice(&bytes).map_err(|_| {
            Error::Config("Invalid local secrets file; repair it or re-enter cookies".into())
        })
    }
    pub fn persist(&self) -> Result<()> {
        self.persist_at(&crate::paths::LocalPaths::current()?)
    }
    #[doc(hidden)]
    pub fn persist_at(&self, paths: &crate::paths::LocalPaths) -> Result<()> {
        let bytes = zeroize::Zeroizing::new(serde_json::to_vec(self)?);
        crate::paths::atomic_write(&paths.secrets(), &bytes)
    }
}
