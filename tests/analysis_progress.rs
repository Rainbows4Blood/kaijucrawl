//! Live totals must match authoritative aggregates, including replacements and rollbacks.
use kaijucrawl::{
    analysis::Status,
    decisions::{self, Decision, InputTokens, Usage},
    dnd,
    paths::LocalPaths,
    players::{Counts, Response},
    storage::{overview, sqlite::Catalogue},
};
use serde_json::{Value, json};

async fn seeded(n: i64) -> (tempfile::TempDir, Catalogue) {
    let temp = tempfile::tempdir().unwrap();
    let db = Catalogue::connect(&LocalPaths::at(temp.path()))
        .await
        .unwrap();
    let intent = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO intents(id,target,product_id,payload,generation,finalized) VALUES(?,'drivethru','1','{}',?,1)").bind(intent).bind(db.generation).execute(&db.pool).await.unwrap();
    sqlx::query("WITH RECURSIVE n(i) AS(VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<?) INSERT INTO products(target,product_id,title,description_text,record,parser_version,record_schema_version,archive,fetched_at,last_checked_at,intent) SELECT 'drivethru',CAST(i AS TEXT),'Synthetic book','Full fictional description',CASE WHEN i%2=0 THEN '{}' ELSE '{\"rating\":4.7,\"rating_count\":1000}' END,'drivethru-rust-1',2,'{}','2026-01-01','2026-01-01',? FROM n").bind(n).bind(intent).execute(&db.pool).await.unwrap();
    (temp, db)
}
async fn check(db: &Catalogue, status: &Status) {
    let mut conn = db.pool.acquire().await.unwrap();
    let mut expected = overview::read(&mut conn).await.unwrap();
    let mut actual = status.overview.clone().expect("live totals present");
    // Missing and explicitly zero category buckets have the same displayed meaning.
    for totals in [&mut expected, &mut actual] {
        for coverage in totals.analyses.values_mut() {
            coverage.counts.retain(|_, n| *n != 0);
        }
    }
    assert_eq!(
        serde_json::to_value(actual).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
}
fn choice(name: &str, value: &str) -> Value {
    json!({"type":"choice","name":name,"choice":value,"confidence":1.0,"probabilities":[{"value":value,"probability":1.0}]})
}
fn decision(answers: Vec<Value>) -> Decision {
    serde_json::from_value(json!({"model":decisions::MODEL,"answers":answers,"usage":{"input_tokens":100,"output_tokens":0,"total_tokens":100,"input_tokens_details":{"cached_tokens":0,"cache_write_tokens":0}}})).unwrap()
}
async fn classify(db: &Catalogue, s: &mut Status, answers: Vec<Value>) {
    let book = db.classification_next(s).await.unwrap().unwrap();
    db.classification_dispatch(s, &book).await.unwrap();
    db.classification_commit(s, &book, Some(&decision(answers)), None)
        .await
        .unwrap();
    check(db, s).await;
}
async fn finish(db: &Catalogue, s: &mut Status) {
    db.finish_analysis(s, "completed", "").await.unwrap();
    check(db, s).await;
}

#[tokio::test]
async fn live_online_totals_replace_categories_and_dependent_results_only_after_commit() {
    let (_temp, db) = seeded(2).await;
    let original: Vec<String> =
        sqlx::query_scalar("SELECT record FROM products ORDER BY product_id")
            .fetch_all(&db.pool)
            .await
            .unwrap();
    let mut role = db.begin_classification(false).await.unwrap();
    check(&db, &role).await;
    for _ in 0..2 {
        classify(
            &db,
            &mut role,
            vec![choice("primary_book_role", "adventure")],
        )
        .await;
    }
    finish(&db, &mut role).await;

    let mut players = db.begin_players(false).await.unwrap();
    let mut after = 0;
    while let Some(book) = db.players_next_after(&players, after).await.unwrap() {
        after = book.document;
        db.classification_dispatch(&mut players, &book)
            .await
            .unwrap();
        let response = Response {
            model: "gpt-6-luna".into(),
            counts: Some(Counts {
                min_players: (after == 1).then_some(3),
                max_players: None,
            }),
            usage: Usage {
                input_tokens: 100,
                output_tokens: 10,
                total_tokens: 110,
                input_tokens_details: InputTokens {
                    cached_tokens: 0,
                    cache_write_tokens: 0,
                },
            },
        };
        db.players_commit(&mut players, &book, Some(&response), None)
            .await
            .unwrap();
        check(&db, &players).await;
    }
    finish(&db, &mut players).await;
    let mut players = db.begin_players(true).await.unwrap();
    let mut after = 0;
    while let Some(book) = db.players_next_after(&players, after).await.unwrap() {
        after = book.document;
        db.classification_dispatch(&mut players, &book)
            .await
            .unwrap();
        let response = Response {
            model: "gpt-6-luna".into(),
            counts: Some(Counts {
                min_players: (after == 2).then_some(4),
                max_players: None,
            }),
            usage: Usage {
                input_tokens: 100,
                output_tokens: 10,
                total_tokens: 110,
                input_tokens_details: InputTokens {
                    cached_tokens: 0,
                    cache_write_tokens: 0,
                },
            },
        };
        db.players_commit(&mut players, &book, Some(&response), None)
            .await
            .unwrap();
        check(&db, &players).await;
    }
    finish(&db, &mut players).await;
    let mut solo = db.begin_solo_classification(false).await.unwrap();
    for category in ["gamebook", "neither"] {
        classify(&db, &mut solo, vec![choice("solo_play", category)]).await;
    }
    finish(&db, &mut solo).await;
    let mut compatible = db.begin_dnd(false, false).await.unwrap();
    for category in ["dnd_only", "dnd_and_others"] {
        classify(&db, &mut compatible, vec![choice("dnd", category)]).await;
    }
    finish(&db, &mut compatible).await;
    let mut editions = db.begin_dnd(true, false).await.unwrap();
    for known in [true, false] {
        classify(
            &db,
            &mut editions,
            dnd::EDITIONS
                .iter()
                .map(|(key, _)| choice(key, if known { "supported" } else { "uncertain" }))
                .collect(),
        )
        .await;
    }
    finish(&db, &mut editions).await;
    let mut editions = db.begin_dnd(true, true).await.unwrap();
    for selected in ["5e_2024", "bx"] {
        classify(
            &db,
            &mut editions,
            dnd::EDITIONS
                .iter()
                .map(|(key, _)| {
                    choice(
                        key,
                        if *key == selected {
                            "supported"
                        } else {
                            "uncertain"
                        },
                    )
                })
                .collect(),
        )
        .await;
    }
    finish(&db, &mut editions).await;
    // Replacing a prerequisite removes only the appropriate dependent results.
    let mut role = db.begin_classification(true).await.unwrap();
    classify(
        &db,
        &mut role,
        vec![choice("primary_book_role", "supplement")],
    )
    .await;
    classify(
        &db,
        &mut role,
        vec![choice("primary_book_role", "adventure")],
    )
    .await;
    finish(&db, &mut role).await;
    let mut compatible = db.begin_dnd(false, true).await.unwrap();
    classify(&db, &mut compatible, vec![choice("dnd", "no")]).await;
    classify(&db, &mut compatible, vec![choice("dnd", "dnd_only")]).await;
    finish(&db, &mut compatible).await;

    let mut solo = db.begin_solo_classification(true).await.unwrap();
    let book = db.classification_next(&solo).await.unwrap().unwrap();
    db.classification_dispatch(&mut solo, &book).await.unwrap();
    let before = serde_json::to_value(&solo.overview).unwrap();
    sqlx::raw_sql("CREATE TRIGGER fail_live_summary BEFORE INSERT ON book_group_memberships BEGIN SELECT RAISE(ABORT,'Synthetic failure'); END;").execute(&db.pool).await.unwrap();
    assert!(
        db.classification_commit(
            &mut solo,
            &book,
            Some(&decision(vec![choice("solo_play", "neither")])),
            None
        )
        .await
        .is_err()
    );
    assert_eq!(serde_json::to_value(&solo.overview).unwrap(), before);
    check(&db, &solo).await;
    sqlx::raw_sql("DROP TRIGGER fail_live_summary")
        .execute(&db.pool)
        .await
        .unwrap();
    db.classification_commit(
        &mut solo,
        &book,
        Some(&decision(vec![
            json!({"type":"refusal","name":"solo_play"}),
        ])),
        None,
    )
    .await
    .unwrap();
    check(&db, &solo).await;
    assert_eq!(serde_json::to_value(&solo.overview).unwrap(), before);
    db.finish_analysis(&mut solo, "cancelled", "")
        .await
        .unwrap();
    assert!(
        kaijucrawl::storage::analysis::latest(&db.pool)
            .await
            .unwrap()
            .overview
            .is_none()
    );
    let current: Vec<String> =
        sqlx::query_scalar("SELECT record FROM products ORDER BY product_id")
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(original, current);
    db.release().await.unwrap();
    db.pool.close().await;
}

#[tokio::test]
async fn live_weighted_totals_include_prior_batches_nulls_and_rollback_without_double_counting() {
    let (_temp, db) = seeded(501).await;
    let mut run = db.begin_analysis().await.unwrap();
    check(&db, &run).await;
    assert!(!db.analysis_batch(&mut run).await.unwrap());
    check(&db, &run).await;
    let before = serde_json::to_value(&run.overview).unwrap();
    sqlx::raw_sql("CREATE TRIGGER fail_live_score BEFORE INSERT ON relevance_scores BEGIN SELECT RAISE(ABORT,'Synthetic failure'); END;").execute(&db.pool).await.unwrap();
    assert!(db.analysis_batch(&mut run).await.is_err());
    assert_eq!(serde_json::to_value(&run.overview).unwrap(), before);
    check(&db, &run).await;
    sqlx::raw_sql("DROP TRIGGER fail_live_score")
        .execute(&db.pool)
        .await
        .unwrap();
    db.finish_analysis(&mut run, "cancelled", "").await.unwrap();
    let mut rerun = db.begin_analysis().await.unwrap();
    check(&db, &rerun).await;
    while !db.analysis_batch(&mut rerun).await.unwrap() {
        check(&db, &rerun).await;
    }
    check(&db, &rerun).await;
    let coverage = &rerun.overview.as_ref().unwrap().analyses[kaijucrawl::analysis::ALGORITHM];
    assert_eq!(coverage.completed, 501);
    assert_eq!(coverage.counts["unknown"], 250);
    db.release().await.unwrap();
    db.pool.close().await;
}
