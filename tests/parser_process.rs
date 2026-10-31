use kaijucrawl::{domain::*, parser::runtime::Runtime, targets::drivethru};
use std::time::Duration;
use tokio_util::sync::CancellationToken;
#[tokio::test]
async fn isolated_parser_roundtrip_and_cancellation() {
    let runtime = Runtime::with_executable(
        1,
        Duration::from_secs(10),
        env!("CARGO_BIN_EXE_kaijucrawl").into(),
    );
    let bytes = br#"{"id":42,"name":"Synthetic","description":"Full description"}"#.to_vec();
    let p = runtime
        .parse(
            bytes.clone(),
            "42".into(),
            drivethru::detail("42"),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(p.title, "Synthetic");
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(matches!(
        runtime
            .parse(bytes, "42".into(), drivethru::detail("42"), &cancelled)
            .await,
        Err(Error::Cancelled)
    ));
}
#[tokio::test]
async fn malformed_body_rejected_without_worker_crash() {
    let runtime = Runtime::with_executable(
        1,
        Duration::from_secs(10),
        env!("CARGO_BIN_EXE_kaijucrawl").into(),
    );
    assert!(matches!(
        runtime
            .parse(
                b"not json".to_vec(),
                "42".into(),
                drivethru::detail("42"),
                &CancellationToken::new()
            )
            .await,
        Err(Error::Job(_))
    ));
}
