use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn server(response: Vec<u8>) -> (Url, tokio::task::JoinHandle<String>) {
    let (url, task) = server_responses(vec![response]).await;
    (
        url,
        tokio::spawn(async move { task.await.unwrap().remove(0) }),
    )
}
async fn server_responses(responses: Vec<Vec<u8>>) -> (Url, tokio::task::JoinHandle<Vec<String>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let mut requests = vec![];
        for response in responses {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut input = vec![];
            loop {
                let mut b = [0; 1024];
                let n = stream.read(&mut b).await.unwrap();
                input.extend_from_slice(&b[..n]);
                if n == 0 || input.windows(4).any(|s| s == b"\r\n\r\n") {
                    break;
                }
            }
            let _ = stream.write_all(&response).await;
            requests.push(String::from_utf8_lossy(&input).into_owned());
        }
        requests
    });
    (Url::parse(&format!("http://{addr}/product")).unwrap(), task)
}

fn assert_user_agent(request: &str, expected: &str) {
    let values: Vec<_> = request
        .lines()
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| name.eq_ignore_ascii_case("user-agent"))
        .map(|(_, value)| value.trim())
        .collect();
    assert_eq!(values, [expected]);
}

#[tokio::test]
async fn compatibility_user_agent_is_used_on_every_request_scope() {
    let agent = RuntimeSettings::default().user_agent;
    for (url, product, robots, expected) in [
        (d::detail("42"), Some("42"), false, d::USER_AGENT),
        (format!("{}robots.txt", d::API), None, true, &agent),
        (format!("{}robots.txt", d::WEBSITE), None, true, &agent),
        (d::WEBSITE.to_owned(), None, false, &agent),
        (d::ROOT.to_owned(), None, false, &agent),
        (d::API.to_owned(), None, false, &agent),
    ] {
        let url = Url::parse(&url).unwrap();
        let mut n = network(&url).await;
        let (endpoint, task) = server(response("synthetic")).await;
        n.fixture_endpoint = Some(endpoint);
        n.fetch_approved(url, product, 100, robots, &CancellationToken::new(), None)
            .await
            .unwrap();
        assert_user_agent(&task.await.unwrap(), expected);
        assert_eq!(n.settings.user_agent, agent);
    }
}

#[tokio::test]
async fn product_compatibility_preserves_kaijucrawl_robots_rules_and_pacing() {
    let url = Url::parse(&d::detail("42")).unwrap();
    for (kaiju_rule, browser_rule, allowed) in [
        ("Disallow: /", "Allow: /", false),
        ("Allow: /", "Disallow: /", true),
    ] {
        let mut n = network(&url).await;
        load_policy(
            &mut n,
            d::API,
            &format!(
                "User-agent: KaijuCrawl\n{kaiju_rule}\nCrawl-delay: 5\n\n\
                 User-agent: Mozilla\nUser-agent: Firefox\n{browser_rule}\nCrawl-delay: 0\n"
            ),
        )
        .await;
        assert_eq!(n.pacing().await.api, Some(Duration::from_secs(5)));
        if allowed {
            // The API interval is still enforced even though the wire header
            // matches a browser group that has no delay.
            let cancel = CancellationToken::new();
            assert!(
                tokio::time::timeout(
                    Duration::from_millis(40),
                    n.get_job(
                        url.as_str(),
                        Some("42"),
                        100,
                        &cancel,
                        Arc::new(|| Box::pin(async { panic!("must wait for robots pacing") }))
                    ),
                )
                .await
                .is_err()
            );
            *n.origin_pacer(d::API.trim_end_matches('/'))
                .await
                .last_start
                .lock()
                .await = Some(Instant::now() - Duration::from_secs(6));
            let (endpoint, task) = server(response("synthetic")).await;
            n.fixture_endpoint = Some(endpoint);
            n.get(url.as_str(), Some("42"), 100, &cancel).await.unwrap();
            assert_user_agent(&task.await.unwrap(), d::USER_AGENT);
        } else {
            let start: Guard =
                Arc::new(|| Box::pin(async { panic!("robots must prevent request") }));
            assert!(matches!(
                n.get_job(
                    url.as_str(),
                    Some("42"),
                    100,
                    &CancellationToken::new(),
                    start
                )
                .await,
                Err(Error::Job(Failure {
                    kind: FailureKind::Robots,
                    ..
                }))
            ));
        }
    }
}

