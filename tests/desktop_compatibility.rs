//! The desktop controller opens the existing TUI file contract without conversion.
use kaijucrawl::{
    auth::BrowserProfile,
    config::{Config, RuntimeSettings, Secrets},
    controller::{AuthDraft, AuthMethod, Controller},
    domain::*,
    paths::LocalPaths,
    storage::{archive::Archives, sqlite::Catalogue},
    targets::{drivethru as d, sitemap::Entry},
};
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
async fn desktop_opens_existing_captures_queue_intents_settings_and_secrets_in_place() {
    let temp = tempfile::tempdir().unwrap();
    let paths = LocalPaths::at(temp.path());
    let config = Config {
        selected: false,
        browser: Some(BrowserProfile {
            browser: "Firefox".into(),
            name: "Synthetic old profile".into(),
            path: temp.path().join("synthetic-profile"),
            container: 2,
        }),
        ..Default::default()
    };
    config.save_at(&paths).unwrap();
    let secrets = Secrets {
        website_cookie: "synthetic=PRIVATE_FIXTURE".into(),
        api_cookie: "synthetic=API_FIXTURE".into(),
    };
    secrets.persist_at(&paths).unwrap();
    let settings_before = std::fs::read(paths.settings()).unwrap();
    let secrets_before = std::fs::read(paths.secrets()).unwrap();
    let db = Catalogue::connect(&paths).await.unwrap();
    let identity = db.catalogue;
    let run = db
        .new_run(&RuntimeSettings::default(), "crawl")
        .await
        .unwrap();
    db.discover(
        run,
        d::ROOT,
        ["42", "43", "44"]
            .into_iter()
            .map(|id| Entry {
                location: d::canonical(id),
                lastmod: None,
                sitemap: false,
            })
            .collect(),
    )
    .await
    .unwrap();
    let archives = Archives::new(&paths);
    let mut saved = vec![];
    for id in ["42", "44"] {
        let bytes = serde_json::to_vec(&json!({"id":id,"name":"Synthetic old capture","description":"A complete synthetic description","price":"12.34567890123456789"})).unwrap();
        let product = kaijucrawl::parser::extract::parse(&bytes, id, &d::detail(id)).unwrap();
        let (archive, compressed) = archives.prepare(identity, d::KEY, id, &bytes).unwrap();
        let capture = Intent {
            id: Uuid::new_v4(),
            target: d::KEY.into(),
            product_id: id.into(),
            fetched_at: chrono::Utc::now(),
            api_url: d::detail(id),
            http: json!({"status":200}),
            archive,
        };
        db.intent(&capture).await.unwrap();
        archives
            .write_verified(&capture.archive, compressed)
            .await
            .unwrap();
        saved.push((capture.clone(), bytes));
        if id == "42" {
            db.commit_captures(vec![(capture, product)]).await.unwrap();
        }
    }
    let claimed = db.claim(run, 10, 5).await.unwrap();
    let failed = claimed.iter().find(|c| c.identity == "43").unwrap();
    db.begin_attempt(failed).await.unwrap();
    db.outcome(
        failed,
        Some(&Failure::new(
            FailureKind::Restricted,
            "Synthetic product restriction",
        )),
        5,
    )
    .await
    .unwrap();
    let record_before: String =
        sqlx::query_scalar("SELECT record FROM products WHERE product_id='42'")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    // Recreate the genuine pre-search (TUI/desktop v3) schema before startup.
    sqlx::raw_sql(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/schema_before_classification.sql"
    )))
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::raw_sql("DROP TRIGGER IF EXISTS search_product_insert; DROP TRIGGER IF EXISTS search_product_update; DROP TRIGGER IF EXISTS search_product_delete; DROP TRIGGER IF EXISTS search_document_insert; DROP TRIGGER IF EXISTS search_document_delete; DROP VIEW IF EXISTS search_source; DROP TABLE IF EXISTS search_facets; DROP TABLE IF EXISTS search_fts; DROP TABLE IF EXISTS search_documents; DROP TABLE IF EXISTS search_progress; UPDATE catalogue SET version=3;").execute(&db.pool).await.unwrap();
    db.release().await.unwrap();
    db.pool.close().await;

    let desktop = Controller::new(paths.clone(), false);
    desktop.initialize().await.unwrap();
    let snapshot = desktop.snapshot().await;
    assert!(snapshot.storage_ok && !snapshot.active && !snapshot.selected);
    assert_eq!(snapshot.metrics.successful, 1);
    assert_eq!(snapshot.auth_method, "browser");
    assert_eq!(snapshot.browser_container, 2);
    assert_eq!(
        desktop.analysis_overview().await.unwrap().catalogue_total,
        1
    );
    assert!(snapshot.catalogue_loaded);
    let wire = serde_json::to_string(&snapshot).unwrap();
    assert!(!wire.contains("PRIVATE_FIXTURE") && !wire.contains("API_FIXTURE"));
    assert_eq!(std::fs::read(paths.settings()).unwrap(), settings_before);
    assert_eq!(std::fs::read(paths.secrets()).unwrap(), secrets_before);
    assert!(desktop.start(false).await.unwrap_err().contains("Select"));

    let reopened = Catalogue::connect(&paths).await.unwrap();
    assert_eq!(reopened.catalogue, identity);
    let record_after: String =
        sqlx::query_scalar("SELECT record FROM products WHERE product_id='42'")
            .fetch_one(&reopened.pool)
            .await
            .unwrap();
    assert_eq!(record_before, record_after);
    assert_eq!(
        reopened.pending_intents(Uuid::nil()).await.unwrap().len(),
        1
    );
    for (intent, bytes) in &saved {
        assert_eq!(
            archives.read(&intent.archive).await.unwrap().as_ref(),
            Some(bytes)
        );
    }
    // The same recovery protocol finalizes an interrupted TUI save, without HTTP.
    let (intent, bytes) = saved.pop().unwrap();
    let product = kaijucrawl::parser::extract::parse(&bytes, "44", &d::detail("44")).unwrap();
    reopened
        .commit_captures(vec![(intent, product)])
        .await
        .unwrap();
    let next = reopened
        .new_run(&RuntimeSettings::default(), "crawl")
        .await
        .unwrap();
    let jobs = reopened.claim(next, 10, 5).await.unwrap();
    assert!(jobs.iter().any(|j| j.identity == "43"));
    assert!(jobs.iter().any(|j| j.kind == "sitemap"));
    assert!(
        !jobs
            .iter()
            .any(|j| ["42", "44"].contains(&j.identity.as_str()))
    );
    reopened.release().await.unwrap();
    reopened.pool.close().await;
    desktop.shutdown().await;
}

