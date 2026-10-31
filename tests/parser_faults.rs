use kaijucrawl::{domain::*, parser::runtime::Runtime, targets::drivethru};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;
fn runtime(path: PathBuf) -> Runtime {
    Runtime::with_executable(1, Duration::from_millis(100), path)
}
#[tokio::test]
async fn failed_worker_launch_is_recoverable() {
    let p = tempfile::tempdir().unwrap().path().join("missing-worker");
    assert!(
        runtime(p)
            .parse(
                b"{}".to_vec(),
                "42".into(),
                drivethru::detail("42"),
                &CancellationToken::new()
            )
            .await
            .is_err()
    );
}
#[cfg(unix)]
#[tokio::test]
async fn hanging_worker_killed_and_reaped() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("hang");
    std::fs::write(&path, "#!/bin/sh\nexec /bin/sleep 600\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let now = Instant::now();
    let r = runtime(path)
        .parse(
            b"{}".to_vec(),
            "42".into(),
            drivethru::detail("42"),
            &CancellationToken::new(),
        )
        .await;
    assert!(matches!(
        r,
        Err(Error::Job(Failure {
            kind: FailureKind::ParseTimeout,
            ..
        }))
    ));
    assert!(now.elapsed() < Duration::from_secs(2));
}
#[cfg(windows)]
#[tokio::test]
async fn hung_worker_deadline_is_enforced() {
    // PowerShell is passed the internal protocol argument and normally exits.
    // A minimal native fixture below sleeps without reading stdin instead.
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("hang.rs");
    let exe = temp.path().join("hang.exe");
    std::fs::write(
        &source,
        "fn main(){std::thread::sleep(std::time::Duration::from_secs(600));}",
    )
    .unwrap();
    let status = std::process::Command::new("rustc")
        .arg(&source)
        .arg("-o")
        .arg(&exe)
        .status()
        .unwrap();
    assert!(status.success());
    let now = Instant::now();
    let r = runtime(exe)
        .parse(
            b"{}".to_vec(),
            "42".into(),
            drivethru::detail("42"),
            &CancellationToken::new(),
        )
        .await;
    assert!(matches!(
        r,
        Err(Error::Job(Failure {
            kind: FailureKind::ParseTimeout,
            ..
        }))
    ));
    assert!(now.elapsed() < Duration::from_secs(2));
}