#[tokio::test]
async fn approved_product_redirect_retains_compatibility_user_agent() {
    let redirect = b"HTTP/1.1 302 Found\r\nLocation: /api/vBeta/products/42?siteId=10&groupId=1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec();
    let (endpoint, task) = server_responses(vec![redirect, response("synthetic")]).await;
    let url = Url::parse(&d::detail("42")).unwrap();
    let mut n = network(&url).await;
    n.fixture_endpoint = Some(endpoint);
    let result = n
        .get(url.as_str(), Some("42"), 100, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(result.status, 200);
    let requests = task.await.unwrap();
    assert_eq!(requests.len(), 2);
    for request in requests {
        assert_user_agent(&request, d::USER_AGENT);
    }
}
fn response(body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}
async fn load_policy(n: &mut Network, origin: &str, body: &str) {
    let (endpoint, request) = server(response(body)).await;
    n.fixture_endpoint = Some(endpoint);
    tokio::time::timeout(
        Duration::from_secs(2),
        n.load_robots(origin, &CancellationToken::new()),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(request.await.unwrap().starts_with("GET /robots.txt "));
}

#[tokio::test]
async fn robots_pacing_is_origin_scoped_without_blocking_the_other_origin() {
    // Both directions matter; API Request-rate must also stay scoped to the API.
    for (limited, free, rule) in [
        (d::WEBSITE, d::API, "Crawl-Delay: 5"),
        (d::API, d::WEBSITE, "Request-rate: 2/10"),
    ] {
        let mut n = network(&Url::parse(d::WEBSITE).unwrap()).await;
        load_policy(&mut n, limited, &format!("User-agent: *\nAllow: /\n{rule}")).await;
        load_policy(&mut n, free, "User-agent: *\nAllow: /\n").await;
        let pacing = n.pacing().await;
        assert_eq!(pacing.workers, n.settings.workers);
        assert_eq!(
            pacing.website,
            Some(if limited == d::WEBSITE {
                Duration::from_secs(5)
            } else {
                Duration::ZERO
            })
        );
        assert_eq!(
            pacing.api,
            Some(if limited == d::API {
                Duration::from_secs(5)
            } else {
                Duration::ZERO
            })
        );

        let (endpoint, request) = server(response("synthetic response")).await;
        n.fixture_endpoint = Some(endpoint);
        let cancel = CancellationToken::new();
        let blocked_network = n.clone();
        let blocked_cancel = cancel.clone();
        let started = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let starts = started.clone();
        let mut blocked = tokio::spawn(async move {
            let on_start: Guard = Arc::new(move || {
                starts.store(true, std::sync::atomic::Ordering::SeqCst);
                Box::pin(async { Ok(()) })
            });
            blocked_network
                .get_job(limited, None, 100, &blocked_cancel, on_start)
                .await
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(40), &mut blocked)
                .await
                .is_err()
        );
        tokio::time::timeout(
            Duration::from_secs(2),
            n.get(free, None, 100, &CancellationToken::new()),
        )
        .await
        .expect("the other origin must not inherit the five-second delay")
        .unwrap();
        assert!(request.await.unwrap().starts_with("GET / "));
        assert!(!blocked.is_finished());
        assert!(
            !started.load(std::sync::atomic::Ordering::SeqCst),
            "pacing must not consume an attempt"
        );
        cancel.cancel();
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(1), blocked)
                .await
                .unwrap()
                .unwrap(),
            Err(Error::Cancelled)
        ));
    }
}

#[tokio::test]
async fn no_added_pacing_but_server_cooldowns_stay_shared_per_target() {
    let settings = RuntimeSettings::default();
    let n = Network::new(
        &settings,
        Cookies::default(),
        Arc::new(|| Box::pin(async { Ok(()) })),
    )
    .unwrap();
    let cancel = CancellationToken::new();
    n.pace_origin(d::WEBSITE, &cancel).await.unwrap();
    // No timer wait is allowed for an ordinary request without a robots delay.
    use futures_util::FutureExt;
    for origin in [d::WEBSITE, d::API, d::API] {
        assert!(matches!(
            n.pace_origin(origin, &cancel).now_or_never(),
            Some(Ok(()))
        ));
    }
    n.limiter.cooldown(60).await;
    assert!(n.pacing().await.cooldown > Duration::from_secs(50));
    for origin in [d::WEBSITE, d::API] {
        assert!(
            tokio::time::timeout(Duration::from_millis(30), n.pace_origin(origin, &cancel))
                .await
                .is_err()
        );
    }
    // A delayed target does not impose a global host-level limit on another target.
    let other = Network::new(
        &RuntimeSettings::default(),
        Cookies::default(),
        Arc::new(|| Box::pin(async { Ok(()) })),
    )
    .unwrap();
    tokio::time::timeout(
        Duration::from_millis(100),
        other.pace_origin(d::WEBSITE, &cancel),
    )
    .await
    .unwrap()
    .unwrap();
}