fn manual(remember: bool) -> AuthDraft {
    AuthDraft {
        method: AuthMethod::Manual,
        profile_id: None,
        container: 0,
        website_cookie: "synthetic=PRIVATE_FIXTURE".into(),
        api_cookie: "synthetic=API_FIXTURE".into(),
        keep_saved: false,
        remember,
        product_id: None,
    }
}
#[tokio::test]
async fn desktop_secret_retention_clear_and_failed_validation_preserve_expected_state() {
    let temp = tempfile::tempdir().unwrap();
    let paths = LocalPaths::at(temp.path());
    let desktop = Controller::new(paths.clone(), false);
    assert!(desktop.save_auth(manual(true)).await.is_err());
    assert!(!paths.settings().exists());
    desktop.initialize().await.unwrap();
    desktop.save_auth(manual(true)).await.unwrap();
    assert!(
        Secrets::load_at(&paths)
            .unwrap()
            .website_cookie
            .contains("PRIVATE_FIXTURE")
    );
    let mut bad = manual(true);
    bad.website_cookie = "synthetic=x\r\nAuthorization: secret".into();
    assert!(desktop.save_auth(bad).await.is_err());
    assert!(
        Secrets::load_at(&paths)
            .unwrap()
            .website_cookie
            .contains("PRIVATE_FIXTURE")
    );
    desktop.save_auth(manual(false)).await.unwrap();
    assert!(Secrets::load_at(&paths).unwrap().website_cookie.is_empty());
    let s = desktop.snapshot().await;
    assert!(s.has_manual_cookies && !s.remembered);
    assert!(
        !serde_json::to_string(&s)
            .unwrap()
            .contains("PRIVATE_FIXTURE")
    );
    desktop.clear_auth().await.unwrap();
    assert_eq!(desktop.snapshot().await.auth_method, "anonymous");
    desktop.shutdown().await;
    assert!(desktop.start(false).await.is_err());
    assert!(desktop.select_target(false).await.is_err());
}

#[tokio::test]
async fn desktop_storage_retry_preserves_configuration_and_never_autostarts() {
    let temp = tempfile::tempdir().unwrap();
    let paths = LocalPaths::at(temp.path());
    Config {
        selected: false,
        ..Default::default()
    }
    .save_at(&paths)
    .unwrap();
    std::fs::write(paths.archives(), b"synthetic obstruction").unwrap();
    let desktop = Controller::new(paths.clone(), false);
    assert!(desktop.initialize().await.is_err());
    assert!(!desktop.snapshot().await.storage_ok);
    std::fs::remove_file(paths.archives()).unwrap();
    desktop.initialize().await.unwrap();
    let s = desktop.snapshot().await;
    assert!(s.storage_ok && !s.active && !s.selected);
    desktop.shutdown().await;
}
