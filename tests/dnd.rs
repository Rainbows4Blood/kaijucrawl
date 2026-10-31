//! Synthetic D&D decisions, independent memberships, eligibility and lifetime totals.
use kaijucrawl::{
    analysis::Status,
    decisions::{self, Decision},
    dnd,
    paths::LocalPaths,
    storage::{classification as stored, overview, sqlite::Catalogue},
};
use serde_json::{Value, json};
fn choice(name: &str, value: &str, options: &[&str]) -> Value {
    json!({"type":"choice","name":name,"choice":value,"confidence":1.0,"probabilities":options.iter().map(|v|json!({"value":v,"probability":if *v==value{1.0}else{0.0}})).collect::<Vec<_>>()})
}
fn decision(answers: Vec<Value>) -> Decision {
    serde_json::from_value(json!({"model":decisions::MODEL,"answers":answers,"usage":{"input_tokens":600,"output_tokens":0,"total_tokens":600,"input_tokens_details":{"cached_tokens":0,"cache_write_tokens":0}}})).unwrap()
}
fn compatibility(value: &str) -> Decision {
    decision(vec![choice(
        "dnd",
        value,
        &["dnd_only", "dnd_and_others", "no"],
    )])
}
fn editions(values: &[&str]) -> Decision {
    decision(
        dnd::EDITIONS
            .iter()
            .map(|(key, _)| {
                choice(
                    key,
                    if values.contains(key) {
                        "supported"
                    } else {
                        "uncertain"
                    },
                    &["supported", "not_supported", "uncertain"],
                )
            })
            .collect(),
    )
}
async fn seeded(n: i64) -> (tempfile::TempDir, LocalPaths, Catalogue) {
    let temp = tempfile::tempdir().unwrap();
    let paths = LocalPaths::at(temp.path());
    let db = Catalogue::connect(&paths).await.unwrap();
    let intent = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO intents(id,target,product_id,payload,generation,finalized) VALUES(?,'drivethru','1','{}',?,1)").bind(intent).bind(db.generation).execute(&db.pool).await.unwrap();
    sqlx::query("WITH RECURSIVE n(i) AS(VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<?) INSERT INTO products(target,product_id,title,description_text,record,parser_version,record_schema_version,archive,fetched_at,last_checked_at,intent) SELECT 'drivethru',CAST(i AS TEXT),'Synthetic book '||i,'Full fictional D&D description',json_object('rating',4.7,'rating_count',1000,'rule_systems',json_array('Messy source taxonomy'),'book_role','DO_NOT_SEND','min_players',99,'raw_metadata','DO_NOT_SEND'),'drivethru-rust-1',2,'{}','2026-01-01','2026-01-01',? FROM n").bind(n).bind(intent).execute(&db.pool).await.unwrap();
    (temp, paths, db)
}
async fn totals(db: &Catalogue) -> kaijucrawl::analysis::Overview {
    let mut tx = db.pool.begin().await.unwrap();
    overview::read(&mut tx).await.unwrap()
}
async fn save(db: &Catalogue, s: &mut Status, answer: &Decision) -> stored::Candidate {
    let b = db.classification_next(s).await.unwrap().unwrap();
    db.classification_dispatch(s, &b).await.unwrap();
    db.classification_commit(s, &b, Some(answer), None)
        .await
        .unwrap();
    b
}
async fn canonical(db: &Catalogue) -> Vec<(String, String, String, String)> {
    sqlx::query_as("SELECT product_id,record,fetched_at,archive FROM products ORDER BY product_id")
        .fetch_all(&db.pool)
        .await
        .unwrap()
}

