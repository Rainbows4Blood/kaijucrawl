//! Large synthetic catalogue only; no storefront or user data access.
use kaijucrawl::targets::{drivethru as d, sitemap::Entry};
use kaijucrawl::{config::RuntimeSettings, paths::LocalPaths, storage::sqlite::Catalogue};
use std::time::Instant;

#[tokio::test]
async fn large_queue_claims_and_dashboard() {
    let temp = tempfile::tempdir().unwrap();
    let paths = LocalPaths::at(temp.path());
    let db = Catalogue::connect(&paths).await.unwrap();
    let settings = RuntimeSettings::default();
    let old = db.new_run(&settings, "crawl").await.unwrap();
    let mut tx = db.pool.begin().await.unwrap();
    sqlx::query("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<350000) INSERT INTO jobs(kind,target,identity,url,run,discovered_run,state) SELECT 'product','drivethru',CAST(x AS TEXT),'https://fixture.invalid/product/'||x,$1,$1,CASE WHEN x<=6000 THEN 'successful' ELSE 'pending' END FROM n")
        .bind(old).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO intents(id,target,product_id,payload,generation,finalized) SELECT randomblob(16),target,identity,'{}',1,true FROM jobs WHERE kind='product' AND state='successful'")
        .execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO products(target,product_id,title,description_text,record,parser_version,record_schema_version,archive,fetched_at,last_checked_at,intent) SELECT target,product_id,'Synthetic','Full',json_object('synthetic',printf('%08000d',1)),'drivethru-rust-1',2,'{}','2026-01-01T00:00:00Z','2026-01-01T00:00:00Z',id FROM intents")
        .execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let run = db.new_run(&settings, "crawl").await.unwrap();
    sqlx::query("INSERT INTO intents(id,target,product_id,payload,generation) SELECT randomblob(16),target,identity,'{}',1 FROM jobs WHERE kind='product' AND CAST(identity AS INTEGER) BETWEEN 6001 AND 6014")
        .execute(&db.pool).await.unwrap();
    let mut claimed = 0;
    for round in 0..3 {
        let start = Instant::now();
        let claims = db.claim(run, 16, 5).await.unwrap();
        let claim_ms = start.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(claims.len(), 16);
        assert_eq!(
            claims.iter().filter(|c| c.kind == "sitemap").count(),
            usize::from(round == 0),
            "discovery must get one slot ahead of the old backlog, without occupying every worker"
        );
        assert!(
            claims
                .iter()
                .all(|c| c.kind == "sitemap" || c.identity.parse::<u32>().unwrap() > 6014)
        );
        claimed += claims.iter().filter(|c| c.kind == "product").count();
        let start = Instant::now();
        let metrics = db.metrics(run).await.unwrap();
        let metrics_ms = start.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(metrics.successful, 6000);
        assert_eq!(metrics.saved_run, 0);
        assert_eq!(metrics.active, claimed as i64);
        assert_eq!(metrics.pending, 344000 - claimed as i64);
        eprintln!(
            "350k jobs / 6k captures, round {round}: claim={claim_ms:.2}ms metrics={metrics_ms:.2}ms sitemap_claimed={}",
            claims.iter().any(|c| c.kind == "sitemap")
        );
    }
    db.release().await.unwrap();
    db.pool.close().await;
}

#[tokio::test]
async fn queue_keeps_due_times_attempt_budgets_and_discovery_progress() {
    let temp = tempfile::tempdir().unwrap();
    let db = Catalogue::connect(&LocalPaths::at(temp.path()))
        .await
        .unwrap();
    let run = db
        .new_run(&RuntimeSettings::default(), "crawl")
        .await
        .unwrap();
    let mut entries: Vec<_> = (1..=6)
        .map(|id| Entry {
            location: d::canonical(&id.to_string()),
            lastmod: None,
            sitemap: false,
        })
        .collect();
    entries.extend(["a.xml", "b.xml"].map(|path| Entry {
        location: format!("{}{path}", d::WEBSITE),
        lastmod: None,
        sitemap: true,
    }));
    db.discover(run, d::ROOT, entries).await.unwrap();
    sqlx::query("UPDATE jobs SET due_at='2099-01-01T00:00:00.000Z' WHERE identity='1'")
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE jobs SET attempts=5 WHERE identity='2'")
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO intents(id,target,product_id,payload,generation) VALUES(randomblob(16),'drivethru','3','{}',1)").execute(&db.pool).await.unwrap();
    let first = db.claim(run, 2, 5).await.unwrap();
    assert_eq!(first.len(), 2);
    let sitemap = first.iter().find(|c| c.kind == "sitemap").unwrap();
    assert_eq!(
        first.iter().find(|c| c.kind == "product").unwrap().identity,
        "4"
    );
    let second = db.claim(run, 16, 5).await.unwrap();
    assert_eq!(second.len(), 2);
    assert!(
        second
            .iter()
            .all(|c| c.kind == "product" && matches!(c.identity.as_str(), "5" | "6"))
    );
    db.begin_attempt(sitemap).await.unwrap();
    db.outcome(sitemap, None, 5).await.unwrap();
    let third = db.claim(run, 1, 5).await.unwrap();
    assert_eq!(third.len(), 1);
    assert_eq!(third[0].kind, "sitemap");
    let metrics = db.metrics(run).await.unwrap();
    assert_eq!((metrics.pending, metrics.active, metrics.failed), (2, 3, 1));
    assert_eq!(
        (
            metrics.sitemaps_pending,
            metrics.sitemaps_active,
            metrics.sitemaps_done
        ),
        (2, 1, 1)
    );
    assert_eq!((metrics.seen, metrics.discovered), (6, 6));
    db.cooldown(d::KEY, 120).await.unwrap();
    assert!(db.claim(run, 16, 5).await.unwrap().is_empty());
    assert!(
        db.metrics(run).await.unwrap().next_due.unwrap()
            > chrono::Utc::now() + chrono::Duration::seconds(100)
    );
    db.release().await.unwrap();
    db.pool.close().await;
}
