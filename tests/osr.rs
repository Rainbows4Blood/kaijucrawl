//! Synthetic OSR classification, metadata subset, live totals and migration contracts.
use kaijucrawl::{
    analysis::Status,
    decisions::{self, Decision},
    osr,
    paths::LocalPaths,
    storage::{overview, sqlite::Catalogue},
};
use serde_json::json;

fn decision(members: &[&str]) -> Decision {
    serde_json::from_value(json!({"model":decisions::MODEL,"answers":osr::SYSTEMS.iter().map(|(key,_,_)| {
        let choice=if members.contains(key){"supported"}else{"not_supported"};
        json!({"type":"choice","name":key,"choice":choice,"confidence":1.0,"probabilities":[{"value":"supported","probability":if choice=="supported"{1.0}else{0.0}},{"value":"not_supported","probability":if choice=="not_supported"{1.0}else{0.0}}]})
    }).collect::<Vec<_>>(),"usage":{"input_tokens":50,"output_tokens":0,"total_tokens":50,"input_tokens_details":{"cached_tokens":0,"cache_write_tokens":0}}})).unwrap()
}

#[test]
fn choices_cover_every_system_allow_overlap_and_keep_refusal_distinct_from_not_osr() {
    let request = osr::request(json!({"title":"Synthetic book","description":"Full description"}));
    request.validate().unwrap();
    assert_eq!(request.questions.len(), 15);
    let text = serde_json::to_string(&request.questions).unwrap();
    for phrase in [
        "OSRIC",
        "MÖRK BORG",
        "Vagabond",
        "fallible",
        "shared ancestry",
        "prior analysis",
        "untrusted",
        "OTHER OSR game",
        "Knave",
    ] {
        assert!(text.contains(phrase), "{phrase}");
    }
    for &(key, _, _) in &osr::SYSTEMS {
        assert_eq!(
            osr::answer(&decision(&[key])).unwrap().unwrap().members,
            [key]
        );
    }
    let selection = osr::answer(&decision(&["osric", "old_school_essentials", "other"]))
        .unwrap()
        .unwrap();
    assert_eq!(selection.category, "identified");
    assert_eq!(
        selection.members,
        ["osric", "old_school_essentials", "other"]
    );
    assert_eq!(selection.probabilities.as_array().unwrap().len(), 15);
    let negative = osr::answer(&decision(&[])).unwrap().unwrap();
    assert_eq!(
        (negative.category, negative.members),
        ("not_osr", vec!["not_osr"])
    );
    let mut raw = serde_json::to_value(decision(&[])).unwrap();
    raw["answers"][0] = json!({"type":"refusal","name":"osric"});
    assert!(
        osr::answer(&serde_json::from_value(raw.clone()).unwrap())
            .unwrap()
            .is_none()
    );
    raw["answers"][1]["name"] = json!("osric");
    assert!(osr::answer(&serde_json::from_value(raw).unwrap()).is_err());
    let mut malformed = decision(&[]);
    malformed.answers.pop();
    assert!(osr::answer(&malformed).is_err());
    let mut raw = serde_json::to_value(decision(&[])).unwrap();
    raw["answers"][0]["choice"] = json!("unexpected");
    assert!(osr::answer(&serde_json::from_value(raw).unwrap()).is_err());
}