#[test]
fn editions_allow_multiple_or_unknown_but_not_partial_refusals_or_invalid_labels() {
    dnd::request(json!({"title":"Synthetic","description":"Full description"}))
        .validate()
        .unwrap();
    let r = dnd::editions_request(json!({"title":"Synthetic","description":"Full description"}));
    r.validate().unwrap();
    assert_eq!(r.questions.len(), 9);
    let a = dnd::editions_answer(&editions(&[
        "0e", "bx", "adnd_1e", "adnd_2e", "3e", "3_5e", "4e", "5e_2014", "5e_2024",
    ]))
    .unwrap()
    .unwrap();
    assert_eq!(a.members.len(), 9);
    assert_eq!(a.category, "identified");
    assert_eq!(
        dnd::editions_answer(&editions(&[]))
            .unwrap()
            .unwrap()
            .members,
        vec!["unknown"]
    );
    let mut raw = serde_json::to_value(editions(&["3_5e"])).unwrap();
    raw["answers"][4] = json!({"type":"refusal","name":"3e"});
    assert!(
        dnd::editions_answer(&serde_json::from_value(raw.clone()).unwrap())
            .unwrap()
            .is_none()
    );
    raw["answers"][0]["name"] = json!("unsupported_edition");
    assert!(dnd::editions_answer(&serde_json::from_value(raw).unwrap()).is_err());
}
#[tokio::test]
async fn independent_compatibility_and_multi_editions_have_whole_catalogue_totals() {
    let (_temp, _paths, db) = seeded(108).await;
    let before = canonical(&db).await;
    let initial = totals(&db).await;
    assert_eq!(initial.analyses[dnd::EDITIONS_ALGORITHM].eligible, 0);
    assert_eq!(
        initial.analyses[dnd::EDITIONS_ALGORITHM].prerequisite_pending,
        108
    );
    let mut s = db.begin_dnd(false, false).await.unwrap();
    assert_eq!(s.total, 108);
    for _ in 0..35 {
        let b = db.classification_next(&s).await.unwrap().unwrap();
        save(
            &db,
            &mut s,
            &compatibility(match b.document % 3 {
                0 => "no",
                1 => "dnd_only",
                _ => "dnd_and_others",
            }),
        )
        .await;
    }
    db.finish_analysis(&mut s, "cancelled", "").await.unwrap();
    assert_eq!(totals(&db).await.analyses[dnd::ALGORITHM].completed, 35);
    let mut s = db.begin_dnd(false, false).await.unwrap();
    assert_eq!(s.total, 73);
    while let Some(b) = db.classification_next(&s).await.unwrap() {
        assert!(!b.evidence.as_ref().unwrap().contains("DO_NOT_SEND"));
        save(
            &db,
            &mut s,
            &compatibility(match b.document % 3 {
                0 => "no",
                1 => "dnd_only",
                _ => "dnd_and_others",
            }),
        )
        .await;
    }
    db.finish_analysis(&mut s, "completed", "").await.unwrap();
    let mut s = db.begin_dnd(true, false).await.unwrap();
    assert_eq!((s.total, s.scope.as_str()), (72, "dnd"));
    while let Some(b) = db.classification_next(&s).await.unwrap() {
        assert_ne!(b.document % 3, 0);
        let evidence: Value = serde_json::from_str(b.evidence.as_ref().unwrap()).unwrap();
        for key in [
            "book_role",
            "min_players",
            "dnd",
            "solo_play",
            "relevance_score",
        ] {
            assert!(evidence.get(key).is_none());
        }
        save(
            &db,
            &mut s,
            &editions(if b.document % 2 == 0 {
                &[]
            } else {
                &["0e", "bx", "5e_2024"]
            }),
        )
        .await;
    }
    db.finish_analysis(&mut s, "completed", "").await.unwrap();
    let t = totals(&db).await;
    assert_eq!(t.catalogue_total, 108);
    assert_eq!(t.analyses[dnd::ALGORITHM].completed, 108);
    assert_eq!(t.analyses[dnd::ALGORITHM].counts["no"], 36);
    let e = &t.analyses[dnd::EDITIONS_ALGORITHM];
    assert_eq!(
        (e.completed, e.eligible, e.prerequisite_pending),
        (72, 72, 0)
    );
    assert_eq!(
        (e.counts["bx"], e.counts["5e_2024"], e.counts["unknown"]),
        (36, 36, 36)
    );
    let mut reused = db.begin_dnd(true, false).await.unwrap();
    assert_eq!(reused.total, 0);
    db.finish_analysis(&mut reused, "completed", "")
        .await
        .unwrap();
    assert_eq!(canonical(&db).await, before);
    assert_eq!(
        stored::saved_dnd(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .unwrap()
            .category,
        dnd::Category::DndOnly
    );
    assert_eq!(
        stored::saved_editions(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .unwrap()
            .editions,
        vec!["0e", "bx", "5e_2024"]
    );
    let mut re = db.begin_dnd(true, true).await.unwrap();
    save(&db, &mut re, &editions(&["3e", "3_5e"])).await;
    assert_eq!(
        stored::saved_editions(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .unwrap()
            .editions,
        vec!["3e", "3_5e"]
    );
    db.finish_analysis(&mut re, "cancelled", "").await.unwrap();
    db.release().await.unwrap();
}
#[tokio::test]
async fn schema_upgrade_and_canonical_or_eligibility_changes_preserve_the_right_results() {
    let (_temp, paths, db) = seeded(4).await;
    let id = db.catalogue;
    let before = canonical(&db).await;
    let mut weighted = db.begin_analysis().await.unwrap();
    db.analysis_batch(&mut weighted).await.unwrap();
    sqlx::raw_sql(include_str!("fixtures/schema_before_dnd.sql"))
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE catalogue SET version=8")
        .execute(&db.pool)
        .await
        .unwrap();
    db.release().await.unwrap();
    let db = Catalogue::connect(&paths).await.unwrap();
    assert_eq!(db.catalogue, id);
    assert_eq!(canonical(&db).await, before);
    assert_eq!(
        totals(&db).await.analyses[kaijucrawl::analysis::ALGORITHM].completed,
        4
    );
    let mut s = db.begin_dnd(false, false).await.unwrap();
    for _ in 0..4 {
        save(&db, &mut s, &compatibility("dnd_only")).await;
    }
    db.finish_analysis(&mut s, "completed", "").await.unwrap();
    let mut s = db.begin_dnd(true, false).await.unwrap();
    for _ in 0..4 {
        save(&db, &mut s, &editions(&["3e", "3_5e"])).await;
    }
    db.finish_analysis(&mut s, "completed", "").await.unwrap();
    sqlx::query("UPDATE book_classifications SET category='dnd_and_others' WHERE system='dnd' AND product_id='1'").execute(&db.pool).await.unwrap();
    assert!(
        stored::saved_editions(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .is_some()
    );
    sqlx::query("UPDATE products SET record=json_set(record,'$.price','5.00','$.min_players',4) WHERE product_id='1'").execute(&db.pool).await.unwrap();
    assert!(
        stored::saved_editions(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .is_some()
    );
    sqlx::query(
        "UPDATE book_classifications SET category='no' WHERE system='dnd' AND product_id='1'",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    assert!(
        stored::saved_editions(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .is_none()
    );
    sqlx::query("DELETE FROM book_classifications WHERE system='dnd' AND product_id='2'")
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(
        stored::saved_editions(&db.pool, "drivethru", "2")
            .await
            .unwrap()
            .is_none()
    );
    sqlx::query("UPDATE products SET description_text='Changed evidence' WHERE product_id='3'")
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(
        stored::saved_dnd(&db.pool, "drivethru", "3")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        stored::saved_editions(&db.pool, "drivethru", "3")
            .await
            .unwrap()
            .is_none()
    );
    sqlx::query("DELETE FROM products WHERE product_id='4'")
        .execute(&db.pool)
        .await
        .unwrap();
    let t = totals(&db).await;
    assert_eq!(
        (
            t.catalogue_total,
            t.analyses[dnd::ALGORITHM].completed,
            t.analyses[dnd::EDITIONS_ALGORITHM].eligible
        ),
        (3, 1, 0)
    );
    db.release().await.unwrap();
}
#[tokio::test]
async fn edition_refusals_and_storage_failures_are_atomic_and_unfinished_gaps_survive_takeover() {
    let (_temp, paths, db) = seeded(3).await;
    let mut s = db.begin_dnd(false, false).await.unwrap();
    for _ in 0..3 {
        save(&db, &mut s, &compatibility("dnd_only")).await;
    }
    db.finish_analysis(&mut s, "completed", "").await.unwrap();
    let mut s = db.begin_dnd(true, false).await.unwrap();
    let first = db.classification_next_after(&s, 0).await.unwrap().unwrap();
    let second = db
        .classification_next_after(&s, first.document)
        .await
        .unwrap()
        .unwrap();
    db.classification_dispatch(&mut s, &first).await.unwrap();
    db.classification_dispatch(&mut s, &second).await.unwrap();
    db.classification_commit(&mut s, &second, Some(&editions(&["bx"])), None)
        .await
        .unwrap();
    db.classification_retry_wait(&s, &first, "Network connection failed")
        .await
        .unwrap();
    db.release().await.unwrap();
    let next = Catalogue::connect(&paths).await.unwrap();
    assert!(db.classification_redispatch(&mut s, &first).await.is_err());
    let mut s = next.begin_dnd(true, false).await.unwrap();
    assert_eq!(s.total, 2);
    assert_eq!(
        next.classification_next(&s)
            .await
            .unwrap()
            .unwrap()
            .document,
        first.document
    );
    next.classification_dispatch(&mut s, &first).await.unwrap();
    sqlx::raw_sql("CREATE TRIGGER fail_editions BEFORE INSERT ON book_group_memberships WHEN new.system='dnd_editions' AND new.group_key='3e' BEGIN SELECT RAISE(ABORT,'synthetic failure'); END").execute(&next.pool).await.unwrap();
    assert!(
        next.classification_commit(&mut s, &first, Some(&editions(&["bx", "3e"])), None)
            .await
            .is_err()
    );
    assert_eq!(s.processed, 0);
    assert!(
        stored::saved_editions(&next.pool, "drivethru", "1")
            .await
            .unwrap()
            .is_none()
    );
    sqlx::raw_sql("DROP TRIGGER fail_editions")
        .execute(&next.pool)
        .await
        .unwrap();
    next.finish_analysis(&mut s, "cancelled", "").await.unwrap();
    let mut re = next.begin_dnd(true, true).await.unwrap();
    let b = next
        .classification_next_after(&re, first.document)
        .await
        .unwrap()
        .unwrap();
    let mut partial = serde_json::to_value(editions(&["3e"])).unwrap();
    partial["answers"][0] = json!({"type":"refusal","name":"0e"});
    next.classification_dispatch(&mut re, &b).await.unwrap();
    next.classification_commit(
        &mut re,
        &b,
        Some(&serde_json::from_value(partial).unwrap()),
        None,
    )
    .await
    .unwrap();
    assert_eq!((re.scored, re.skipped), (0, 1));
    assert_eq!(
        stored::saved_editions(&next.pool, "drivethru", "2")
            .await
            .unwrap()
            .unwrap()
            .editions,
        vec!["bx"]
    );
    next.finish_analysis(&mut re, "cancelled", "")
        .await
        .unwrap();
    next.release().await.unwrap();
}
