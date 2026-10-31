//! Synthetic edition classifications, subset ownership, migrations and live totals.
use kaijucrawl::{
    analysis::Status,
    decisions::{self, Decision},
    earth_editions::Kind,
    paths::LocalPaths,
    storage::{classification as stored, overview, sqlite::Catalogue},
};
use serde_json::json;
use sqlx::Row;
fn decision(kind: Kind, supported: &[&str]) -> Decision {
    serde_json::from_value(json!({"model":decisions::MODEL,"answers":kind.editions().iter().map(|(key,_)| {
 let selected=if supported.contains(key){"supported"}else{"uncertain"};
 json!({"type":"choice","name":key,"choice":selected,"confidence":1.0,"probabilities":(["supported","not_supported","uncertain"].into_iter().map(|v|json!({"value":v,"probability":if v==selected{1.0}else{0.0}})).collect::<Vec<_>>())})
 }).collect::<Vec<_>>(),"usage":{"input_tokens":600,"output_tokens":0,"total_tokens":600,"input_tokens_details":{"cached_tokens":0,"cache_write_tokens":0}}})).unwrap()
}
async fn seed() -> (tempfile::TempDir, LocalPaths, Catalogue) {
    let temp = tempfile::tempdir().unwrap();
    let paths = LocalPaths::at(temp.path());
    let db = Catalogue::connect(&paths).await.unwrap();
    let intent = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO intents(id,target,product_id,payload,generation,finalized) VALUES(?,'drivethru','1','{}',?,1)").bind(intent).bind(db.generation).execute(&db.pool).await.unwrap();
    let rules = [
        json!(["Shadowrun"]),
        json!(["sHaDoWrUn, 6th Ed."]),
        json!(["Shadowrun: Anarchy", " Call of Cthulhu "]),
        json!(["CALL OF CTHULHU"]),
        json!(["Call of Cthulhu", "Cthulhu Eternal"]),
        json!(["Cthulhu Eternal"]),
        json!(["CoC"]),
        json!(["D&D"]),
        json!(null),
    ];
    for (i, rules) in rules.into_iter().enumerate() {
        sqlx::query("INSERT INTO products(target,product_id,title,description_text,record,parser_version,record_schema_version,archive,fetched_at,last_checked_at,intent) VALUES('drivethru',?,'Synthetic book','Full fictional description, copyright 2026',?,'drivethru-rust-1',2,'{}','2026-01-01','2026-01-01',?)").bind((i+1).to_string()).bind(json!({"rule_systems":rules,"rating":4.7,"rating_count":1000,"in_universe_date":"DO_NOT_SEND","dnd_settings":"DO_NOT_SEND","raw_metadata":"DO_NOT_SEND"}).to_string()).bind(intent).execute(&db.pool).await.unwrap();
    }
    (temp, paths, db)
}
async fn canonical(db: &Catalogue) -> Vec<(i64, String, String, String)> {
    sqlx::query_as("SELECT rowid,record,archive,fetched_at FROM products ORDER BY rowid")
        .fetch_all(&db.pool)
        .await
        .unwrap()
}
async fn check(db: &Catalogue, s: &Status) {
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

async fn fill(db: &Catalogue, kind: Kind, again: bool) -> Status {
    let mut status = db.begin_earth_editions(kind, again).await.unwrap();
    assert_eq!(status.total, 3);
    while let Some(book) = db.classification_next(&status).await.unwrap() {
        assert!(!book.evidence.as_ref().unwrap().contains("DO_NOT_SEND"));
        db.classification_dispatch(&mut status, &book)
            .await
            .unwrap();
        let supported = if book.product_id == "3" {
            &[][..]
        } else if kind == Kind::Coc {
            &["6e", "7e"][..]
        } else {
            &["6e", "anarchy_2"][..]
        };
        db.classification_commit(&mut status, &book, Some(&decision(kind, supported)), None)
            .await
            .unwrap();
        check(db, &status).await;
    }
    db.finish_analysis(&mut status, "completed", "")
        .await
        .unwrap();
    status
}
#[test]
fn independent_edition_questions_accept_multiple_unknown_and_atomic_refusal() {
    for kind in Kind::ALL {
        let request = kind.request(json!({"title":"Synthetic","description":"Full saved source"}));
        request.validate().unwrap();
        assert_eq!(
            request.questions.len(),
            if kind == Kind::Coc { 7 } else { 8 }
        );
        let selected = kind
            .answer(&decision(kind, &["1e", "6e"]))
            .unwrap()
            .unwrap();
        assert_eq!(selected.members, vec!["1e", "6e"]);
        assert_eq!(selected.category, "identified");
        assert_eq!(
            kind.answer(&decision(kind, &[])).unwrap().unwrap().members,
            vec!["unknown"]
        );
        let mut response = decision(kind, &["1e"]);
        response.answers[1] = decisions::Answer::Refusal { name: "2e".into() };
        assert!(kind.answer(&response).unwrap().is_none());
        response.answers.pop();
        assert!(kind.answer(&response).is_err());
        let mut response = decision(kind, &[]);
        response.answers.swap(0, 1);
        assert!(kind.answer(&response).is_err());
    }
    let r = Kind::Shadowrun
        .answer(&decision(Kind::Shadowrun, &["anarchy_2"]))
        .unwrap()
        .unwrap();
    assert_eq!(r.members, vec!["anarchy_2"]);
}
#[tokio::test]
async fn both_subsets_multiple_memberships_unknown_reuse_and_live_totals() {
    let (_temp, _paths, db) = seed().await;
    let before = canonical(&db).await;
    for kind in Kind::ALL {
        let status = fill(&db, kind, false).await;
        assert_eq!(status.scope, kind.scope());
        let totals = &status.overview.as_ref().unwrap().analyses[kind.algorithm()];
        assert_eq!(
            (
                totals.eligible,
                totals.completed,
                totals.prerequisite_pending
            ),
            (3, 3, 0)
        );
        assert_eq!(totals.counts["unknown"], 1);
        assert_eq!(totals.counts["identified"], 2);
        assert_eq!(totals.counts["6e"], 2);
        assert!(
            stored::saved_earth_editions(&db.pool, "drivethru", "3", kind)
                .await
                .unwrap()
                .unwrap()
                .editions
                .is_empty()
        );
        let mut next = db.begin_earth_editions(kind, false).await.unwrap();
        assert_eq!(next.total, 0);
        db.finish_analysis(&mut next, "completed", "")
            .await
            .unwrap();
        fill(&db, kind, true).await;
    }
    assert_eq!(canonical(&db).await, before);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM book_classifications WHERE system='dnd'")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(count, 0);
    let plan=sqlx::query("EXPLAIN QUERY PLAN SELECT s.rowid FROM earth_scope e JOIN products s ON s.target=e.target AND s.product_id=e.product_id JOIN classification_source c ON c.target=s.target AND c.product_id=s.product_id WHERE e.system='coc' AND e.document>0 AND e.document<100 ORDER BY e.document LIMIT 1").fetch_all(&db.pool).await.unwrap();
    assert!(
        plan.iter()
            .any(|r| r.get::<String, _>("detail").contains("earth_scope_keyset"))
    );
    assert!(
        !plan
            .iter()
            .any(|r| r.get::<String, _>("detail").contains("SCAN p"))
    );
    db.release().await.unwrap();
}
#[tokio::test]
async fn source_scope_and_ratings_have_correct_invalidation_boundaries() {
    let (_temp, _paths, db) = seed().await;
    for kind in Kind::ALL {
        fill(&db, kind, false).await;
    }
    sqlx::query("UPDATE products SET record=json_set(record,'$.rating',2.0) WHERE product_id='3'")
        .execute(&db.pool)
        .await
        .unwrap();
    for kind in Kind::ALL {
        assert!(
            stored::saved_earth_editions(&db.pool, "drivethru", "3", kind)
                .await
                .unwrap()
                .is_some()
        );
    }
    // Editing a prior analysis does not invalidate either edition result.
    let mut prior = db
        .begin_earth(kaijucrawl::earth_context::Kind::ShadowrunLocation, false)
        .await
        .unwrap();
    let book = db.earth_next_after(&prior, 0).await.unwrap().unwrap();
    let response = kaijucrawl::earth_context::Response {
        model: "gpt-6-luna".into(),
        data: Some(kaijucrawl::earth_context::Data::Location {
            location: Some("Seattle".into()),
        }),
        usage: decision(Kind::Coc, &[]).usage,
    };
    db.earth_commit(&mut prior, &book, Some(&response), None)
        .await
        .unwrap();
    check(&db, &prior).await;
    db.finish_analysis(&mut prior, "completed", "")
        .await
        .unwrap();
    assert!(
        stored::saved_earth_editions(&db.pool, "drivethru", "1", Kind::Shadowrun)
            .await
            .unwrap()
            .is_some()
    );
    let mut rerun = db
        .begin_earth_editions(Kind::Shadowrun, true)
        .await
        .unwrap();
    let b = db.classification_next(&rerun).await.unwrap().unwrap();
    db.classification_commit(
        &mut rerun,
        &b,
        Some(&decision(Kind::Shadowrun, &["1e"])),
        None,
    )
    .await
    .unwrap();
    check(&db, &rerun).await;
    assert!(
        kaijucrawl::storage::earth_context::saved(
            &db.pool,
            "drivethru",
            "1",
            kaijucrawl::earth_context::Kind::ShadowrunLocation
        )
        .await
        .unwrap()
        .is_some()
    );
    db.finish_analysis(&mut rerun, "cancelled", "")
        .await
        .unwrap();
    sqlx::query(
        "UPDATE products SET description_text='Changed source evidence' WHERE product_id='1'",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    assert!(
        stored::saved_earth_editions(&db.pool, "drivethru", "1", Kind::Shadowrun)
            .await
            .unwrap()
            .is_none()
    );
    sqlx::query("UPDATE products SET record=json_set(record,'$.rule_systems',json_array('Cthulhu Eternal')) WHERE product_id='3'").execute(&db.pool).await.unwrap();
    for kind in Kind::ALL {
        assert!(
            stored::saved_earth_editions(&db.pool, "drivethru", "3", kind)
                .await
                .unwrap()
                .is_none()
        );
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM book_group_memberships WHERE product_id='3'"
        )
        .fetch_one(&db.pool)
        .await
        .unwrap(),
        0
    );
    db.release().await.unwrap();
}
#[tokio::test]
async fn rollback_refusal_out_of_order_and_stale_owner_preserve_results() {
    let (_temp, paths, db) = seed().await;
    fill(&db, Kind::Coc, false).await;
    let mut status = db.begin_earth_editions(Kind::Coc, true).await.unwrap();
    let a = db
        .classification_next_after(&status, 0)
        .await
        .unwrap()
        .unwrap();
    let b = db
        .classification_next_after(&status, a.document)
        .await
        .unwrap()
        .unwrap();
    db.classification_dispatch(&mut status, &a).await.unwrap();
    db.classification_dispatch(&mut status, &b).await.unwrap();
    db.classification_commit(&mut status, &b, Some(&decision(Kind::Coc, &["1e"])), None)
        .await
        .unwrap();
    check(&db, &status).await;
    let mut refused = decision(Kind::Coc, &[]);
    refused.answers[0] = decisions::Answer::Refusal { name: "1e".into() };
    db.classification_commit(&mut status, &a, Some(&refused), None)
        .await
        .unwrap();
    check(&db, &status).await;
    assert!(
        stored::saved_earth_editions(&db.pool, "drivethru", &a.product_id, Kind::Coc)
            .await
            .unwrap()
            .unwrap()
            .editions
            .is_empty()
    );
    let c = db
        .classification_next_after(&status, b.document)
        .await
        .unwrap()
        .unwrap();
    sqlx::raw_sql("CREATE TRIGGER fail_editions BEFORE INSERT ON book_group_memberships BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").execute(&db.pool).await.unwrap();
    let processed = status.processed;
    assert!(
        db.classification_commit(&mut status, &c, Some(&decision(Kind::Coc, &["1e"])), None)
            .await
            .is_err()
    );
    assert_eq!(status.processed, processed);
    check(&db, &status).await;
    sqlx::raw_sql("DROP TRIGGER fail_editions;")
        .execute(&db.pool)
        .await
        .unwrap();
    db.release().await.unwrap();
    let next = Catalogue::connect(&paths).await.unwrap();
    assert!(
        db.classification_commit(&mut status, &c, Some(&decision(Kind::Coc, &["1e"])), None)
            .await
            .is_err()
    );
    let mut resumed = next
        .begin_earth_editions(Kind::Shadowrun, false)
        .await
        .unwrap();
    assert_eq!(resumed.total, 3);
    next.finish_analysis(&mut resumed, "cancelled", "")
        .await
        .unwrap();
    next.release().await.unwrap();
}
#[tokio::test]
async fn schema_twelve_migration_preserves_scope_identity_context_and_history() {
    let (_temp, paths, db) = seed().await;
    let before = canonical(&db).await;
    let identity = db.catalogue;
    let mut old = db
        .begin_earth(kaijucrawl::earth_context::Kind::CocLocation, false)
        .await
        .unwrap();
    let book = db.earth_next_after(&old, 0).await.unwrap().unwrap();
    let response = kaijucrawl::earth_context::Response {
        model: "gpt-6-luna".into(),
        data: Some(kaijucrawl::earth_context::Data::Location {
            location: Some("Arkham".into()),
        }),
        usage: decision(Kind::Coc, &[]).usage,
    };
    db.earth_commit(&mut old, &book, Some(&response), None)
        .await
        .unwrap();
    db.finish_analysis(&mut old, "completed", "").await.unwrap();
    sqlx::raw_sql(include_str!("fixtures/schema_before_earth_editions.sql"))
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE catalogue SET version=12")
        .execute(&db.pool)
        .await
        .unwrap();
    db.release().await.unwrap();
    db.pool.close().await;
    let db = Catalogue::connect(&paths).await.unwrap();
    assert_eq!(db.catalogue, identity);
    assert_eq!(canonical(&db).await, before);
    assert!(
        kaijucrawl::storage::earth_context::saved(
            &db.pool,
            "drivethru",
            &book.product_id,
            kaijucrawl::earth_context::Kind::CocLocation
        )
        .await
        .unwrap()
        .is_some()
    );
    let overview = overview::read(&mut db.pool.acquire().await.unwrap())
        .await
        .unwrap();
    for kind in Kind::ALL {
        let c = &overview.analyses[kind.algorithm()];
        assert_eq!((c.eligible, c.completed), (3, 0));
    }
    assert_eq!(overview.analyses.len(), 24);
    fill(&db, Kind::Coc, false).await;
    db.release().await.unwrap();
}