#[tokio::test]
async fn robots_refresh_can_remove_one_origin_delay_without_changing_the_other() {
    let mut n = network(&Url::parse(d::WEBSITE).unwrap()).await;
    load_policy(&mut n, d::WEBSITE, "User-agent: *\nCrawl-delay: 5").await;
    load_policy(&mut n, d::API, "User-agent: *\nCrawl-delay: 1").await;
    // Model an elapsed API interval before refreshing, without a wall-clock wait.
    *n.origin_pacer(d::API.trim_end_matches('/'))
        .await
        .last_start
        .lock()
        .await = Some(Instant::now() - Duration::from_secs(2));
    load_policy(&mut n, d::API, "User-agent: *\nAllow: /\n").await;
    let pacing = n.pacing().await;
    assert_eq!(pacing.website, Some(Duration::from_secs(5)));
    assert_eq!(pacing.api, Some(Duration::ZERO));
    let (endpoint, request) = server(response("synthetic")).await;
    n.fixture_endpoint = Some(endpoint);
    tokio::time::timeout(
        Duration::from_millis(500),
        n.get(d::API, None, 100, &CancellationToken::new()),
    )
    .await
    .unwrap()
    .unwrap();
    request.await.unwrap();
}

#[tokio::test]
async fn product_api_403_is_a_failure_unless_session_or_challenge_evidence_is_explicit() {
    for (body, challenge, expected) in [
        ("", false, FailureKind::Restricted),
        ("Forbidden", false, FailureKind::Restricted),
        ("Access denied", false, FailureKind::Restricted),
        (
            "<html><title>Access denied</title></html>",
            false,
            FailureKind::Restricted,
        ),
        (
            r#"{"accessDenied":true,"private":"PRIVATE_DENIED_BODY"}"#,
            false,
            FailureKind::Restricted,
        ),
        (
            r#"{"message":"Product disabled","private":"PRIVATE_DENIED_BODY"}"#,
            false,
            FailureKind::Restricted,
        ),
        (
            r#"{"adultContentRestricted":true}"#,
            false,
            FailureKind::Restricted,
        ),
        ("Age verification required", false, FailureKind::Restricted),
        (
            r#"{"loginRequired":true}"#,
            false,
            FailureKind::Authentication,
        ),
        (
            r#"{"authenticationRequired":true}"#,
            false,
            FailureKind::Authentication,
        ),
        (
            r#"{"sessionExpired":true}"#,
            false,
            FailureKind::Authentication,
        ),
        (
            r#"{"message":"session expired"}"#,
            false,
            FailureKind::Authentication,
        ),
        (
            r#"{"message":"product restricted","detail":"session expired"}"#,
            false,
            FailureKind::Authentication,
        ),
        (
            "<html><title>Just a moment...</title></html>",
            false,
            FailureKind::Authentication,
        ),
        (
            "<html><title>Access denied</title><form id='challenge-form' action='/cdn-cgi/challenge-platform/check'></form></html>",
            false,
            FailureKind::Authentication,
        ),
        (
            "<html><title>Product restricted</title><form id='challenge-form' action='/cdn-cgi/challenge-platform/check'></form></html>",
            false,
            FailureKind::Authentication,
        ),
        ("Forbidden", true, FailureKind::Authentication),
    ] {
        let header = if challenge {
            "cf-mitigated: challenge\r\n"
        } else {
            ""
        };
        let wire = format!(
            "HTTP/1.1 403 Forbidden\r\n{header}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let (endpoint, request) = server(wire.into_bytes()).await;
        let mut n = network(&Url::parse(d::API).unwrap()).await;
        n.fixture_endpoint = Some(endpoint);
        let Err(Error::Job(failure)) = n
            .get(
                &d::detail("42"),
                Some("42"),
                4096,
                &CancellationToken::new(),
            )
            .await
        else {
            panic!("HTTP 403 must not be accepted as a successful product");
        };
        assert_eq!(failure.kind, expected, "fixture {body}");
        assert_eq!(failure.status, Some(403));
        assert_eq!(failure.pauses(), expected == FailureKind::Authentication);
        assert_eq!(failure.retry_after_secs, 0);
        assert!(
            !serde_json::to_string(&failure)
                .unwrap()
                .contains("PRIVATE_DENIED_BODY")
        );
        assert!(
            request
                .await
                .unwrap()
                .starts_with("GET /api/vBeta/products/42?")
        );
    }
}

#[tokio::test]
async fn product_forbidden_handling_does_not_relax_required_robots_or_website_checks() {
    for origin in [d::WEBSITE, d::API] {
        let (endpoint, request) = server(
            b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
        )
        .await;
        let mut n = network(&Url::parse(origin).unwrap()).await;
        n.fixture_endpoint = Some(endpoint);
        assert!(
            matches!(n.load_robots(origin, &CancellationToken::new()).await, Err(Error::Job(f)) if f.pauses())
        );
        request.await.unwrap();
    }
    let (endpoint, request) = server(
        b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
    )
    .await;
    let mut n = network(&Url::parse(d::WEBSITE).unwrap()).await;
    n.fixture_endpoint = Some(endpoint);
    assert!(
        matches!(n.get(d::WEBSITE, None, 4096, &CancellationToken::new()).await, Err(Error::Job(f)) if f.pauses())
    );
    request.await.unwrap();
}
async fn network(url: &Url) -> Network {
    let settings = RuntimeSettings {
        connect_secs: 1,
        read_secs: 1,
        total_secs: 2,
        ..Default::default()
    };
    let n = Network::new(
        &settings,
        Cookies::default(),
        Arc::new(|| Box::pin(async { Ok(()) })),
    )
    .unwrap();
    n.policies.write().await.insert(
        url.origin().ascii_serialization(),
        Policy::parse(String::new(), &settings.robots_agent).unwrap(),
    );
    n
}
#[tokio::test]
async fn bounded_http_and_gzip() {
    let body = crate::storage::archive::compress(b"synthetic response").unwrap();
    let mut wire=format!("HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).into_bytes();
    wire.extend(body);
    let (url, task) = server(wire).await;
    let n = network(&url).await;
    let r = n
        .fetch_approved(url, None, 100, false, &CancellationToken::new(), None)
        .await
        .unwrap();
    assert_eq!(r.body, b"synthetic response");
    task.await.unwrap();
}
#[tokio::test]
async fn gzip_bomb_is_rejected() {
    let body = crate::storage::archive::compress(&vec![b'x'; 10000]).unwrap();
    let mut wire=format!("HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).into_bytes();
    wire.extend(body);
    let (url, task) = server(wire).await;
    let n = network(&url).await;
    assert!(
        n.fetch_approved(url, None, 100, false, &CancellationToken::new(), None)
            .await
            .is_err()
    );
    task.await.unwrap();
}
#[tokio::test]
async fn failures_classified_and_retry_after_persisted_before_return() {
    let (url,task)=server(b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nRetry-After: 120\r\nConnection: close\r\n\r\n".to_vec()).await;
    let saved = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let s = saved.clone();
    let n = network(&url).await.with_cooldown(Arc::new(move |seconds| {
        s.store(seconds, std::sync::atomic::Ordering::SeqCst);
        Box::pin(async { Ok(()) })
    }));
    let Err(Error::Job(f)) = n
        .fetch_approved(url, None, 100, false, &CancellationToken::new(), None)
        .await
    else {
        panic!()
    };
    assert_eq!(f.kind, FailureKind::Transient);
    assert_eq!(f.retry_after_secs, 120);
    assert_eq!(saved.load(std::sync::atomic::Ordering::SeqCst), 120);
    task.await.unwrap();
}
#[tokio::test]
async fn robots_denial_never_starts_request_or_attempt() {
    let url = Url::parse("http://127.0.0.1:1/product").unwrap();
    let n = network(&url).await;
    n.policies.write().await.insert(
        url.origin().ascii_serialization(),
        Policy::parse("User-agent: *\nDisallow: /".into(), "KaijuCrawl").unwrap(),
    );
    let start: Guard = Arc::new(|| Box::pin(async { panic!("no actual attempt allowed") }));
    assert!(matches!(
        n.fetch_approved(
            url,
            None,
            100,
            false,
            &CancellationToken::new(),
            Some(start)
        )
        .await,
        Err(Error::Job(Failure {
            kind: FailureKind::Robots,
            ..
        }))
    ));
}
#[tokio::test]
async fn foreign_redirect_never_forwards_cookie() {
    let (url,task)=server(b"HTTP/1.1 302 Found\r\nLocation: https://unrelated.invalid/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()).await;
    let n = network(&url).await;
    let cookies = Cookies {
        raw: vec![(
            format!("{}/", url.origin().ascii_serialization()),
            "synthetic=fixture".into(),
        )],
        ..Default::default()
    };
    *n.cookies.write().await = Arc::new(cookies);
    assert!(matches!(
        n.fetch_approved(url, None, 100, false, &CancellationToken::new(), None)
            .await,
        Err(Error::Job(Failure {
            kind: FailureKind::Authentication,
            ..
        }))
    ));
    let request = task.await.unwrap();
    assert!(request.contains("synthetic=fixture"));
}
#[tokio::test]
async fn suspension_interrupts_pacing_without_start() {
    let url = Url::parse("http://127.0.0.1:1/product").unwrap();
    let n = network(&url).await;
    n.limiter.cooldown(3600).await;
    n.suspend().await;
    let now = Instant::now();
    assert!(matches!(
        n.fetch_approved(url, None, 100, false, &CancellationToken::new(), None)
            .await,
        Err(Error::Cancelled)
    ));
    assert!(now.elapsed() < Duration::from_millis(100));
}

#[tokio::test]
async fn cloudflare_challenge_header_is_not_a_success_even_with_http_200() {
    let (url, task) = server(b"HTTP/1.1 200 OK\r\ncf-mitigated: challenge\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()).await;
    let n = network(&url).await;
    let result = n
        .fetch_approved(url, None, 100, true, &CancellationToken::new(), None)
        .await;
    let Err(Error::Job(failure)) = result else {
        panic!("challenge must fail")
    };
    assert_eq!(failure.kind, FailureKind::Authentication);
    assert_eq!(failure.status, Some(200));
    assert!(failure.reason.contains("Cloudflare challenge"));
    task.await.unwrap();
}

#[tokio::test]
async fn firefox_partitioned_clearance_is_sent_on_website_and_api_robots_requests() {
    let temp = tempfile::tempdir().unwrap();
    let db = rusqlite::Connection::open(temp.path().join("cookies.sqlite")).unwrap();
    db.execute_batch("PRAGMA user_version=15; CREATE TABLE moz_cookies(name TEXT,value TEXT,host TEXT,path TEXT,isSecure INTEGER,expiry INTEGER,originAttributes TEXT); INSERT INTO moz_cookies VALUES('cf_clearance','SYNTHETIC_CLEARANCE','.drivethrurpg.com','/',1,9999999999,'^partitionKey=%28https%2Cdrivethrurpg.com%29')").unwrap();
    let snapshot = serde_json::json!({"cookies": [
        {"host": ".drivethrurpg.com", "name": "siteSettings", "value": "{\"language\":\"en\",\"currency\":\"USD\"}", "path": "/", "secure": true},
        {"host": ".unrelated.invalid", "name": "other", "value": "DO_NOT_SEND"}
    ]});
    let mut compressed = b"mozLz40\0".to_vec();
    compressed.extend(lz4_flex::block::compress_prepend_size(
        &serde_json::to_vec(&snapshot).unwrap(),
    ));
    std::fs::write(temp.path().join("sessionstore.jsonlz4"), compressed).unwrap();
    let cookies = crate::auth::import(&crate::auth::BrowserProfile {
        browser: "Firefox".into(),
        name: "Synthetic fixture".into(),
        path: temp.path().into(),
        container: 0,
    })
    .unwrap();
    for origin in [d::WEBSITE, d::API] {
        let (endpoint, captured) =
            server(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec())
                .await;
        let mut n = network(&endpoint).await;
        n.fixture_endpoint = Some(endpoint);
        n.replace(cookies.clone()).await;
        let (status, _) = n
            .load_robots(origin, &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(status, 200);
        let request = captured.await.unwrap();
        assert!(request.starts_with("GET /robots.txt "));
        assert!(request.contains("cf_clearance=SYNTHETIC_CLEARANCE"));
        assert!(request.contains("siteSettings={\"language\":\"en\",\"currency\":\"USD\"}"));
        assert!(!request.contains("DO_NOT_SEND"));
    }
}
