use kaijucrawl::{
    analysis::Status,
    decisions,
    osr_levels::{self, Response},
    paths::LocalPaths,
    responses,
    storage::{osr_levels as stored, overview, sqlite::Catalogue},
};
use serde_json::{Value, json};
use sqlx::Row;

fn range(
    system: &str,
    game: Option<&str>,
    status: &str,
    min: Option<i32>,
    max: Option<i32>,
) -> Value {
    json!({"system":system,"game":game,"status":status,"min_level":min,"max_level":max})
}
fn decoded(ranges: Vec<Value>, systems: &[&str]) -> decisions::Result<Response> {
    osr_levels::decode_response(
        responses::Response {
            model: responses::MODEL.into(),
            value: Some(json!({"ranges":ranges})),
            usage: serde_json::from_value(json!({"input_tokens":10,"output_tokens":5,"total_tokens":15,"input_tokens_details":{"cached_tokens":0,"cache_write_tokens":0}})).unwrap(),
        },
        &systems.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
    )
}
fn evidence(systems: &[&str]) -> Value {
    json!({"book":{"title":"Fictional adventure","description":"Original full description"},"requested_systems":systems})
}

#[test]
fn independent_ranges_strict_scope_unknown_and_level_free_outputs() {
    let r = decoded(
        vec![
            range("old_school_essentials", None, "known", Some(3), Some(5)),
            range("dungeon_crawl_classics", None, "known", Some(0), Some(0)),
            range("cairn", None, "not_applicable", None, None),
            range("other", Some("Knave"), "known", Some(1), None),
            range("other", Some("Basic Fantasy RPG"), "known", None, Some(4)),
        ],
        &[
            "old_school_essentials",
            "dungeon_crawl_classics",
            "cairn",
            "other",
        ],
    )
    .unwrap();
    assert_eq!(r.data.unwrap().ranges.len(), 5);
    assert!(
        decoded(
            vec![range("osric", None, "known", Some(21), Some(36))],
            &["osric"]
        )
        .is_ok()
    );
    assert!(
        decoded(
            vec![range("other", None, "unknown", None, None)],
            &["other"]
        )
        .is_ok()
    );
    for bad in [
        vec![],
        vec![range("osric", None, "known", None, None)],
        vec![range("osric", None, "unknown", Some(1), None)],
        vec![range("osric", None, "not_applicable", None, Some(2))],
        vec![range("osric", None, "known", Some(-1), Some(3))],
        vec![range("osric", None, "known", Some(4), Some(2))],
        vec![range("cairn", None, "unknown", None, None)],
        vec![range(
            "osric",
            Some("Wrong game"),
            "known",
            Some(2),
            Some(3),
        )],
        vec![
            range("osric", None, "unknown", None, None),
            range("osric", None, "unknown", None, None),
        ],
        vec![json!({"system":"osric","game":null,"status":"known","min_level":2})],
        vec![json!({"system":"osric","game":null,"status":"known","min_level":2.5,"max_level":3})],
    ] {
        assert!(decoded(bad, &["osric"]).is_err());
    }
    assert!(
        decoded(
            vec![range("osric", None, "unknown", None, None)],
            &["osric", "cairn"]
        )
        .is_err()
    );
    assert!(
        decoded(
            vec![
                range("other", Some("Knave"), "unknown", None, None),
                range("other", Some("KNAVE"), "unknown", None, None)
            ],
            &["other"]
        )
        .is_err()
    );
    assert!(
        decoded(
            vec![range("other", None, "known", Some(1), Some(2))],
            &["other"]
        )
        .is_err()
    );
    assert!(
        decoded(
            vec![
                range("other", None, "unknown", None, None),
                range("other", Some("Knave"), "unknown", None, None)
            ],
            &["other"]
        )
        .is_err()
    );
    let refused = osr_levels::decode_response(
        responses::Response {
            model: responses::MODEL.into(),
            value: None,
            usage: serde_json::from_value(json!({"input_tokens":10,"output_tokens":5,"total_tokens":15,"input_tokens_details":{"cached_tokens":0,"cache_write_tokens":0}})).unwrap(),
        },
        &["osric".into()],
    )
    .unwrap();
    assert!(refused.data.is_none());
}

