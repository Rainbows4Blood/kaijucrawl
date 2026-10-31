//! Synthetic local analysis, independent of storefronts and user storage.
use kaijucrawl::{
    analysis,
    paths::LocalPaths,
    storage::{analysis as stored, sqlite::Catalogue},
};
use serde_json::json;

async fn seed(db: &Catalogue, n: i64) {
    let intent = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO intents(id,target,product_id,payload,generation,finalized) VALUES(?,'drivethru','1','{}',?,1)").bind(intent).bind(db.generation).execute(&db.pool).await.unwrap();
    sqlx::query("WITH RECURSIVE n(i) AS(VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<?) INSERT INTO products(target,product_id,title,description_text,record,parser_version,record_schema_version,archive,fetched_at,last_checked_at,intent) SELECT 'drivethru',CAST(i AS TEXT),'Fixture '||i,'Complete synthetic description',json_object('rating',CASE WHEN i%2=0 THEN '4.7' ELSE '5.0' END,'rating_count',CASE WHEN i%2=0 THEN 1000 ELSE 3 END,'raw_metadata',json_object('untouched',1)),'drivethru-rust-1',2,'{}','2026-01-01','2026-01-01',? FROM n")
        .bind(n).bind(intent).execute(&db.pool).await.unwrap();
}
async fn complete(db: &Catalogue) -> analysis::Status {
    let mut s = db.begin_analysis().await.unwrap();
    while !db.analysis_batch(&mut s).await.unwrap() {}
    s
}
async fn canonical(db: &Catalogue) -> Vec<(String, String, String)> {
    sqlx::query_as("SELECT product_id,record,fetched_at FROM products ORDER BY rowid")
        .fetch_all(&db.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn migration_preserves_capture_identity_without_calculating() {
    let temp = tempfile::tempdir().unwrap();
    let paths = LocalPaths::at(temp.path());
    let db = Catalogue::connect(&paths).await.unwrap();
    seed(&db, 4).await;
    let identity = db.catalogue;
    let before = canonical(&db).await;
    sqlx::raw_sql(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/schema_before_classification.sql"
    )))
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::raw_sql("DROP TRIGGER relevance_inputs_changed; DROP TABLE relevance_scores; DROP TABLE analysis_runs; DROP TABLE analysis_revision; UPDATE catalogue SET version=4;").execute(&db.pool).await.unwrap();
    db.release().await.unwrap();
    db.pool.close().await;
    let db = Catalogue::connect(&paths).await.unwrap();
    assert_eq!(db.catalogue, identity);
    assert_eq!(canonical(&db).await, before);
    assert_eq!(
        sqlx::query_scalar::<_, i32>("SELECT version FROM catalogue")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        kaijucrawl::domain::CATALOGUE_VERSION
    );
    assert_eq!(stored::latest(&db.pool).await.unwrap().state, "idle");
    assert!(
        stored::score(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .is_none()
    );
    db.release().await.unwrap();
    db.pool.close().await;
}

#[tokio::test]
async fn catalogue_wide_scores_preserve_captures_and_store_input_provenance() {
    let temp = tempfile::tempdir().unwrap();
    let db = Catalogue::connect(&LocalPaths::at(temp.path()))
        .await
        .unwrap();
    seed(&db, 130).await;
    let before = canonical(&db).await;
    let status = complete(&db).await;
    assert_eq!(
        (
            status.total,
            status.processed,
            status.scored,
            status.skipped
        ),
        (130, 130, 130, 0)
    );
    assert!(status.query.is_none());
    assert_eq!(status.scope, "all");
    let score = stored::score(&db.pool, "drivethru", "2")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(score.value, Some(4.666667));
    assert_eq!(score.count, Some(1000));
    assert_eq!(score.rating, Some(4.7));
    assert_eq!(
        stored::score(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .unwrap()
            .value,
        Some(3.260870)
    );
    assert_eq!(canonical(&db).await, before);
    let rerun = complete(&db).await;
    assert_eq!(rerun.scored, 130);
    assert_eq!(rerun.overview.unwrap().catalogue_total, 130);
    assert_eq!(canonical(&db).await, before);
    db.release().await.unwrap();
    db.pool.close().await;
}

#[tokio::test]
async fn invalid_inputs_skip_scoring_and_changed_inputs_invalidate_without_automatic_work() {
    let temp = tempfile::tempdir().unwrap();
    let db = Catalogue::connect(&LocalPaths::at(temp.path()))
        .await
        .unwrap();
    seed(&db, 8).await;
    for (id, rating, count) in [
        ("1", json!(null), json!(3)),
        ("2", json!("bad"), json!(8)),
        ("3", json!(5), json!(0)),
        ("4", json!(7), json!(3)),
        ("5", json!(5), json!(1.5)),
        ("6", json!("NaN"), json!(10)),
    ] {
        sqlx::query("UPDATE products SET record=json_set(record,'$.rating',json(?),'$.rating_count',json(?)) WHERE product_id=?").bind(rating.to_string()).bind(count.to_string()).bind(id).execute(&db.pool).await.unwrap();
    }
    let status = complete(&db).await;
    assert_eq!((status.scored, status.skipped), (2, 6));
    for id in 1..=6 {
        assert_eq!(
            stored::score(&db.pool, "drivethru", &id.to_string())
                .await
                .unwrap()
                .unwrap()
                .value,
            None
        );
    }
    let old = stored::score(&db.pool, "drivethru", "8")
        .await
        .unwrap()
        .unwrap();
    sqlx::query("UPDATE products SET record=json_set(record,'$.unrelated','preserve score') WHERE product_id='8'").execute(&db.pool).await.unwrap();
    assert_eq!(
        stored::score(&db.pool, "drivethru", "8")
            .await
            .unwrap()
            .unwrap()
            .calculated_at,
        old.calculated_at
    );
    let mut tx = db.pool.begin().await.unwrap();
    sqlx::query("UPDATE products SET record=json_set(record,'$.rating',4.8) WHERE product_id='8'")
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    assert!(
        stored::score(&db.pool, "drivethru", "8")
            .await
            .unwrap()
            .is_some()
    );
    sqlx::query("UPDATE products SET record=json_set(record,'$.rating',4.8) WHERE product_id='8'")
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(
        stored::score(&db.pool, "drivethru", "8")
            .await
            .unwrap()
            .is_none()
    );
    sqlx::query("DELETE FROM products WHERE product_id='7'")
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(
        stored::score(&db.pool, "drivethru", "7")
            .await
            .unwrap()
            .is_none()
    );
    db.release().await.unwrap();
    db.pool.close().await;
}

#[tokio::test]
async fn batch_failure_rolls_back_scores_and_counts_and_stale_writers_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let paths = LocalPaths::at(temp.path());
    let db = Catalogue::connect(&paths).await.unwrap();
    seed(&db, 600).await;
    let mut status = db.begin_analysis().await.unwrap();
    sqlx::raw_sql("CREATE TRIGGER fail_analysis BEFORE INSERT ON relevance_scores WHEN new.product_id='250' BEGIN SELECT RAISE(ABORT,'synthetic analysis failure'); END;").execute(&db.pool).await.unwrap();
    assert!(db.analysis_batch(&mut status).await.is_err());
    assert_eq!(status.processed, 0);
    assert_eq!(stored::latest(&db.pool).await.unwrap().processed, 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM relevance_scores")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        0
    );
    sqlx::raw_sql("DROP TRIGGER fail_analysis")
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(!db.analysis_batch(&mut status).await.unwrap());
    assert_eq!(status.processed, 500);
    // Model death after a durable batch: the next owner must not auto-calculate.
    db.release().await.unwrap();
    let next = Catalogue::connect(&paths).await.unwrap();
    let recovered = stored::latest(&next.pool).await.unwrap();
    assert_eq!(recovered.state, "interrupted");
    assert_eq!(recovered.processed, 500);
    assert!(db.analysis_batch(&mut status).await.is_err());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM relevance_scores")
            .fetch_one(&next.pool)
            .await
            .unwrap(),
        500
    );
    let rerun = complete(&next).await;
    assert_eq!(rerun.processed, 600);
    next.release().await.unwrap();
    next.pool.close().await;
    db.pool.close().await;
}

