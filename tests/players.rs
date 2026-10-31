//! Synthetic player-count persistence, migration, eligibility; no network.
use kaijucrawl::{
    analysis::Status,
    decisions::{InputTokens, Usage},
    domain::CATALOGUE_VERSION,
    paths::LocalPaths,
    players::{Counts, Response},
    storage::{players, sqlite::Catalogue},
};

async fn seeded(n: i64) -> (tempfile::TempDir, LocalPaths, Catalogue) {
    let temp = tempfile::tempdir().unwrap();
    let paths = LocalPaths::at(temp.path());
    let db = Catalogue::connect(&paths).await.unwrap();
    let intent = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO intents(id,target,product_id,payload,generation,finalized) VALUES(?,'drivethru','1','{}',?,1)").bind(intent).bind(db.generation).execute(&db.pool).await.unwrap();
    sqlx::query("WITH RECURSIVE n(i) AS(VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<?) INSERT INTO products(target,product_id,title,description_text,record,parser_version,record_schema_version,archive,fetched_at,last_checked_at,intent) SELECT 'drivethru',CAST(i AS TEXT),'Synthetic adventure '||i,'For three to five players plus a GM.','{\"raw_metadata\":\"DO_NOT_SEND\"}','drivethru-rust-1',2,'{}','2026-01-01','2026-01-01',? FROM n").bind(n).bind(intent).execute(&db.pool).await.unwrap();
    let mut run = db.begin_classification(false).await.unwrap();
    sqlx::query("INSERT INTO book_classifications(target,product_id,system,category,algorithm,model,confidence,probabilities,input_hash,calculated_at,run_id) SELECT target,product_id,'book_role',CASE WHEN CAST(product_id AS INTEGER)%4=0 THEN 'supplement' ELSE 'adventure' END,'primary-book-role-1','gpt-6-luna',1,'[]','synthetic','2026-01-01',? FROM products").bind(run.id).execute(&db.pool).await.unwrap();
    sqlx::query("INSERT INTO book_group_memberships SELECT target,product_id,system,category FROM book_classifications").execute(&db.pool).await.unwrap();
    db.finish_analysis(&mut run, "completed", "").await.unwrap();
    (temp, paths, db)
}
fn answer(min: Option<i32>, max: Option<i32>) -> Response {
    Response {
        model: "gpt-6-luna".into(),
        counts: Some(Counts {
            min_players: min,
            max_players: max,
        }),
        usage: Usage {
            input_tokens: 1500,
            output_tokens: 20,
            total_tokens: 1520,
            input_tokens_details: InputTokens {
                cached_tokens: 1100,
                cache_write_tokens: 0,
            },
        },
    }
}
async fn next(
    db: &Catalogue,
    s: &Status,
    after: i64,
) -> kaijucrawl::storage::classification::Candidate {
    db.players_next_after(s, after).await.unwrap().unwrap()
}
async fn canonical(db: &Catalogue) -> Vec<(String, String, String)> {
    sqlx::query_as("SELECT product_id,record,fetched_at FROM products ORDER BY product_id")
        .fetch_all(&db.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn only_adventures_run_and_known_and_null_results_are_saved() {
    let (_temp, _paths, db) = seeded(80).await;
    let before = canonical(&db).await;
    let mut s = db.begin_players(false).await.unwrap();
    assert_eq!(s.total, 60);
    let mut after = 0;
    while let Some(b) = db.players_next_after(&s, after).await.unwrap() {
        after = b.document;
        assert!(!b.evidence.as_ref().unwrap().contains("DO_NOT_SEND"));
        db.classification_dispatch(&mut s, &b).await.unwrap();
        let a = match b.document % 3 {
            0 => answer(None, None),
            1 => answer(Some(3), Some(5)),
            _ => answer(Some(3), None),
        };
        db.players_commit(&mut s, &b, Some(&a), None).await.unwrap();
        assert!(db.players_commit(&mut s, &b, Some(&a), None).await.is_err());
    }
    assert_eq!((s.processed, s.scored, s.skipped), (60, 60, 0));
    let p = s.classification.as_ref().unwrap();
    assert_eq!(
        (p.known_players, p.unknown_players, p.requests),
        (40, 20, 60)
    );
    db.finish_analysis(&mut s, "completed", "").await.unwrap();
    assert_eq!(canonical(&db).await, before);
    let mut skip = db.begin_players(false).await.unwrap();
    assert_eq!(skip.total, 0);
    db.finish_analysis(&mut skip, "completed", "")
        .await
        .unwrap();
    let unknown = players::saved(&db.pool, "drivethru", "3")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        unknown.counts,
        Counts {
            min_players: None,
            max_players: None
        }
    );
    assert!(
        players::saved(&db.pool, "drivethru", "4")
            .await
            .unwrap()
            .is_none()
    );
    let known = players::saved(&db.pool, "drivethru", "1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(known.counts.min_players, Some(3));
    assert_eq!(known.counts.max_players, Some(5));
    let partial = players::saved(&db.pool, "drivethru", "2")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(partial.counts.min_players, Some(3));
    assert_eq!(partial.counts.max_players, None);
    db.release().await.unwrap();
}
#[tokio::test]
async fn migration_preserves_catalogue_and_classifications_without_running_analysis() {
    let (_temp, paths, db) = seeded(4).await;
    let id = db.catalogue;
    let before = canonical(&db).await;
    let roles: i64 = sqlx::query_scalar("SELECT count(*) FROM book_classifications")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!("fixtures/schema_before_players.sql"))
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE catalogue SET version=6")
        .execute(&db.pool)
        .await
        .unwrap();
    db.release().await.unwrap();
    db.pool.close().await;
    let next = Catalogue::connect(&paths).await.unwrap();
    assert_eq!(next.catalogue, id);
    assert_eq!(canonical(&next).await, before);
    let version: i32 = sqlx::query_scalar("SELECT version FROM catalogue")
        .fetch_one(&next.pool)
        .await
        .unwrap();
    assert_eq!(version, CATALOGUE_VERSION);
    let kept: i64 = sqlx::query_scalar("SELECT count(*) FROM book_classifications")
        .fetch_one(&next.pool)
        .await
        .unwrap();
    assert_eq!(kept, roles);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM adventure_players")
        .fetch_one(&next.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert_eq!(next.begin_players(false).await.unwrap().total, 3);
    next.release().await.unwrap();
}
#[tokio::test]
async fn role_and_description_changes_invalidate_counts_while_unrelated_changes_keep_them() {
    let (_temp, _paths, db) = seeded(3).await;
    let mut s = db.begin_players(false).await.unwrap();
    for after in 0..3 {
        let b = next(&db, &s, after).await;
        db.classification_dispatch(&mut s, &b).await.unwrap();
        db.players_commit(&mut s, &b, Some(&answer(Some(3), Some(5))), None)
            .await
            .unwrap();
    }
    db.finish_analysis(&mut s, "completed", "").await.unwrap();
    sqlx::query("UPDATE products SET price='5' WHERE product_id='1'")
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(
        players::saved(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .is_some()
    );
    sqlx::query("UPDATE book_classifications SET confidence=0.5 WHERE product_id='1'")
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(
        players::saved(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .is_some()
    );
    sqlx::query("UPDATE book_classifications SET category='supplement' WHERE product_id='1'")
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE products SET description_text='Changed full description' WHERE product_id='2'",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query("DELETE FROM products WHERE product_id='3'")
        .execute(&db.pool)
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM adventure_players")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert_eq!(db.begin_players(false).await.unwrap().total, 0);
    db.release().await.unwrap();
}
#[tokio::test]
async fn refusals_and_write_failures_preserve_prior_results_and_progress_atomically() {
    let (_temp, _paths, db) = seeded(1).await;
    let mut s = db.begin_players(false).await.unwrap();
    let b = next(&db, &s, 0).await;
    db.classification_dispatch(&mut s, &b).await.unwrap();
    db.players_commit(&mut s, &b, Some(&answer(Some(3), Some(5))), None)
        .await
        .unwrap();
    db.finish_analysis(&mut s, "completed", "").await.unwrap();
    let mut s = db.begin_players(true).await.unwrap();
    let b = next(&db, &s, 0).await;
    db.classification_dispatch(&mut s, &b).await.unwrap();
    let mut refusal = answer(None, None);
    refusal.counts = None;
    db.players_commit(&mut s, &b, Some(&refusal), None)
        .await
        .unwrap();
    assert_eq!(s.skipped, 1);
    assert_eq!(
        players::saved(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .unwrap()
            .counts
            .max_players,
        Some(5)
    );
    db.finish_analysis(&mut s, "completed", "").await.unwrap();
    let mut s = db.begin_players(true).await.unwrap();
    let b = next(&db, &s, 0).await;
    db.classification_dispatch(&mut s, &b).await.unwrap();
    sqlx::raw_sql("CREATE TRIGGER fail_player_write BEFORE UPDATE ON adventure_players BEGIN SELECT RAISE(ABORT,'Synthetic storage failure'); END;").execute(&db.pool).await.unwrap();
    assert!(
        db.players_commit(&mut s, &b, Some(&answer(Some(2), Some(6))), None)
            .await
            .is_err()
    );
    assert_eq!((s.processed, s.scored), (0, 0));
    assert_eq!(
        players::saved(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .unwrap()
            .counts
            .max_players,
        Some(5)
    );
    db.release().await.unwrap();
}
#[tokio::test]
async fn retry_takeover_reselects_unfinished_gaps_rejects_stale_writers() {
    let (_temp, paths, db) = seeded(72).await;
    let mut s = db.begin_players(false).await.unwrap();
    let first = next(&db, &s, 0).await;
    let second = next(&db, &s, first.document).await;
    db.classification_dispatch(&mut s, &first).await.unwrap();
    db.classification_dispatch(&mut s, &second).await.unwrap();
    db.classification_retry_wait(&s, &first, "Network connection failed")
        .await
        .unwrap();
    db.players_commit(&mut s, &second, Some(&answer(None, None)), None)
        .await
        .unwrap();
    db.release().await.unwrap();
    let successor = Catalogue::connect(&paths).await.unwrap();
    assert!(db.classification_redispatch(&mut s, &first).await.is_err());
    assert!(
        db.players_commit(&mut s, &first, Some(&answer(Some(1), Some(1))), None)
            .await
            .is_err()
    );
    let remaining = successor.begin_players(false).await.unwrap();
    assert_eq!(remaining.total, 53);
    assert_eq!(
        next(&successor, &remaining, 0).await.document,
        first.document
    );
    successor.release().await.unwrap();
}
