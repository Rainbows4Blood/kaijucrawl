//! Synthetic group persistence; no API calls or production catalogue access.
use kaijucrawl::{
    analysis::Status,
    classification::{self, Category},
    decisions::{self, Decision},
    paths::LocalPaths,
    storage::{classification as stored, sqlite::Catalogue},
};
use serde_json::json;

async fn seeded(n: i64) -> (tempfile::TempDir, LocalPaths, Catalogue) {
    let temp = tempfile::tempdir().unwrap();
    let paths = LocalPaths::at(temp.path());
    let db = Catalogue::connect(&paths).await.unwrap();
    let intent = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO intents(id,target,product_id,payload,generation,finalized) VALUES(?,'drivethru','1','{}',?,1)").bind(intent).bind(db.generation).execute(&db.pool).await.unwrap();
    sqlx::query("WITH RECURSIVE n(i) AS(VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<?) INSERT INTO products(target,product_id,title,description_text,record,parser_version,record_schema_version,archive,fetched_at,last_checked_at,intent) SELECT 'drivethru',CAST(i AS TEXT),'Synthetic book '||i,'A full fictional book description',json_object('rating',4.7,'rating_count',1000,'product_types',json_array('Unreliable taxonomy'),'raw_metadata',json_object('private_marker','DO_NOT_SEND')),'drivethru-rust-1',2,'{}','2026-01-01','2026-01-01',? FROM n").bind(n).bind(intent).execute(&db.pool).await.unwrap();
    (temp, paths, db)
}
fn answer(category: Category, refusal: bool) -> Decision {
    let values = ["core_rulebook", "supplement", "adventure", "none"];
    serde_json::from_value(json!({"model":decisions::MODEL,"answers":[if refusal{json!({"type":"refusal","name":"primary_book_role"})}else{json!({"type":"choice","name":"primary_book_role","choice":category.key(),"confidence":0.98,"probabilities":values.map(|v|json!({"value":v,"probability":if v==category.key(){1.0}else{0.0}}))})}],"usage":{"input_tokens":400,"input_tokens_details":{"cached_tokens":0,"cache_write_tokens":0},"output_tokens":0,"total_tokens":400}})).unwrap()
}
async fn classify(db: &Catalogue, status: &mut Status, category: Category) {
    let book = db.classification_next(status).await.unwrap().unwrap();
    assert!(!book.evidence.as_ref().unwrap().contains("DO_NOT_SEND"));
    classification::request(serde_json::from_str(book.evidence.as_ref().unwrap()).unwrap())
        .validate()
        .unwrap();
    db.classification_dispatch(status, &book).await.unwrap();
    db.classification_commit(status, &book, Some(&answer(category, false)), None)
        .await
        .unwrap();
}
async fn canonical(db: &Catalogue) -> Vec<(String, String, String, String)> {
    sqlx::query_as("SELECT product_id,record,fetched_at,archive FROM products ORDER BY rowid")
        .fetch_all(&db.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn out_of_order_checkpoints_keep_gaps_eligible_and_reject_duplicate_commits() {
    let (_temp, paths, db) = seeded(4).await;
    let mut status = db.begin_classification(false).await.unwrap();
    let mut after = 0;
    let mut books = Vec::new();
    for _ in 0..3 {
        let book = db
            .classification_next_after(&status, after)
            .await
            .unwrap()
            .unwrap();
        after = book.document;
        db.classification_dispatch(&mut status, &book)
            .await
            .unwrap();
        books.push(book);
    }
    for index in [2, 0] {
        db.classification_commit(
            &mut status,
            &books[index],
            Some(&answer(Category::Adventure, false)),
            None,
        )
        .await
        .unwrap();
    }
    assert_eq!(status.processed, 2);
    assert_eq!(status.classification.as_ref().unwrap().input_tokens, 800);
    let checkpoint: i64 = sqlx::query_scalar("SELECT last_id FROM analysis_runs WHERE id=?")
        .bind(status.id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(checkpoint, books[2].document);
    assert!(
        db.classification_commit(
            &mut status,
            &books[0],
            Some(&answer(Category::Adventure, false)),
            None
        )
        .await
        .is_err()
    );
    assert_eq!(status.processed, 2);
    // Old progress JSON lacks worker fields; new live activity must not survive reopening.
    sqlx::query("UPDATE analysis_runs SET classification=json_remove(json_set(classification,'$.in_flight',3),'$.workers') WHERE id=?").bind(status.id).execute(&db.pool).await.unwrap();
    db.release().await.unwrap();
    let next = Catalogue::connect(&paths).await.unwrap();
    let history = kaijucrawl::storage::analysis::latest(&next.pool)
        .await
        .unwrap();
    let old = history.classification.unwrap();
    assert_eq!(
        (history.state.as_str(), old.workers, old.in_flight),
        ("interrupted", 1, 0)
    );
    let resumed = next.begin_classification(false).await.unwrap();
    assert_eq!(resumed.total, 2);
    let gap = next
        .classification_next_after(&resumed, 0)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(gap.document, books[1].document);
    let fourth = next
        .classification_next_after(&resumed, gap.document)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fourth.document, 4);
    assert!(
        db.classification_dispatch(&mut status, &books[1])
            .await
            .is_err()
    );
    next.release().await.unwrap();
}

#[tokio::test]
async fn retry_dispatches_are_fenced_counted_and_recoverable_without_duplicate_results() {
    let (_temp, paths, db) = seeded(1).await;
    let mut s = db.begin_classification(false).await.unwrap();
    let book = db.classification_next(&s).await.unwrap().unwrap();
    db.classification_dispatch(&mut s, &book).await.unwrap();
    db.classification_retry_wait(&s, &book, "Network connection failed")
        .await
        .unwrap();
    assert!(
        db.classification_retry_wait(&s, &book, "Network connection failed")
            .await
            .is_err()
    );
    db.classification_redispatch(&mut s, &book).await.unwrap();
    assert!(db.classification_redispatch(&mut s, &book).await.is_err());
    assert_eq!(s.processed, 0);
    let p = s.classification.as_ref().unwrap();
    assert_eq!((p.requests, p.retries), (2, 1));
    db.classification_retry_wait(&s, &book, "Invalid model response")
        .await
        .unwrap();
    db.release().await.unwrap();
    let next = Catalogue::connect(&paths).await.unwrap();
    let previous = kaijucrawl::storage::analysis::latest(&next.pool)
        .await
        .unwrap();
    assert_eq!(
        (previous.state.as_str(), previous.processed),
        ("interrupted", 0)
    );
    let p = previous.classification.unwrap();
    assert_eq!(p.retry_reason, None);
    assert_eq!(
        (p.requests, p.retries, p.retrying, p.retry_in_secs),
        (2, 1, 0, 0)
    );
    let attempt: String =
        sqlx::query_scalar("SELECT state FROM classification_attempts WHERE run_id=?")
            .bind(s.id)
            .fetch_one(&next.pool)
            .await
            .unwrap();
    assert_eq!(attempt, "interrupted");
    assert!(db.classification_redispatch(&mut s, &book).await.is_err());
    let mut resumed = next.begin_classification(false).await.unwrap();
    assert_eq!(resumed.total, 1);
    classify(&next, &mut resumed, Category::Adventure).await;
    assert_eq!((resumed.processed, resumed.scored), (1, 1));
    next.release().await.unwrap();
}

#[tokio::test]
async fn migration_preserves_records_ratings_and_history_without_classifying() {
    let (_temp, paths, db) = seeded(3).await;
    let identity = db.catalogue;
    let mut weighted = db.begin_analysis().await.unwrap();
    db.analysis_batch(&mut weighted).await.unwrap();
    let before = canonical(&db).await;
    sqlx::raw_sql(include_str!("fixtures/schema_before_classification.sql"))
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE catalogue SET version=5")
        .execute(&db.pool)
        .await
        .unwrap();
    db.release().await.unwrap();
    db.pool.close().await;
    let db = Catalogue::connect(&paths).await.unwrap();
    assert_eq!(db.catalogue, identity);
    assert_eq!(canonical(&db).await, before);
    assert!(
        kaijucrawl::storage::analysis::score(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        kaijucrawl::storage::analysis::latest(&db.pool)
            .await
            .unwrap()
            .id,
        weighted.id
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM book_classifications")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM analysis_groups WHERE system='book_role'"
        )
        .fetch_one(&db.pool)
        .await
        .unwrap(),
        5
    );
}

#[tokio::test]
async fn groups_are_primary_persisted_and_reused_without_new_requests() {
    let (_temp, paths, db) = seeded(71).await;
    let before = canonical(&db).await;
    let mut status = db.begin_classification(false).await.unwrap();
    for i in 0..71 {
        classify(
            &db,
            &mut status,
            [
                Category::CoreRulebook,
                Category::Supplement,
                Category::Adventure,
                Category::None,
            ][i % 4],
        )
        .await;
    }
    assert!(db.classification_next(&status).await.unwrap().is_none());
    db.finish_analysis(&mut status, "completed", "")
        .await
        .unwrap();
    assert_eq!(status.scored, 71);
    assert_eq!(
        status.classification.as_ref().unwrap().input_tokens,
        71 * 400
    );
    assert_eq!(canonical(&db).await, before);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM book_group_memberships")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        71
    );
    let totals = kaijucrawl::storage::overview::read(&mut db.pool.acquire().await.unwrap())
        .await
        .unwrap();
    let counts = &totals.analyses[classification::ALGORITHM].counts;
    for (role, total) in [
        ("core_rulebook", 18),
        ("supplement", 18),
        ("adventure", 18),
        ("none", 17),
    ] {
        assert_eq!(counts[role], total);
    }
    let c = stored::saved(&db.pool, "drivethru", "1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(c.category, Category::CoreRulebook);
    assert_eq!(c.algorithm, classification::ALGORITHM);
    assert_eq!(c.input_hash.len(), 64);
    db.release().await.unwrap();
    db.pool.close().await;
    let db = Catalogue::connect(&paths).await.unwrap();
    let mut next = db.begin_classification(false).await.unwrap();
    assert_eq!(next.total, 0);
    assert_eq!(next.classification.as_ref().unwrap().already_classified, 71);
    assert!(db.classification_next(&next).await.unwrap().is_none());
    assert_eq!(next.classification.as_ref().unwrap().requests, 0);
    db.finish_analysis(&mut next, "completed", "")
        .await
        .unwrap();
    let force = db.begin_classification(true).await.unwrap();
    assert_eq!(force.total, 71);
    assert_eq!(force.classification.as_ref().unwrap().already_classified, 0);
}

#[tokio::test]
async fn changed_evidence_invalidates_memberships_but_unrelated_edits_and_rollback_preserve_them() {
    let (_temp, _paths, db) = seeded(2).await;
    let mut status = db.begin_classification(false).await.unwrap();
    classify(&db, &mut status, Category::Adventure).await;
    classify(&db, &mut status, Category::Supplement).await;
    db.finish_analysis(&mut status, "completed", "")
        .await
        .unwrap();
    let old = stored::saved(&db.pool, "drivethru", "1")
        .await
        .unwrap()
        .unwrap()
        .calculated_at;
    sqlx::query("UPDATE products SET record=json_set(record,'$.rating',2,'$.price','25') WHERE product_id='1'").execute(&db.pool).await.unwrap();
    assert_eq!(
        stored::saved(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .unwrap()
            .calculated_at,
        old
    );
    let mut tx = db.pool.begin().await.unwrap();
    sqlx::query("UPDATE products SET description_text='Changed' WHERE product_id='1'")
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    assert!(
        stored::saved(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .is_some()
    );
    sqlx::query(
        "UPDATE products SET description_text='A completely different book' WHERE product_id='1'",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    assert!(
        stored::saved(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM book_group_memberships WHERE product_id='1'"
        )
        .fetch_one(&db.pool)
        .await
        .unwrap(),
        0
    );
    let status = db.begin_classification(false).await.unwrap();
    assert_eq!(status.total, 1);
    assert_eq!(status.classification.unwrap().already_classified, 1);
    sqlx::query("DELETE FROM products WHERE product_id='2'")
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(
        stored::saved(&db.pool, "drivethru", "2")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn refusal_is_not_none_and_failed_reclassification_preserves_good_results() {
    let (_temp, _paths, db) = seeded(1).await;
    let mut status = db.begin_classification(false).await.unwrap();
    classify(&db, &mut status, Category::None).await;
    db.finish_analysis(&mut status, "completed", "")
        .await
        .unwrap();
    let old = stored::saved(&db.pool, "drivethru", "1")
        .await
        .unwrap()
        .unwrap()
        .calculated_at;
    let mut rerun = db.begin_classification(true).await.unwrap();
    let book = db.classification_next(&rerun).await.unwrap().unwrap();
    db.classification_dispatch(&mut rerun, &book).await.unwrap();
    db.classification_commit(
        &mut rerun,
        &book,
        Some(&answer(Category::Adventure, true)),
        None,
    )
    .await
    .unwrap();
    assert_eq!(rerun.scored, 0);
    assert_eq!(rerun.skipped, 1);
    assert_eq!(rerun.classification.unwrap().none, 0);
    assert_eq!(
        stored::saved(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .unwrap()
            .calculated_at,
        old
    );
}

#[tokio::test]
async fn failure_rolls_back_result_memberships_progress_and_usage_together() {
    let (_temp, _paths, db) = seeded(1).await;
    let mut status = db.begin_classification(false).await.unwrap();
    let book = db.classification_next(&status).await.unwrap().unwrap();
    db.classification_dispatch(&mut status, &book)
        .await
        .unwrap();
    sqlx::raw_sql("CREATE TRIGGER fixture_fail BEFORE INSERT ON book_group_memberships BEGIN SELECT RAISE(FAIL,'Synthetic failure'); END;").execute(&db.pool).await.unwrap();
    assert!(
        db.classification_commit(
            &mut status,
            &book,
            Some(&answer(Category::Adventure, false)),
            None
        )
        .await
        .is_err()
    );
    assert_eq!(status.processed, 0);
    assert_eq!(status.classification.as_ref().unwrap().input_tokens, 0);
    assert!(
        stored::saved(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .is_none()
    );
    let latest = kaijucrawl::storage::analysis::latest(&db.pool)
        .await
        .unwrap();
    assert_eq!(latest.processed, 0);
    assert_eq!(latest.classification.unwrap().requests, 1);
}

#[tokio::test]
async fn interrupted_dispatch_and_cancel_resume_only_unfinished_books_with_stale_writer_rejection()
{
    let (_temp, paths, db) = seeded(3).await;
    let mut status = db.begin_classification(false).await.unwrap();
    classify(&db, &mut status, Category::CoreRulebook).await;
    let book = db.classification_next(&status).await.unwrap().unwrap();
    db.classification_dispatch(&mut status, &book)
        .await
        .unwrap();
    db.release().await.unwrap();
    let next = Catalogue::connect(&paths).await.unwrap();
    let old = kaijucrawl::storage::analysis::latest(&next.pool)
        .await
        .unwrap();
    assert_eq!(old.state, "interrupted");
    assert_eq!(old.processed, 1);
    assert_eq!(old.classification.unwrap().requests, 2);
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT state FROM classification_attempts WHERE document=2"
        )
        .fetch_one(&next.pool)
        .await
        .unwrap(),
        "interrupted"
    );
    assert!(
        db.classification_commit(
            &mut status,
            &book,
            Some(&answer(Category::Adventure, false)),
            None
        )
        .await
        .is_err()
    );
    let mut resumed = next.begin_classification(false).await.unwrap();
    assert_eq!(resumed.total, 2);
    classify(&next, &mut resumed, Category::Adventure).await;
    next.finish_analysis(&mut resumed, "cancelled", "")
        .await
        .unwrap();
    let remainder = next.begin_classification(false).await.unwrap();
    assert_eq!(remainder.total, 1);
    assert_eq!(remainder.classification.unwrap().already_classified, 2);
}

#[tokio::test]
async fn oversized_evidence_does_not_send_or_truncate() {
    let (_temp, _paths, db) = seeded(52).await;
    let mut status = db.begin_classification(false).await.unwrap();
    for _ in 0..51 {
        classify(&db, &mut status, Category::Adventure).await;
    }
    sqlx::query("UPDATE products SET description_text=? WHERE product_id='52'")
        .bind("large".repeat(30000))
        .execute(&db.pool)
        .await
        .unwrap();
    let book = db.classification_next(&status).await.unwrap().unwrap();
    assert!(book.evidence.is_none());
    db.classification_commit(&mut status, &book, None, Some("Evidence too large"))
        .await
        .unwrap();
    assert_eq!(status.classification.as_ref().unwrap().requests, 51);
    assert_eq!(status.skipped, 1);
    assert!(
        stored::saved(&db.pool, "drivethru", "52")
            .await
            .unwrap()
            .is_none()
    );
    sqlx::query("UPDATE products SET description_text='Changed' WHERE product_id='1'")
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(
        stored::saved(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .is_none()
    );
}