#[test]
fn cached_luna_request_and_escalation_preserve_system_scope_and_evidence() {
    let input = evidence(&["osric", "other"]);
    let mut bytes = osr_levels::encode(&input).unwrap();
    let original: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(original["store"], false);
    assert_eq!(original["prompt_cache_options"]["mode"], "explicit");
    assert!(original["input"][0]["content"][0]["prompt_cache_breakpoint"].is_object());
    assert!(original["input"][1]["content"][0]["prompt_cache_breakpoint"].is_null());
    assert_eq!(
        serde_json::from_str::<Value>(original["input"][1]["content"][0]["text"].as_str().unwrap())
            .unwrap(),
        input
    );
    for model in responses::Model::ALL {
        let current: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(current["model"], model.name());
        assert_eq!(current["input"], original["input"]);
        assert_eq!(current["text"], original["text"]);
        assert_eq!(responses::escalate(&mut bytes).unwrap(), model.next());
    }
    for scope in [json!([]), json!(["not_osr"]), json!(["osric", "osric"])] {
        let mut invalid = input.clone();
        invalid["requested_systems"] = scope;
        assert!(osr_levels::encode(&invalid).is_err());
    }
}

async fn classify(
    db: &Catalogue,
    id: &str,
    system: &str,
    category: &str,
    algorithm: &str,
    members: &[&str],
    run: i64,
) {
    sqlx::query("INSERT INTO book_classifications VALUES('drivethru',?,?,?,?, 'synthetic',1,'[]','source','2026-01-01',?)")
        .bind(id).bind(system).bind(category).bind(algorithm).bind(run).execute(&db.pool).await.unwrap();
    for m in members {
        sqlx::query("INSERT INTO book_group_memberships VALUES('drivethru',?,?,?)")
            .bind(id)
            .bind(system)
            .bind(m)
            .execute(&db.pool)
            .await
            .unwrap();
    }
}
async fn seed() -> (tempfile::TempDir, LocalPaths, Catalogue) {
    let temp = tempfile::tempdir().unwrap();
    let paths = LocalPaths::at(temp.path());
    let db = Catalogue::connect(&paths).await.unwrap();
    let intent = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO intents(id,target,product_id,payload,generation,finalized) VALUES(?,'drivethru','1','{}',?,1)").bind(intent).bind(db.generation).execute(&db.pool).await.unwrap();
    let mut seed_run = db.begin_classification(false).await.unwrap();
    let run = seed_run.id.unwrap();
    for i in 1..=8 {
        let id = i.to_string();
        let record = json!({"rule_systems":if i==3 {vec!["OSRIC"]}else{vec!["Old-School Revival (OSR)"]},"min_players":99,"old_analysis":"DO_NOT_SEND","rating":4.7,"rating_count":1000});
        sqlx::query("INSERT INTO products(target,product_id,title,description_text,record,parser_version,record_schema_version,archive,fetched_at,last_checked_at,intent) VALUES('drivethru',?,'Fictional adventure','Original full description',?,'drivethru-rust-1',2,'{}','2026-01-01','2026-01-01',?)").bind(&id).bind(record.to_string()).bind(intent).execute(&db.pool).await.unwrap();
        if i != 6 {
            classify(
                &db,
                &id,
                "book_role",
                if i == 2 { "supplement" } else { "adventure" },
                kaijucrawl::classification::ALGORITHM,
                &[],
                run,
            )
            .await;
        }
        if i != 3 && i != 5 {
            let members: &[&str] = match i {
                1 => &["old_school_essentials", "dungeon_crawl_classics"],
                4 => &["not_osr"],
                7 => &["other", "swords_wizardry"],
                _ => &["cairn"],
            };
            classify(
                &db,
                &id,
                "osr",
                if i == 4 { "not_osr" } else { "identified" },
                kaijucrawl::osr::ALGORITHM,
                members,
                run,
            )
            .await;
        }
    }
    db.finish_analysis(&mut seed_run, "completed", "")
        .await
        .unwrap();
    (temp, paths, db)
}
async fn live_matches(db: &Catalogue, s: &Status) {
    let mut actual = overview::read(&mut db.pool.acquire().await.unwrap())
        .await
        .unwrap();
    let mut expected = s.overview.clone().unwrap();
    for o in [&mut actual, &mut expected] {
        for c in o.analyses.values_mut() {
            c.counts.retain(|_, v| *v != 0);
        }
    }
    assert_eq!(
        serde_json::to_value(actual).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
}
fn result(id: &str) -> Response {
    match id {
        "1" => decoded(
            vec![
                range("dungeon_crawl_classics", None, "known", Some(0), Some(0)),
                range("old_school_essentials", None, "known", Some(3), Some(5)),
            ],
            &["dungeon_crawl_classics", "old_school_essentials"],
        ),
        "7" => decoded(
            vec![
                range("other", Some("Knave"), "known", Some(1), Some(2)),
                range(
                    "other",
                    Some("Basic Fantasy RPG"),
                    "known",
                    Some(3),
                    Some(4),
                ),
                range("swords_wizardry", None, "unknown", None, None),
            ],
            &["other", "swords_wizardry"],
        ),
        _ => decoded(
            vec![range("cairn", None, "not_applicable", None, None)],
            &["cairn"],
        ),
    }
    .unwrap()
}
async fn capture(db: &Catalogue) -> Vec<(i64, String, String, String)> {
    sqlx::query_as("SELECT rowid,record,archive,fetched_at FROM products ORDER BY rowid")
        .fetch_all(&db.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn adventure_subset_multi_system_results_live_totals_reuse_and_atomic_replacement() {
    let (_temp, paths, db) = seed().await;
    let canonical = capture(&db).await;
    let mut run = db.begin_osr_levels(false).await.unwrap();
    assert_eq!(run.total, 3);
    let totals = &run.overview.as_ref().unwrap().analyses[osr_levels::ALGORITHM];
    assert_eq!((totals.eligible, totals.prerequisite_pending), (3, 2));
    let mut books = Vec::new();
    let mut after = 0;
    while let Some(book) = db.osr_levels_next_after(&run, after).await.unwrap() {
        after = book.document;
        assert!(!book.evidence.as_ref().unwrap().contains("DO_NOT_SEND"));
        db.classification_dispatch(&mut run, &book).await.unwrap();
        books.push(book);
    }
    assert_eq!(
        books
            .iter()
            .map(|b| b.product_id.as_str())
            .collect::<Vec<_>>(),
        ["1", "7", "8"]
    );
    for b in books.iter().rev() {
        db.osr_levels_commit(&mut run, b, Some(&result(&b.product_id)), None)
            .await
            .unwrap();
        live_matches(&db, &run).await;
    }
    let totals = &run.overview.as_ref().unwrap().analyses[osr_levels::ALGORITHM];
    assert_eq!(
        (
            totals.completed,
            totals.counts["known"],
            totals.counts["unknown"],
            totals.counts["not_applicable"]
        ),
        (3, 2, 1, 1)
    );
    assert_eq!(
        stored::saved(&db.pool, "drivethru", "7")
            .await
            .unwrap()
            .unwrap()
            .data
            .ranges
            .len(),
        3
    );
    db.finish_analysis(&mut run, "completed", "").await.unwrap();
    let mut reused = db.begin_osr_levels(false).await.unwrap();
    assert_eq!(reused.total, 0);
    db.finish_analysis(&mut reused, "completed", "")
        .await
        .unwrap();
    let mut again = db.begin_osr_levels(true).await.unwrap();
    let b = db.osr_levels_next_after(&again, 0).await.unwrap().unwrap();
    let unknown = decoded(
        vec![
            range("dungeon_crawl_classics", None, "unknown", None, None),
            range("old_school_essentials", None, "unknown", None, None),
        ],
        &["dungeon_crawl_classics", "old_school_essentials"],
    )
    .unwrap();
    sqlx::raw_sql("CREATE TRIGGER reject_level_fixture BEFORE INSERT ON osr_level_ranges WHEN new.system='old_school_essentials' BEGIN SELECT RAISE(ABORT,'synthetic rollback'); END;").execute(&db.pool).await.unwrap();
    assert!(
        db.osr_levels_commit(&mut again, &b, Some(&unknown), None)
            .await
            .is_err()
    );
    assert_eq!(again.processed, 0);
    assert_eq!(
        stored::saved(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .unwrap()
            .data,
        result("1").data.unwrap()
    );
    sqlx::raw_sql("DROP TRIGGER reject_level_fixture")
        .execute(&db.pool)
        .await
        .unwrap();
    db.osr_levels_commit(&mut again, &b, Some(&unknown), None)
        .await
        .unwrap();
    live_matches(&db, &again).await;
    assert!(
        !stored::saved(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .unwrap()
            .data
            .known()
    );
    db.finish_analysis(&mut again, "completed", "")
        .await
        .unwrap();
    let mut refused = db.begin_osr_levels(true).await.unwrap();
    let b = db
        .osr_levels_next_after(&refused, 0)
        .await
        .unwrap()
        .unwrap();
    db.osr_levels_commit(&mut refused, &b, None, Some("Synthetic exhausted response"))
        .await
        .unwrap();
    assert_eq!(refused.skipped, 1);
    assert!(
        stored::saved(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(capture(&db).await, canonical);
    db.release().await.unwrap();
    let reopened = Catalogue::connect(&paths).await.unwrap();
    assert_eq!(reopened.begin_osr_levels(false).await.unwrap().total, 0);
    reopened.release().await.unwrap();
}

#[tokio::test]
async fn source_roles_and_memberships_invalidate_without_stale_or_partial_writes() {
    let (_temp, paths, db) = seed().await;
    let mut run = db.begin_osr_levels(false).await.unwrap();
    while let Some(b) = db.osr_levels_next_after(&run, 0).await.unwrap() {
        db.osr_levels_commit(&mut run, &b, Some(&result(&b.product_id)), None)
            .await
            .unwrap();
    }
    sqlx::query("UPDATE products SET record=json_set(record,'$.rating',4.9) WHERE product_id='1'")
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(
        stored::saved(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .is_some()
    );
    db.finish_analysis(&mut run, "completed", "").await.unwrap();
    let mut rerun = db.begin_osr_levels(true).await.unwrap();
    let b = db.osr_levels_next_after(&rerun, 0).await.unwrap().unwrap();
    sqlx::query("DELETE FROM book_group_memberships WHERE product_id='1' AND system='osr' AND group_key='old_school_essentials'").execute(&db.pool).await.unwrap();
    assert!(
        stored::saved(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        db.osr_levels_commit(&mut rerun, &b, Some(&result("1")), None)
            .await
            .is_err()
    );
    assert_eq!(rerun.processed, 0);
    sqlx::query("UPDATE book_classifications SET category='supplement' WHERE product_id='7' AND system='book_role'").execute(&db.pool).await.unwrap();
    assert!(
        stored::saved(&db.pool, "drivethru", "7")
            .await
            .unwrap()
            .is_none()
    );
    sqlx::query("UPDATE products SET description_text='Changed source' WHERE product_id='8'")
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(
        stored::saved(&db.pool, "drivethru", "8")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM osr_level_ranges")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        0
    );
    let stale = db.clone();
    db.release().await.unwrap();
    let successor = Catalogue::connect(&paths).await.unwrap();
    assert!(
        stale
            .osr_levels_commit(&mut rerun, &b, Some(&result("1")), None)
            .await
            .is_err()
    );
    successor.release().await.unwrap();
}

#[tokio::test]
async fn schema_fourteen_upgrade_preserves_data_and_keysets_use_scope_index() {
    let (_temp, paths, db) = seed().await;
    let id = db.catalogue;
    let canonical = capture(&db).await;
    let mut relevance = db.begin_relevance(false).await.unwrap();
    while !db.analysis_batch(&mut relevance).await.unwrap() {}
    let prior: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT product_id,system,category FROM book_classifications ORDER BY product_id,system",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    sqlx::raw_sql(include_str!("fixtures/schema_before_osr_levels.sql"))
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE catalogue SET version=14")
        .execute(&db.pool)
        .await
        .unwrap();
    db.release().await.unwrap();
    let db = Catalogue::connect(&paths).await.unwrap();
    assert_eq!(db.catalogue, id);
    assert_eq!(capture(&db).await, canonical);
    assert_eq!(
        sqlx::query_as::<_, (String, String, String)>(
            "SELECT product_id,system,category FROM book_classifications ORDER BY product_id,system"
        )
        .fetch_all(&db.pool)
        .await
        .unwrap(),
        prior
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM relevance_scores")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        8
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM osr_levels")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        0
    );
    let plans=sqlx::query("EXPLAIN QUERY PLAN SELECT document FROM osr_levels_eligible WHERE document>0 AND document<=100 ORDER BY document LIMIT 1").fetch_all(&db.pool).await.unwrap().into_iter().map(|r|r.get::<String,_>("detail")).collect::<Vec<_>>().join("\n");
    assert!(plans.contains("osr_scope_keyset"), "{plans}");
    assert!(!plans.contains("TEMP B-TREE"), "{plans}");
    db.release().await.unwrap();
}
