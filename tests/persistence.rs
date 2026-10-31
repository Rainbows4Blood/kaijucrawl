//! Disposable local directories only. Never uses application data/config.
use kaijucrawl::{
    config::{Config, RuntimeSettings},
    domain::*,
    parser::extract,
    scheduler,
    storage::{archive::Archives, sqlite::Catalogue},
    targets::{drivethru as d, sitemap::Entry},
};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

#[tokio::test]
async fn local_initialization_retries_after_filesystem_failure_without_configuration() {
    let temp = tempfile::tempdir().unwrap();
    let paths = kaijucrawl::paths::LocalPaths::at(temp.path());
    std::fs::write(paths.archives(), b"synthetic path conflict").unwrap();
    assert!(
        scheduler::validate_at(&Config::default(), &paths)
            .await
            .is_err()
    );
    std::fs::remove_file(paths.archives()).unwrap();
    let db = scheduler::validate_at(&Config::default(), &paths)
        .await
        .unwrap();
    assert_eq!(db.metrics(Uuid::nil()).await.unwrap().successful, 0);
    assert!(paths.database().is_file() && paths.archives().is_dir());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for file in [
            paths.database(),
            paths.lock(),
            paths.root().join("catalogue.sqlite3-wal"),
            paths.root().join("catalogue.sqlite3-shm"),
        ] {
            assert_eq!(
                std::fs::metadata(file).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
    db.pool.close().await;
}

#[tokio::test]
async fn catalogue_crash_child() {
    let Ok(directory) = std::env::var("KAIJUCRAWL_TEST_CRASH_DIRECTORY") else {
        return;
    };
    let phase = std::env::var("KAIJUCRAWL_TEST_CRASH_PHASE").unwrap();
    let paths = kaijucrawl::paths::LocalPaths::at(directory);
    let db = Catalogue::connect(&paths).await.unwrap();
    let run = db
        .new_run(&RuntimeSettings::default(), "crawl")
        .await
        .unwrap();
    db.discover(run, d::ROOT, vec![entry("42")]).await.unwrap();
    let archives = Archives::new(&paths);
    let (capture, product, bytes) = intent(&db, &archives, "42").await;
    db.intent(&capture).await.unwrap();
    if phase == "staging" {
        let staging = paths.archives().join(".staging");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(
            staging.join("archive-interrupted"),
            &bytes[..bytes.len() / 2],
        )
        .unwrap();
    }
    if matches!(phase.as_str(), "archive" | "commit") {
        archives
            .write_verified(&capture.archive, bytes)
            .await
            .unwrap();
    }
    if phase == "commit" {
        db.commit_captures(vec![(capture, product)]).await.unwrap();
    }
    std::fs::write(paths.root().join("ready"), b"ready").unwrap();
    std::future::pending::<()>().await;
}

#[tokio::test]
async fn abrupt_process_exit_releases_lock_and_recovers_each_persistence_boundary() {
    for phase in ["intent", "staging", "archive", "commit"] {
        let temp = tempfile::tempdir().unwrap();
        let paths = kaijucrawl::paths::LocalPaths::at(temp.path());
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "catalogue_crash_child", "--nocapture"])
            .env("KAIJUCRAWL_TEST_CRASH_DIRECTORY", temp.path())
            .env("KAIJUCRAWL_TEST_CRASH_PHASE", phase)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !paths.root().join("ready").exists() {
            if std::time::Instant::now() > deadline || child.try_wait().unwrap().is_some() {
                let _ = child.kill();
                let _ = child.wait();
                panic!("crash fixture did not reach {phase}");
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(matches!(
            Catalogue::connect(&paths).await,
            Err(Error::Ownership)
        ));
        child.kill().unwrap();
        child.wait().unwrap();
        let db = Catalogue::connect(&paths).await.unwrap();
        assert!(
            !paths
                .archives()
                .join(".staging/archive-interrupted")
                .exists()
        );
        let archives = Archives::new(&paths);
        let pending = db.pending_intents(Uuid::nil()).await.unwrap();
        assert_eq!(pending.len(), usize::from(phase != "commit"));
        for capture in pending {
            if let Some(body) = archives.read(&capture.archive).await.unwrap() {
                let parsed = extract::parse(&body, &capture.product_id, &capture.api_url).unwrap();
                db.commit_captures(vec![(capture, parsed)]).await.unwrap();
            } else {
                db.absent_intent(&capture).await.unwrap();
            }
        }
        let run = db
            .new_run(&RuntimeSettings::default(), "crawl")
            .await
            .unwrap();
        let claims = db.claim(run, 10, 5).await.unwrap();
        assert_eq!(
            claims.iter().any(|c| c.identity == "42"),
            matches!(phase, "intent" | "staging")
        );
        db.release().await.unwrap();
        db.pool.close().await;
    }
}

fn entry(id: &str) -> Entry {
    Entry {
        location: d::canonical(id),
        lastmod: Some("2020".into()),
        sitemap: false,
    }
}
fn body(id: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({"id":id,"name":"Synthetic","description":"Full synthetic description","price":"123456789.1234567890123456789"})).unwrap()
}
async fn intent(db: &Catalogue, a: &Archives, id: &str) -> (Intent, Product, Vec<u8>) {
    let bytes = body(id);
    let p = extract::parse(&bytes, id, &d::detail(id)).unwrap();
    let (archive, gz) = a.prepare(db.catalogue, d::KEY, id, &bytes).unwrap();
    (
        Intent {
            id: Uuid::new_v4(),
            target: d::KEY.into(),
            product_id: id.into(),
            fetched_at: chrono::Utc::now(),
            api_url: d::detail(id),
            http: json!({"status":200}),
            archive,
        },
        p,
        gz,
    )
}

#[tokio::test]
async fn ownership_recovery_queue_and_reprocessing() {
    let temp = tempfile::tempdir().unwrap();
    let paths = kaijucrawl::paths::LocalPaths::at(temp.path());
    let archive = Archives::new(&paths);
    archive.test().await.unwrap();
    let db = Catalogue::connect(&paths).await.unwrap();
    assert!(matches!(
        Catalogue::connect(&paths).await,
        Err(Error::Ownership)
    ));
    let settings = RuntimeSettings::default();
    let run = db.new_run(&settings, "crawl").await.unwrap();
    let mut aliases = vec![entry("42"), entry("43"), entry("44")];
    aliases.push(Entry {
        location: "https://drivethrurpg.com/de/product/00042/slug?x=1".into(),
        lastmod: None,
        sitemap: false,
    });
    db.discover(run, d::ROOT, aliases).await.unwrap();
    db.discover(run, d::ROOT, vec![entry("42")]).await.unwrap();
    assert_eq!(db.metrics(run).await.unwrap().pending, 3);
    let claims = db.claim(run, 20, 5).await.unwrap();
    assert_eq!(claims.len(), 4);
    let product = claims.iter().find(|c| c.identity == "42").unwrap();
    assert_eq!(product.attempts, 0);
    // Unstarted reservations do not count; started claims do, even after a crash.
    db.begin_attempt(product).await.unwrap();
    db.release_claims(run).await.unwrap();
    let c = db.claim(run, 20, 5).await.unwrap();
    assert_eq!(c.iter().find(|c| c.identity == "42").unwrap().attempts, 1);
    let (i, p, gz) = intent(&db, &archive, "42").await;
    db.intent(&i).await.unwrap();
    assert_eq!(db.metrics(run).await.unwrap().successful, 0);
    // Crash after intent, before archive publication: recovery restores fetch eligibility.
    assert!(archive.read(&i.archive).await.unwrap().is_none());
    db.absent_intent(&i).await.unwrap();
    // Crash after archive publication, before commit: exact bytes are recoverable from the local archive.
    db.intent(&i).await.unwrap();
    archive
        .write_verified(&i.archive, gz.clone())
        .await
        .unwrap();
    assert_eq!(db.pending_intents(Uuid::nil()).await.unwrap().len(), 1);
    let old_generation = db.generation;
    db.release().await.unwrap();
    let successor = Catalogue::connect(&paths).await.unwrap();
    assert!(successor.generation > old_generation);
    assert!(matches!(
        db.commit_captures(vec![(i.clone(), p.clone())]).await,
        Err(Error::Ownership)
    ));
    assert!(matches!(
        db.discover(run, d::ROOT, vec![entry("99")]).await,
        Err(Error::Ownership)
    ));
    assert!(matches!(db.verify().await, Err(Error::Ownership)));
    let recovered = archive.read(&i.archive).await.unwrap().unwrap();
    let product = extract::parse(&recovered, "42", &d::detail("42")).unwrap();
    assert_eq!(
        successor
            .commit_captures(vec![(i.clone(), product)])
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        successor
            .commit_captures(vec![(i.clone(), p.clone())])
            .await
            .unwrap(),
        0
    );
    assert_eq!(successor.metrics(run).await.unwrap().successful, 1);
    // Product numeric values preserve decimal precision as exact text and JSON.
    let price: String = sqlx::query_scalar("SELECT price FROM products WHERE product_id='42'")
        .fetch_one(&successor.pool)
        .await
        .unwrap();
    assert_eq!(price, "123456789.1234567890123456789");
    // Atomic batch: one invalid FK rolls back every product/task transition.
    let (i43, p43, g43) = intent(&successor, &archive, "43").await;
    successor.intent(&i43).await.unwrap();
    archive.write_verified(&i43.archive, g43).await.unwrap();
    let (i44, p44, g44) = intent(&successor, &archive, "44").await;
    // Verification alone cannot bypass the durable-intent prerequisite.
    archive.write_verified(&i44.archive, g44).await.unwrap();
    assert!(
        successor
            .commit_captures(vec![(i43.clone(), p43.clone()), (i44, p44)])
            .await
            .is_err()
    );
    assert_eq!(successor.metrics(run).await.unwrap().successful, 1);
    successor.commit_captures(vec![(i43, p43)]).await.unwrap();
    // A stale pending intent for a successful product cannot requeue that product.
    let (old, _, _) = intent(&successor, &archive, "42").await;
    successor.intent(&old).await.unwrap();
    successor.absent_intent(&old).await.unwrap();
    assert_eq!(successor.metrics(run).await.unwrap().successful, 2);
    let next = successor.new_run(&settings, "crawl").await.unwrap();
    let mut changed = entry("42");
    changed.lastmod = Some("2099".into());
    successor
        .discover(next, d::ROOT, vec![changed, entry("44")])
        .await
        .unwrap();
    let claims = successor.claim(next, 20, 1).await.unwrap();
    assert!(
        claims
            .iter()
            .all(|c| c.identity != "42" && c.identity != "43")
    );
    let failed = claims.iter().find(|c| c.identity == "44").unwrap();
    successor.begin_attempt(failed).await.unwrap();
    let f = Failure::new(FailureKind::Incomplete, "Missing description");
    successor.outcome(failed, Some(&f), 1).await.unwrap();
    successor
        .discover(next, "second sitemap", vec![entry("44")])
        .await
        .unwrap();
    assert!(
        successor
            .claim(next, 20, 1)
            .await
            .unwrap()
            .iter()
            .all(|c| c.identity != "44")
    );
    let next2 = successor.new_run(&settings, "crawl").await.unwrap();
    assert!(
        successor
            .claim(next2, 20, 1)
            .await
            .unwrap()
            .iter()
            .any(|c| c.identity == "44" && c.attempts == 0)
    );
    // Target cooldown remains durable through release/reacquisition and a new run.
    successor.cooldown(d::KEY, 120).await.unwrap();
    successor.release().await.unwrap();
    let owner = Catalogue::connect(&paths).await.unwrap();
    let next3 = owner.new_run(&settings, "crawl").await.unwrap();
    assert!(owner.claim(next3, 20, 5).await.unwrap().is_empty());
    assert!(owner.remaining_cooldown(d::KEY).await.unwrap() >= 100);
    // Reprocessing has no network parameter; failures preserve the old good row.
    let maintenance = owner.new_run(&settings, "reprocess").await.unwrap();
    let old: serde_json::Value =
        sqlx::query_scalar("SELECT record FROM products WHERE product_id='42'")
            .fetch_one(&owner.pool)
            .await
            .unwrap();
    owner
        .reprocess_commit(
            maintenance,
            d::KEY,
            "42",
            &old,
            Err(Failure::new(
                FailureKind::Incomplete,
                "Synthetic parser failure",
            )),
        )
        .await
        .unwrap();
    let unchanged: serde_json::Value =
        sqlx::query_scalar("SELECT record FROM products WHERE product_id='42'")
            .fetch_one(&owner.pool)
            .await
            .unwrap();
    assert_eq!(old, unchanged);
    let mut update = p;
    update.title = "Updated parser title".into();
    owner
        .reprocess_commit(maintenance, d::KEY, "42", &old, Ok(update))
        .await
        .unwrap();
    let row =
        sqlx::query("SELECT record,fetched_at,last_checked_at FROM products WHERE product_id='42'")
            .fetch_one(&owner.pool)
            .await
            .unwrap();
    let updated: serde_json::Value = row.get("record");
    assert_eq!(updated["fetched_at"], old["fetched_at"]);
    assert_eq!(updated["last_checked_at"], old["last_checked_at"]);
    assert_eq!(updated["archive"], old["archive"]);
    assert_eq!(updated["title"], "Updated parser title");
    owner
        .run_state(maintenance, "finished", true)
        .await
        .unwrap();
    let counters = sqlx::query("SELECT processed,failures,summary FROM runs WHERE id=$1")
        .bind(maintenance)
        .fetch_one(&owner.pool)
        .await
        .unwrap();
    assert_eq!(counters.get::<i64, _>("processed"), 1);
    assert_eq!(counters.get::<i64, _>("failures"), 0);
    assert_eq!(
        counters.get::<serde_json::Value, _>("summary")["successful"],
        2
    );
    owner.release().await.unwrap();
    sqlx::query("UPDATE catalogue SET version=999")
        .execute(&owner.pool)
        .await
        .unwrap();
    assert!(matches!(
        Catalogue::connect(&paths).await,
        Err(Error::Config(_))
    ));
    db.pool.close().await;
    successor.pool.close().await;
    owner.pool.close().await;
}

#[tokio::test]
async fn version_two_upgrade_preserves_captures_queue_and_recovery() {
    let temp = tempfile::tempdir().unwrap();
    let paths = kaijucrawl::paths::LocalPaths::at(temp.path());
    let archive = Archives::new(&paths);
    let old = Catalogue::connect(&paths).await.unwrap();
    let run = old
        .new_run(&RuntimeSettings::default(), "crawl")
        .await
        .unwrap();
    old.discover(run, d::ROOT, vec![entry("42"), entry("43")])
        .await
        .unwrap();
    let (saved, product, bytes) = intent(&old, &archive, "42").await;
    old.intent(&saved).await.unwrap();
    archive.write_verified(&saved.archive, bytes).await.unwrap();
    old.commit_captures(vec![(saved.clone(), product)])
        .await
        .unwrap();
    let (pending, _, _) = intent(&old, &archive, "43").await;
    old.intent(&pending).await.unwrap();
    let before: serde_json::Value = sqlx::query_scalar("SELECT record FROM products")
        .fetch_one(&old.pool)
        .await
        .unwrap();
    let metrics = serde_json::to_value(old.metrics(run).await.unwrap()).unwrap();
    let identity = old.catalogue;
    let generation = old.generation;
    // Version 2 has identical data tables, without the version 3 access indexes.
    sqlx::raw_sql(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/schema_before_classification.sql"
    )))
    .execute(&old.pool)
    .await
    .unwrap();
    sqlx::raw_sql("DROP INDEX jobs_ready; DROP INDEX jobs_attempt_budget; DROP INDEX pending_product_intents; DROP INDEX jobs_run_counts; DROP INDEX jobs_discovered; DROP INDEX products_saved; UPDATE catalogue SET version=2;")
        .execute(&old.pool).await.unwrap();
    old.release().await.unwrap();
    old.pool.close().await;
    let upgraded = Catalogue::connect(&paths).await.unwrap();
    assert_eq!(upgraded.catalogue, identity);
    assert!(upgraded.generation > generation);
    assert_eq!(
        sqlx::query_scalar::<_, i32>("SELECT version FROM catalogue")
            .fetch_one(&upgraded.pool)
            .await
            .unwrap(),
        CATALOGUE_VERSION
    );
    assert_eq!(
        sqlx::query_scalar::<_, serde_json::Value>("SELECT record FROM products")
            .fetch_one(&upgraded.pool)
            .await
            .unwrap(),
        before
    );
    assert_eq!(
        serde_json::to_value(upgraded.metrics(run).await.unwrap()).unwrap(),
        metrics
    );
    assert_eq!(
        archive.read(&saved.archive).await.unwrap().unwrap(),
        body("42")
    );
    assert_eq!(
        upgraded.pending_intents(Uuid::nil()).await.unwrap()[0].id,
        pending.id
    );
    assert!(
        upgraded
            .claim(run, 16, 5)
            .await
            .unwrap()
            .iter()
            .all(|c| c.kind == "sitemap")
    );
    // Heartbeat/read guards no longer queue behind an otherwise idle writer.
    let held_write = upgraded.pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), upgraded.verify())
        .await
        .unwrap()
        .unwrap();
    held_write.rollback().await.unwrap();
    upgraded.release().await.unwrap();
    upgraded.pool.close().await;
}