#[tokio::test]
async fn cancelled_runs_keep_batches_across_restart_and_empty_catalogues_finish_cleanly() {
    let temp = tempfile::tempdir().unwrap();
    let paths = LocalPaths::at(temp.path());
    let db = Catalogue::connect(&paths).await.unwrap();
    seed(&db, 501).await;
    let mut status = db.begin_analysis().await.unwrap();
    assert!(!db.analysis_batch(&mut status).await.unwrap());
    db.finish_analysis(&mut status, "cancelled", "Synthetic stop")
        .await
        .unwrap();
    assert_eq!(status.processed, 500);
    db.release().await.unwrap();
    db.pool.close().await;
    let db = Catalogue::connect(&paths).await.unwrap();
    assert_eq!(stored::latest(&db.pool).await.unwrap().state, "cancelled");
    assert!(
        stored::score(&db.pool, "drivethru", "501")
            .await
            .unwrap()
            .is_none()
    );
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM relevance_scores")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(total, 500);
    assert_eq!(complete(&db).await.scored, 501);
    db.release().await.unwrap();
    db.pool.close().await;
    let empty_temp = tempfile::tempdir().unwrap();
    let empty_db = Catalogue::connect(&LocalPaths::at(empty_temp.path()))
        .await
        .unwrap();
    let empty = complete(&empty_db).await;
    assert_eq!(empty.state, "completed");
    assert_eq!(empty.total, 0);
    empty_db.release().await.unwrap();
    empty_db.pool.close().await;
}

#[tokio::test]
async fn discovery_relevance_reuses_unknowns_and_fills_only_missing_or_invalidated_results() {
    let temp = tempfile::tempdir().unwrap();
    let db = Catalogue::connect(&LocalPaths::at(temp.path()))
        .await
        .unwrap();
    seed(&db, 501).await;
    sqlx::query("UPDATE products SET record='{}' WHERE product_id='1'")
        .execute(&db.pool)
        .await
        .unwrap();
    let original = complete(&db).await;
    let mut reused = db.begin_relevance(false).await.unwrap();
    assert_eq!(reused.total, 0);
    assert!(db.analysis_batch(&mut reused).await.unwrap());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM relevance_scores WHERE run_id=?")
            .bind(original.id)
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        501
    );
    sqlx::query("UPDATE products SET record=json_set(record,'$.rating',4.8) WHERE product_id IN ('2','501')").execute(&db.pool).await.unwrap();
    let mut missing = db.begin_relevance(false).await.unwrap();
    assert_eq!(missing.total, 2);
    assert!(db.analysis_batch(&mut missing).await.unwrap());
    assert_eq!(missing.processed, 2);
    assert_eq!(
        missing.overview.as_ref().unwrap().analyses[analysis::ALGORITHM].completed,
        501
    );
    let mut rerun = db.begin_relevance(true).await.unwrap();
    assert_eq!(rerun.total, 501);
    while !db.analysis_batch(&mut rerun).await.unwrap() {}
    assert_eq!(rerun.skipped, 1);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM relevance_scores WHERE run_id=?")
            .bind(rerun.id)
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        501
    );
    db.release().await.unwrap();
}
