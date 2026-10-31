use super::*;
use crate::{
    config::{RuntimeSettings, Secrets},
    paths::LocalPaths,
};
use serde_json::json;
use std::{collections::HashMap, sync::Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Fixture {
    endpoint: url::Url,
}
struct StorageFaultFixture {
    base: Fixture,
    fail_once: Arc<AtomicBool>,
    paths: LocalPaths,
}
impl RuntimeFactory for StorageFaultFixture {
    fn paths(&self) -> Result<LocalPaths> {
        Ok(self.paths.clone())
    }
    fn network(&self, s: &RuntimeSettings, c: Cookies, g: Guard) -> Result<Network> {
        let fail = self.fail_once.clone();
        self.base.network(
            s,
            c,
            Arc::new(move || {
                let fail = fail.clone();
                let guard = g.clone();
                Box::pin(async move {
                    if fail.swap(false, Ordering::SeqCst) {
                        return Err(Error::Database(sqlx::Error::PoolClosed));
                    }
                    guard().await
                })
            }),
        )
    }
    fn parser(&self, s: &RuntimeSettings) -> Result<Runtime> {
        self.base.parser(s)
    }
}
struct LocalFixture {
    base: Fixture,
    paths: LocalPaths,
}
impl RuntimeFactory for LocalFixture {
    fn paths(&self) -> Result<LocalPaths> {
        Ok(self.paths.clone())
    }
    fn network(&self, s: &RuntimeSettings, c: Cookies, g: Guard) -> Result<Network> {
        self.base.network(s, c, g)
    }
    fn parser(&self, s: &RuntimeSettings) -> Result<Runtime> {
        self.base.parser(s)
    }
}
impl RuntimeFactory for Fixture {
    fn network(&self, s: &RuntimeSettings, c: Cookies, g: Guard) -> Result<Network> {
        let mut n = Network::new(s, c, g)?;
        n.fixture_endpoint = Some(self.endpoint.clone());
        Ok(n)
    }
    fn parser(&self, s: &RuntimeSettings) -> Result<Runtime> {
        let executable = std::env::current_exe()?
            .parent()
            .and_then(|p| p.parent())
            .ok_or(Error::Cancelled)?
            .join(if cfg!(windows) {
                "kaijucrawl.exe"
            } else {
                "kaijucrawl"
            });
        Ok(Runtime::with_executable(
            s.parsers,
            s.parse_deadline(),
            executable,
        ))
    }
}
async fn storefront() -> (
    url::Url,
    Arc<Mutex<HashMap<String, u32>>>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let counts = Arc::new(Mutex::new(HashMap::<String, u32>::new()));
    let c = counts.clone();
    let task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let c = c.clone();
            tokio::spawn(async move {
                let mut raw = vec![];
                loop {
                    let mut b = [0; 4096];
                    let n = stream.read(&mut b).await.unwrap();
                    raw.extend_from_slice(&b[..n]);
                    if n == 0 || raw.windows(4).any(|s| s == b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8_lossy(&raw);
                let path = request
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("")
                    .split('?')
                    .next()
                    .unwrap_or("");
                let (run, visit) = {
                    let mut counts = c.lock().unwrap();
                    *counts.entry(path.into()).or_default() += 1;
                    (
                        counts.get("/sitemap.xml").copied().unwrap_or(0),
                        counts.get(path).copied().unwrap_or(0),
                    )
                };
                let (status,body)=match path {
            "/"=>(200,br#"<!doctype html><html><title>Synthetic public website</title><body><h1>Browse games</h1><script src="/cdn-cgi/challenge-platform/scripts/jsd/main.js"></script><script>const captcha = {"message":"session expired"};</script><div hidden><h2>Login required</h2></div></body></html>"#.to_vec()),
            "/robots.txt"=>(200,b"User-agent: *\nAllow: /\nSitemap: https://www.drivethrurpg.com/sitemap.xml".to_vec()),
            "/sitemap.xml"=>(200,b"<sitemapindex><sitemap><loc>https://www.drivethrurpg.com/nested.xml.gz</loc></sitemap><sitemap><loc>https://www.drivethrurpg.com/sitemap.xml</loc></sitemap><sitemap><loc>https://evil.invalid/never.xml</loc></sitemap></sitemapindex>".to_vec()),
            "/nested.xml.gz"=>{let xml=format!("<urlset>{}</urlset>",[42,42,43,44,46,47,48,49,50].iter().map(|id|format!("<url><loc>https://www.drivethrurpg.com/en/product/{id}</loc><lastmod>{run}</lastmod></url>")).collect::<String>());(200,crate::storage::archive::compress(xml.as_bytes()).unwrap())},
            "/api/vBeta/products/42"=>(200,serde_json::to_vec(&json!({"id":42,"title":"Good","description":"Full"})).unwrap()),
            "/api/vBeta/products/43"=>(200,serde_json::to_vec(&json!({"id":43,"title":"Initially incomplete","description":if visit>=2{"Now complete"}else{""}})).unwrap()),
            "/api/vBeta/products/44" if !request.contains("fixture=authorized")=>(401,b"session expired".to_vec()),
            "/api/vBeta/products/44"=>(200,serde_json::to_vec(&json!({"id":44,"title":"Cookie scoped","description":"Full"})).unwrap()),
            "/api/vBeta/products/46"=>(500,b"synthetic server failure".to_vec()),
            "/api/vBeta/products/47"=>(200,b"{malformed}".to_vec()),
            "/api/vBeta/products/48" if visit == 1 => (403, br#"{"message":"Product disabled","private":"PRIVATE_DENIED_BODY"}"#.to_vec()),
            "/api/vBeta/products/49" if visit == 1 => (403, br#"{"accessDenied":true}"#.to_vec()),
            "/api/vBeta/products/50" if visit == 1 => (403, br#"{"adultContentRestricted":true}"#.to_vec()),
            "/api/vBeta/products/48" | "/api/vBeta/products/49" | "/api/vBeta/products/50" => (200, serde_json::to_vec(&json!({"id":path.rsplit('/').next().unwrap(),"title":"Now available","description":"Full"})).unwrap()),
            _=>(404,b"missing".to_vec()),
        };
                let wire = format!(
                    "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(wire.as_bytes()).await;
                let _ = stream.write_all(&body).await;
            });
        }
    });
    (
        url::Url::parse(&format!("http://{address}/")).unwrap(),
        counts,
        task,
    )
}
#[tokio::test]
async fn access_reports_partial_outcomes_without_discovery_or_authentication_claims() {
    let (endpoint, counts, server) = storefront().await;
    let fixture = Fixture { endpoint };
    let config = Config {
        runtime: RuntimeSettings {
            ..Default::default()
        },
        ..Default::default()
    };
    let ok = test_access_with(&config, Cookies::default(), Some("42".into()), &fixture)
        .await
        .unwrap();
    assert!(ok.passed);
    assert!(ok.text.contains("Cookies loaded: no"));
    assert!(ok.text.contains("API reachable: HTTP 200"));
    assert!(ok.text.contains("Authenticated access: unverified"));
    assert!(ok.text.contains("identity and completeness verified"));
    let bad = test_access_with(&config, Cookies::default(), Some("47".into()), &fixture)
        .await
        .unwrap();
    assert!(!bad.passed);
    assert!(bad.text.contains("Website reachable: HTTP 200"));
    assert!(bad.text.contains("API reachable: HTTP 200"));
    assert!(bad.text.contains("Product response: Schema"));
    assert!(!counts.lock().unwrap().contains_key("/sitemap.xml"));
    server.abort();
}

#[tokio::test]
async fn access_without_product_id_accepts_ordinary_homepage_and_reports_untested_api() {
    let (endpoint, counts, server) = storefront().await;
    let config = Config {
        runtime: RuntimeSettings {
            ..Default::default()
        },
        ..Default::default()
    };
    let report = test_access_with(&config, Cookies::default(), None, &Fixture { endpoint })
        .await
        .unwrap();
    assert!(report.passed);
    assert!(report.text.contains("Website reachable: HTTP 200"));
    assert!(report.text.contains("Authenticated access: unverified"));
    assert!(
        report.text.contains(
            "API product endpoint / product response: not tested; enter a known product ID"
        )
    );
    let observed = counts.lock().unwrap();
    assert_eq!(observed.get("/robots.txt"), Some(&2));
    assert_eq!(observed.get("/"), Some(&1));
    assert_eq!(observed.len(), 2);
    server.abort();
}
#[tokio::test]
async fn robots_failure_reports_both_hosts_and_sends_only_eligible_cookies() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let server = tokio::spawn(async move {
        let mut requests = vec![];
        for challenged in [true, false] {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut raw = vec![];
            loop {
                let mut bytes = [0; 4096];
                let n = stream.read(&mut bytes).await.unwrap();
                raw.extend_from_slice(&bytes[..n]);
                if n == 0 || raw.windows(4).any(|b| b == b"\r\n\r\n") {
                    break;
                }
            }
            requests.push(String::from_utf8(raw).unwrap());
            let body = if challenged {
                "PRIVATE_FAILURE_BODY"
            } else {
                "User-agent: *\nAllow: /"
            };
            let status = if challenged { 403 } else { 200 };
            let header = if challenged {
                "cf-mitigated: challenge\r\n"
            } else {
                ""
            };
            stream.write_all(format!("HTTP/1.1 {status} Fixture\r\n{header}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        }
        requests
    });
    let clearance = crate::auth::Cookie {
        name: "cf_clearance".into(),
        first_party_site: None,
        created_micros: None,
        value: "CLEARANCE_SECRET".into(),
        domain: "www.drivethrurpg.com".into(),
        host_only: true,
        path: "/".into(),
        secure: true,
        expires: Some(9999999999),
    };
    let mut session = clearance.clone();
    session.name = "session".into();
    session.value = "SESSION_SECRET".into();
    session.host_only = false;
    session.domain = ".drivethrurpg.com".into();
    let mut path_only = session.clone();
    path_only.name = "path_only".into();
    path_only.value = "PATH_SECRET".into();
    path_only.path = "/account".into();
    let cookies = Cookies {
        imported: vec![clearance, session, path_only],
        warnings: vec![
            "Partitioned Firefox cookies omitted; use manual paste for this session".into(),
        ],
        ..Default::default()
    };
    let config = Config {
        runtime: RuntimeSettings {
            ..Default::default()
        },
        ..Default::default()
    };
    let report = tokio::time::timeout(
        Duration::from_secs(5),
        test_access_with(&config, cookies, Some("42".into()), &Fixture { endpoint }),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!report.passed);
    assert!(
        report
            .text
            .contains("Website robots: https://www.drivethrurpg.com/robots.txt")
    );
    assert!(
        report
            .text
            .contains("API robots: https://api.drivethrurpg.com/robots.txt")
    );
    assert!(
        report
            .text
            .contains("HTTP 403 — Authentication: Cloudflare challenge")
    );
    assert!(report.text.contains("Robots policy: HTTP 200 — loaded"));
    assert!(
        report
            .text
            .contains("Eligible cookies: 2; cf_clearance: eligible")
    );
    assert!(
        report
            .text
            .contains("Eligible cookies: 1; cf_clearance: loaded but expired or outside")
    );
    assert!(
        report
            .text
            .contains("Import warning: Partitioned Firefox cookies omitted")
    );
    assert!(
        report
            .text
            .contains("Website/API endpoints: not tested; required robots policies")
    );
    assert!(report.text.contains("Authenticated access: unverified"));
    assert!(!report.text.contains("SECRET") && !report.text.contains("PRIVATE_FAILURE_BODY"));
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|r| r.starts_with("GET /robots.txt ")));
    assert!(requests[0].contains("cf_clearance=CLEARANCE_SECRET"));
    assert!(!requests[1].contains("cf_clearance="));
    assert!(
        requests
            .iter()
            .all(|r| r.contains("session=SESSION_SECRET") && !r.contains("PATH_SECRET"))
    );
}

#[tokio::test]
async fn bounded_local_throughput_includes_parser_archives_and_commits() {
    const PRODUCTS: u32 = 128;
    let temp = tempfile::tempdir().unwrap();
    let paths = LocalPaths::at(temp.path());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = requests.clone();
    let server = tokio::spawn(async move {
        let mut handlers = JoinSet::new();
        loop {
            tokio::select! {
                result=listener.accept()=>{
                    let (mut stream, _) = result.unwrap();
                    let count = count.clone();
                    handlers.spawn(async move {
                        let mut raw = vec![];
                        loop {
                            let mut buf = [0; 4096];
                            let n = stream.read(&mut buf).await.unwrap();
                            raw.extend_from_slice(&buf[..n]);
                            if n == 0 || raw.windows(4).any(|w| w == b"\r\n\r\n") { break; }
                        }
                        let request = String::from_utf8(raw).unwrap();
                        let path = request.split_whitespace().nth(1).unwrap().split('?').next().unwrap();
                        let body = if path == "/robots.txt" {
                            b"User-agent: *\nAllow: /".to_vec()
                        } else if path == "/sitemap.xml" {
                            format!("<urlset>{}</urlset>", (1..=PRODUCTS).map(|id| format!("<url><loc>{}</loc></url>", d::canonical(&id.to_string()))).collect::<String>()).into_bytes()
                        } else {
                            assert!(path.starts_with("/api/vBeta/products/"));
                            let id: u32 = path.rsplit('/').next().unwrap().parse().unwrap();
                            assert!((1..=PRODUCTS).contains(&id));
                            count.fetch_add(1, Ordering::SeqCst);
                            tokio::time::sleep(Duration::from_millis(25)).await;
                            serde_json::to_vec(&json!({"id":id,"title":"Synthetic throughput fixture","description":"<p>Full synthetic metadata description.</p>".repeat(512),"price":"12.3400"})).unwrap()
                        };
                        stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).await.unwrap();
                        stream.write_all(&body).await.unwrap();
                    });
                }
                Some(result)=handlers.join_next(),if !handlers.is_empty()=>{result.unwrap();}
            }
        }
    });
    let config = Config {
        runtime: RuntimeSettings {
            workers: 16,
            parsers: 16,
            ..Default::default()
        },
        ..Default::default()
    };
    let (tx, rx) = watch::channel(Status::default());
    let (_resume, sessions) = mpsc::channel(1);
    let started = tokio::time::Instant::now();
    tokio::time::timeout(
        Duration::from_secs(60),
        run_with(
            config,
            Cookies::default(),
            false,
            tx,
            CancellationToken::new(),
            CancellationToken::new(),
            sessions,
            LocalFixture {
                base: Fixture { endpoint },
                paths,
            },
        ),
    )
    .await
    .unwrap()
    .unwrap();
    let elapsed = started.elapsed().as_secs_f64();
    assert_eq!(requests.load(Ordering::SeqCst), PRODUCTS as usize);
    assert_eq!(rx.borrow().state, "finished");
    assert_eq!(rx.borrow().metrics.saved_run, i64::from(PRODUCTS));
    assert_eq!(rx.borrow().metrics.failed, 0);
    assert_eq!(rx.borrow().metrics.sitemaps_done, 1);
    let saved_events: Vec<_> = rx
        .borrow()
        .events
        .iter()
        .filter(|e| e.contains("Saved "))
        .cloned()
        .collect();
    eprintln!(
        "Local pipeline: {PRODUCTS} durable captures in {elapsed:.2}s ({:.1}/s), {} save batches; 16 workers, 25ms fixture HTTP, ~20KiB bodies",
        f64::from(PRODUCTS) / elapsed,
        saved_events.len()
    );
    let pool = rx.borrow().catalogue.as_ref().unwrap().pool.clone();
    pool.close().await;
    server.abort();
}

#[tokio::test]
async fn end_to_end_sitemaps_auth_resume_next_run_and_no_refetch() {
    let temp = tempfile::tempdir().unwrap();
    let paths = LocalPaths::at(temp.path());
    let config = Config {
        runtime: RuntimeSettings {
            workers: 2,
            parsers: 1,
            attempts: 2,

            connect_secs: 1,
            read_secs: 2,
            total_secs: 3,
            ..Default::default()
        },
        ..Default::default()
    };
    let (endpoint, counts, server) = storefront().await;
    let (tx, mut rx) = watch::channel(Status::default());
    let (resume, sessions) = mpsc::channel(1);
    let c = config.clone();
    let test_paths = paths.clone();
    let e = endpoint.clone();
    let work = tokio::spawn(async move {
        run_with(
            c,
            Cookies::default(),
            false,
            tx,
            CancellationToken::new(),
            CancellationToken::new(),
            sessions,
            StorageFaultFixture {
                base: Fixture { endpoint: e },
                fail_once: Arc::new(AtomicBool::new(true)),
                paths: test_paths,
            },
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(20), async {
        while rx.borrow().state != "storage paused" {
            rx.changed().await.unwrap();
        }
    })
    .await
    .expect("storage failure should pause before any requests");
    assert!(counts.lock().unwrap().is_empty());
    resume.send(Cookies::default()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(20), async {
        while rx.borrow().state != "authentication paused" {
            rx.changed().await.unwrap();
        }
    })
    .await
    .expect("authentication should pause");
    let mut cookies = Secrets::default();
    cookies.api_cookie = "fixture=authorized".into();
    resume
        .send(Cookies::manual(&cookies).unwrap())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(30), work)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(rx.borrow().state, "finished");
    assert_eq!(rx.borrow().metrics.successful, 2);
    assert_eq!(rx.borrow().metrics.failed, 6);
    assert_eq!(rx.borrow().metrics.sitemaps_done, 2);
    let db = rx.borrow().catalogue.as_ref().unwrap().clone();
    let denied: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT outcome FROM jobs WHERE kind='product' AND identity IN ('48','49','50')",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(denied.len(), 3);
    for outcome in denied {
        assert_eq!(outcome["kind"], "restricted");
        assert_eq!(outcome["status"], 403);
        assert!(!outcome.to_string().contains("PRIVATE_DENIED_BODY"));
    }
    let intents: i64 =
        sqlx::query_scalar("SELECT count(*) FROM intents WHERE product_id IN ('48','49','50')")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(
        intents, 0,
        "403 bodies must never enter archive persistence"
    );
    for id in [48, 49, 50] {
        assert_eq!(
            counts.lock().unwrap()[&format!("/api/vBeta/products/{id}")],
            1,
            "no automatic in-run retry for 403"
        );
        assert!(
            !paths
                .archives()
                .join(format!("{}/{}/{id}", db.catalogue, d::KEY))
                .exists()
        );
    }
    db.pool.close().await;
    let (tx, rx2) = watch::channel(Status::default());
    let (_resume, sessions) = mpsc::channel(1);
    tokio::time::timeout(
        Duration::from_secs(30),
        run_with(
            config.clone(),
            Cookies::manual(&cookies).unwrap(),
            false,
            tx,
            CancellationToken::new(),
            CancellationToken::new(),
            sessions,
            LocalFixture {
                base: Fixture {
                    endpoint: endpoint.clone(),
                },
                paths: paths.clone(),
            },
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(rx2.borrow().metrics.successful, 6);
    assert_eq!(rx2.borrow().metrics.saved_run, 4);
    let pacing = rx2.borrow().pacing.unwrap();
    assert_eq!(pacing.workers, config.runtime.workers);
    assert_eq!(pacing.website, Some(Duration::ZERO));
    assert_eq!(pacing.api, Some(Duration::ZERO));
    {
        let counts = counts.lock().unwrap();
        assert_eq!(counts["/sitemap.xml"], 2);
        assert_eq!(counts["/nested.xml.gz"], 2);
        assert_eq!(counts["/api/vBeta/products/42"], 1);
        assert_eq!(counts["/api/vBeta/products/43"], 2);
        assert_eq!(counts["/api/vBeta/products/44"], 2);
        for id in [48, 49, 50] {
            assert_eq!(
                counts[&format!("/api/vBeta/products/{id}")],
                2,
                "403 products are retried on the next run"
            );
        }
    }
    let before = counts.lock().unwrap().clone();
    let inspect = Catalogue::connect(&paths).await.unwrap();
    let product_pool = inspect.pool.clone();
    inspect.release().await.unwrap();
    sqlx::query("UPDATE products SET parser_version='synthetic-prior-parser'")
        .execute(&product_pool)
        .await
        .unwrap();
    let (tx, reprocess_status) = watch::channel(Status::default());
    let (_resume, sessions) = mpsc::channel(1);
    tokio::time::timeout(
        Duration::from_secs(20),
        run_with(
            config,
            Cookies::default(),
            true,
            tx,
            CancellationToken::new(),
            CancellationToken::new(),
            sessions,
            LocalFixture {
                base: Fixture { endpoint },
                paths: paths.clone(),
            },
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(reprocess_status.borrow().reprocessed, 6);
    assert!(reprocess_status.borrow().pacing.is_none());
    assert_eq!(
        *counts.lock().unwrap(),
        before,
        "reprocessing must not contact the storefront"
    );
    product_pool.close().await;
    server.abort();
}