async fn seed() -> (tempfile::TempDir, LocalPaths, Catalogue) {
    let temp = tempfile::tempdir().unwrap();
    let paths = LocalPaths::at(temp.path());
    let db = Catalogue::connect(&paths).await.unwrap();
    let intent = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO intents(id,target,product_id,payload,generation,finalized) VALUES(?,'drivethru','1','{}',?,1)").bind(intent).bind(db.generation).execute(&db.pool).await.unwrap();
    let rules = [
        json!(["Old-School Revival (OSR)", "OSRIC"]),
        json!([" OLD-SCHOOL REVIVAL (osr) ", "Old-School Revival (OSR)"]),
        json!(["OSRIC"]),
        json!(["Not Old-School Revival (OSR)"]),
        json!(["D&D"]),
        json!(null),
        json!(["Old-School Revival (OSR)", "Shadowrun"]),
    ];
    for (i, rules) in rules.into_iter().enumerate() {
        sqlx::query("INSERT INTO products(target,product_id,title,description_text,record,parser_version,record_schema_version,archive,fetched_at,last_checked_at,intent) VALUES('drivethru',?,'Synthetic OSR title','Full fictional description',?,'drivethru-rust-1',2,'{}','2026-01-01','2026-01-01',?)").bind((i+1).to_string()).bind(json!({"rule_systems":rules,"genres":["Old-School Revival (OSR)"],"rating":4.7,"rating_count":1000,"book_role":"DO_NOT_SEND","min_players":99,"osr":"DO_NOT_SEND","raw_metadata":"DO_NOT_SEND"}).to_string()).bind(intent).execute(&db.pool).await.unwrap();
    }
    (temp, paths, db)
}
async fn canonical(db: &Catalogue) -> Vec<(i64, String, String, String)> {
    sqlx::query_as("SELECT rowid,record,archive,fetched_at FROM products ORDER BY rowid")
        .fetch_all(&db.pool)
        .await
        .unwrap()
}
async fn live_matches(db: &Catalogue, status: &Status) {
    let mut actual = overview::read(&mut db.pool.acquire().await.unwrap())
        .await
        .unwrap();
    let mut expected = status.overview.clone().unwrap();
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
async fn members(db: &Catalogue, id: &str) -> Vec<String> {
    sqlx::query_scalar("SELECT group_key FROM book_group_memberships WHERE target='drivethru' AND product_id=? AND system='osr' ORDER BY group_key").bind(id).fetch_all(&db.pool).await.unwrap()
}

#[tokio::test]
async fn subset_multilabel_live_totals_default_reuse_replacements_and_rollback() {
    let (_temp, paths, db) = seed().await;
    let before = canonical(&db).await;
    let mut run = db.begin_osr(false).await.unwrap();
    assert_eq!((run.scope.as_str(), run.total), ("osr", 3));
    let a = db
        .classification_next_after(&run, 0)
        .await
        .unwrap()
        .unwrap();
    let b = db
        .classification_next_after(&run, a.document)
        .await
        .unwrap()
        .unwrap();
    let c = db
        .classification_next_after(&run, b.document)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        [
            a.product_id.as_str(),
            b.product_id.as_str(),
            c.product_id.as_str()
        ],
        ["1", "2", "7"]
    );
    assert!(
        db.classification_next_after(&run, c.document)
            .await
            .unwrap()
            .is_none()
    );
    for book in [&a, &b, &c] {
        assert!(!book.evidence.as_ref().unwrap().contains("DO_NOT_SEND"));
        db.classification_dispatch(&mut run, book).await.unwrap();
    }
    for (book, selected) in [
        (&c, vec!["other"]),
        (&a, vec!["osric", "old_school_essentials"]),
        (&b, vec![]),
    ] {
        db.classification_commit(&mut run, book, Some(&decision(&selected)), None)
            .await
            .unwrap();
        live_matches(&db, &run).await;
    }
    let counts = &run.overview.as_ref().unwrap().analyses[osr::ALGORITHM];
    assert_eq!(
        (
            counts.eligible,
            counts.completed,
            counts.counts["identified"],
            counts.counts["not_osr"]
        ),
        (3, 3, 2, 1)
    );
    assert_eq!(members(&db, "1").await, ["old_school_essentials", "osric"]);
    assert_eq!(members(&db, "2").await, ["not_osr"]);
    db.finish_analysis(&mut run, "completed", "").await.unwrap();
    let mut skipped = db.begin_osr(false).await.unwrap();
    assert_eq!(skipped.total, 0);
    db.finish_analysis(&mut skipped, "completed", "")
        .await
        .unwrap();
    let mut rerun = db.begin_osr(true).await.unwrap();
    let book = db.classification_next(&rerun).await.unwrap().unwrap();
    sqlx::raw_sql("CREATE TRIGGER synthetic_osr_failure BEFORE INSERT ON book_group_memberships WHEN new.system='osr' BEGIN SELECT RAISE(ABORT,'Synthetic rollback'); END;").execute(&db.pool).await.unwrap();
    let saved = serde_json::to_value(&rerun.overview).unwrap();
    assert!(
        db.classification_commit(&mut rerun, &book, Some(&decision(&[])), None)
            .await
            .is_err()
    );
    assert_eq!(serde_json::to_value(&rerun.overview).unwrap(), saved);
    assert_eq!(members(&db, "1").await.len(), 2);
    sqlx::raw_sql("DROP TRIGGER synthetic_osr_failure")
        .execute(&db.pool)
        .await
        .unwrap();
    db.classification_commit(&mut rerun, &book, Some(&decision(&[])), None)
        .await
        .unwrap();
    live_matches(&db, &rerun).await;
    assert_eq!(members(&db, "1").await, ["not_osr"]);
    let last = db
        .classification_next_after(&rerun, b.document)
        .await
        .unwrap()
        .unwrap();
    db.classification_commit(&mut rerun, &last, None, Some("Synthetic refusal"))
        .await
        .unwrap();
    assert_eq!(members(&db, "7").await, ["other"]);
    live_matches(&db, &rerun).await;
    db.finish_analysis(&mut rerun, "cancelled", "")
        .await
        .unwrap();
    assert_eq!(canonical(&db).await, before);
    db.release().await.unwrap();
    db.pool.close().await;
    let next = Catalogue::connect(&paths).await.unwrap();
    assert_eq!(next.begin_osr(false).await.unwrap().total, 0);
    next.release().await.unwrap();
}

#[tokio::test]
async fn source_changes_scope_removal_and_stale_writers_preserve_other_results() {
    let (_temp, paths, db) = seed().await;
    let mut relevance = db.begin_analysis().await.unwrap();
    while !db.analysis_batch(&mut relevance).await.unwrap() {}
    let mut run = db.begin_osr(false).await.unwrap();
    while let Some(book) = db.classification_next(&run).await.unwrap() {
        db.classification_commit(&mut run, &book, Some(&decision(&["osric"])), None)
            .await
            .unwrap();
    }
    db.finish_analysis(&mut run, "completed", "").await.unwrap();
    sqlx::query("UPDATE products SET record=json_set(record,'$.rating',4.2) WHERE product_id='1'")
        .execute(&db.pool)
        .await
        .unwrap();
    assert_eq!(members(&db, "1").await, ["osric"]);
    sqlx::query(
        "UPDATE products SET description_text='A changed full description' WHERE product_id='1'",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    assert!(members(&db, "1").await.is_empty());
    sqlx::query("UPDATE products SET record=json_set(record,'$.rule_systems',json_array('OSRIC')) WHERE product_id='2'").execute(&db.pool).await.unwrap();
    assert!(members(&db, "2").await.is_empty());
    let mut pending = db.begin_osr(false).await.unwrap();
    assert_eq!(pending.total, 1);
    let book = db.classification_next(&pending).await.unwrap().unwrap();
    sqlx::query("UPDATE products SET record=json_set(record,'$.rule_systems',json_array('D&D')) WHERE product_id='1'").execute(&db.pool).await.unwrap();
    assert!(
        db.classification_commit(&mut pending, &book, Some(&decision(&[])), None)
            .await
            .is_err()
    );
    db.finish_analysis(&mut pending, "cancelled", "")
        .await
        .unwrap();
    sqlx::query("UPDATE products SET record=json_set(record,'$.rule_systems',json_array('Old-School Revival (OSR)')) WHERE product_id='3'").execute(&db.pool).await.unwrap();
    let mut pending = db.begin_osr(false).await.unwrap();
    assert_eq!(pending.total, 1);
    let book = db.classification_next(&pending).await.unwrap().unwrap();
    assert_eq!(book.product_id, "3");
    db.classification_dispatch(&mut pending, &book)
        .await
        .unwrap();
    db.release().await.unwrap();
    let next = Catalogue::connect(&paths).await.unwrap();
    assert!(
        db.classification_commit(&mut pending, &book, Some(&decision(&[])), None)
            .await
            .is_err()
    );
    sqlx::query("DELETE FROM products WHERE product_id='7'")
        .execute(&next.pool)
        .await
        .unwrap();
    let totals = overview::read(&mut next.pool.acquire().await.unwrap())
        .await
        .unwrap();
    assert_eq!(
        (
            totals.analyses[osr::ALGORITHM].eligible,
            totals.analyses[osr::ALGORITHM].completed
        ),
        (1, 0)
    );
    assert_eq!(
        totals.analyses[kaijucrawl::analysis::ALGORITHM].completed,
        5
    );
    next.release().await.unwrap();
}

#[tokio::test]
async fn schema_thirteen_upgrade_builds_only_osr_index_and_preserves_identity_history_and_captures()
{
    let (_temp, paths, db) = seed().await;
    let before = canonical(&db).await;
    let identity = db.catalogue;
    let mut relevance = db.begin_analysis().await.unwrap();
    while !db.analysis_batch(&mut relevance).await.unwrap() {}
    let history: Vec<(i64, String, String)> =
        sqlx::query_as("SELECT id,algorithm,state FROM analysis_runs ORDER BY id")
            .fetch_all(&db.pool)
            .await
            .unwrap();
    sqlx::raw_sql(include_str!("fixtures/schema_before_osr.sql"))
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE catalogue SET version=13")
        .execute(&db.pool)
        .await
        .unwrap();
    db.release().await.unwrap();
    db.pool.close().await;
    let next = Catalogue::connect(&paths).await.unwrap();
    assert_eq!(next.catalogue, identity);
    assert_eq!(canonical(&next).await, before);
    let after: Vec<(i64, String, String)> =
        sqlx::query_as("SELECT id,algorithm,state FROM analysis_runs ORDER BY id")
            .fetch_all(&next.pool)
            .await
            .unwrap();
    assert_eq!(history, after);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT version FROM catalogue")
            .fetch_one(&next.pool)
            .await
            .unwrap(),
        i64::from(kaijucrawl::domain::CATALOGUE_VERSION)
    );
    let totals = overview::read(&mut next.pool.acquire().await.unwrap())
        .await
        .unwrap();
    assert_eq!(
        (
            totals.analyses[osr::ALGORITHM].eligible,
            totals.analyses[osr::ALGORITHM].completed
        ),
        (3, 0)
    );
    assert_eq!(
        totals.analyses[kaijucrawl::analysis::ALGORITHM].completed,
        7
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM analysis_groups WHERE system='osr'")
            .fetch_one(&next.pool)
            .await
            .unwrap(),
        18
    );
    let plan: Vec<(i64,i64,i64,String)> = sqlx::query_as("EXPLAIN QUERY PLAN SELECT s.rowid FROM osr_scope e JOIN products s ON s.target=e.target AND s.product_id=e.product_id WHERE e.document>0 AND e.document<=999 ORDER BY e.document LIMIT 1").fetch_all(&next.pool).await.unwrap();
    assert!(plan.iter().any(|p| p.3.contains("osr_scope_keyset")));
    next.release().await.unwrap();
}
