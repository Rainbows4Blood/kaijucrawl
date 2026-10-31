//! Synthetic contracts only; the user's catalogue is never opened by tests.
use kaijucrawl::{
    analysis::Status,
    earth_context::{self, Basis, Data, Kind, Precision},
    paths::LocalPaths,
    storage::{earth_context as stored, overview, sqlite::Catalogue},
};
use serde_json::{Value, json};
use sqlx::Row;
fn envelope(data: Value) -> Value {
    json!({"status":"completed","model":"gpt-6-luna","error":null,"incomplete_details":null,"output":[{"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":data.to_string()}]}],"usage":{"input_tokens":3000,"output_tokens":40,"total_tokens":3040,"input_tokens_details":{"cached_tokens":2000,"cache_write_tokens":0}}})
}
fn unknown(kind: Kind) -> Value {
    if kind.field() == "location" {
        json!({"location":null})
    } else {
        json!({"date_start":null,"date_end":null,"precision":"unknown","basis":"unknown"})
    }
}
fn known(kind: Kind) -> Value {
    match kind {
        Kind::ShadowrunLocation | Kind::CocLocation => json!({"location":"Seattle; Berlin"}),
        Kind::ShadowrunDate => {
            json!({"date_start":"2084-01-01","date_end":"2084-12-31","precision":"year","basis":"explicit"})
        }
        Kind::CocDate => {
            json!({"date_start":null,"date_end":null,"precision":"era","basis":"dark_ages"})
        }
    }
}

#[test]
fn dates_preserve_calendar_precision_and_apply_only_approved_coc_eras() {
    for (start, end, precision) in [
        ("2084-01-01", "2084-12-31", "year"),
        ("1920-01-01", "1929-12-31", "decade"),
        ("1924-02-01", "1924-02-29", "month"),
        ("1924-10-31", "1924-10-31", "day"),
        ("0950-01-01", "1050-12-31", "range"),
    ] {
        let data = Kind::ShadowrunDate
            .data(
                json!({"date_start":start,"date_end":end,"precision":precision,"basis":"explicit"}),
            )
            .unwrap();
        let Data::Date { range } = data else { panic!() };
        assert_eq!(range.start.unwrap().to_string(), start);
        assert_eq!(range.end.unwrap().to_string(), end);
    }
    for (basis, start, end) in [
        ("modern", "1980-01-01", "2030-12-31"),
        ("dark_ages", "0950-01-01", "1050-12-31"),
    ] {
        let value = json!({"date_start":null,"date_end":null,"precision":"era","basis":basis});
        assert!(Kind::ShadowrunDate.data(value.clone()).is_err());
        let Data::Date { range } = Kind::CocDate.data(value).unwrap() else {
            panic!()
        };
        assert_eq!(range.start.unwrap().to_string(), start);
        assert_eq!(range.end.unwrap().to_string(), end);
        assert_eq!(range.precision, Precision::Era);
        assert_ne!(range.basis, Basis::Explicit);
    }
    for value in [
        json!({"date_start":"1920-01-01","date_end":null,"precision":"range","basis":"explicit"}),
        json!({"date_start":null,"date_end":"1929-12-31","precision":"range","basis":"explicit"}),
    ] {
        assert!(Kind::CocDate.data(value).unwrap().known())
    }
    let base = json!({"date_start":"2084-01-01","date_end":"2084-12-31","precision":"year","basis":"explicit"});
    for (key, value) in [
        ("date_start", json!("2085-01-01")),
        ("date_start", json!("2084-02-30")),
        ("date_start", json!("0000-01-01")),
        ("date_start", json!("2084-1-01")),
        ("date_start", json!(2084)),
        ("date_end", json!("2084-11-30")),
        ("precision", json!("day")),
        ("basis", json!("modern")),
        ("extra", json!(true)),
    ] {
        let mut bad = base.clone();
        bad[key] = value;
        assert!(Kind::CocDate.data(bad).is_err(), "{key}");
    }
    for kind in Kind::ALL {
        assert!(!kind.data(unknown(kind)).unwrap().known());
        let mut refusal = envelope(unknown(kind));
        refusal["output"][0]["content"][0] = json!({"type":"refusal"});
        assert!(kind.decode(refusal).unwrap().data.is_none());
    }
    assert_eq!(
        Kind::CocLocation
            .data(json!({"location":"  Arkham, Massachusetts  "}))
            .unwrap(),
        Data::Location {
            location: Some("Arkham, Massachusetts".into())
        }
    );
    for bad in [
        json!({"location":[]}),
        json!({"location":""}),
        json!({"location":"a\nb"}),
        json!({"location":"a".repeat(1025)}),
        json!({"location":"Paris","date":null}),
    ] {
        assert!(Kind::CocLocation.data(bad).is_err())
    }
}

#[test]
fn extraction_requests_are_strict_cached_bounded_and_independent() {
    for kind in Kind::ALL {
        let bytes = kind
            .encode(&json!({"title":"Synthetic","description":"Saved full evidence"}))
            .unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["model"], earth_context::MODEL);
        assert_eq!(body["text"]["format"]["strict"], true);
        assert_eq!(body["store"], false);
        assert!(
            body["input"][0]
                .to_string()
                .contains("prompt_cache_breakpoint")
        );
        assert!(
            !body["input"][1]
                .to_string()
                .contains("prompt_cache_breakpoint")
        );
        assert_eq!(body["prompt_cache_key"], kind.algorithm());
        assert!(body["input"][1].to_string().contains("Saved full evidence"));
        assert!(
            kind.encode(&json!({"title":"large","description":"x".repeat(128*1024)}))
                .is_err()
        );
        let changed: Value = serde_json::from_slice(
            &kind
                .encode(&json!({"title":"Another","description":"Different full evidence"}))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(body["input"][0], changed["input"][0]);
    }
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
async fn fill(db: &Catalogue, kind: Kind, again: bool) {
    let mut s = db.begin_earth(kind, again).await.unwrap();
    assert_eq!(s.total, 3);
    let mut after = 0;
    while let Some(book) = db.earth_next_after(&s, after).await.unwrap() {
        after = book.document;
        assert!(!book.evidence.as_ref().unwrap().contains("DO_NOT_SEND"));
        db.classification_dispatch(&mut s, &book).await.unwrap();
        let response = kind
            .decode(envelope(if book.product_id == "3" {
                unknown(kind)
            } else {
                known(kind)
            }))
            .unwrap();
        db.earth_commit(&mut s, &book, Some(&response), None)
            .await
            .unwrap();
        check(db, &s).await;
    }
    assert_eq!(s.processed, 3);
    db.finish_analysis(&mut s, "completed", "").await.unwrap();
}
#[tokio::test]
async fn rules_filter_without_prior_analyses_and_all_four_results_resume_independently() {
    let (_temp, paths, db) = seed().await;
    let before = canonical(&db).await;
    let identity = db.catalogue;
    for kind in Kind::ALL {
        fill(&db, kind, false).await;
        let saved = stored::saved(&db.pool, "drivethru", "3", kind)
            .await
            .unwrap()
            .unwrap();
        assert!(!saved.data.known());
    }
    assert_eq!(canonical(&db).await, before);
    db.release().await.unwrap();
    db.pool.close().await;
    let db = Catalogue::connect(&paths).await.unwrap();
    assert_eq!(db.catalogue, identity);
    for kind in Kind::ALL {
        let mut s = db.begin_earth(kind, false).await.unwrap();
        assert_eq!(s.total, 0);
        assert_eq!(
            s.overview.as_ref().unwrap().analyses[kind.algorithm()].completed,
            3
        );
        db.finish_analysis(&mut s, "completed", "").await.unwrap();
        fill(&db, kind, true).await;
    }
    assert_eq!(canonical(&db).await, before);
    db.release().await.unwrap();
}
#[tokio::test]
async fn scope_and_evidence_changes_invalidate_but_other_analysis_and_rating_changes_do_not() {
    let (_temp, _paths, db) = seed().await;
    for kind in Kind::ALL {
        fill(&db, kind, false).await;
    }
    sqlx::query("UPDATE products SET record=json_set(record,'$.rating',2,'$.min_players',4)")
        .execute(&db.pool)
        .await
        .unwrap();
    let mut score = db.begin_analysis().await.unwrap();
    db.analysis_batch(&mut score).await.unwrap();
    check(&db, &score).await;
    db.finish_analysis(&mut score, "completed", "")
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM earth_context")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        12
    );
    sqlx::query("UPDATE products SET record=json_set(record,'$.rule_systems',json('[\"Call of Cthulhu\"]')) WHERE product_id='1'").execute(&db.pool).await.unwrap();
    sqlx::query(
        "UPDATE products SET description_text='Changed full description' WHERE product_id='4'",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query("DELETE FROM products WHERE product_id='3'")
        .execute(&db.pool)
        .await
        .unwrap();
    let totals = overview::read(&mut db.pool.acquire().await.unwrap())
        .await
        .unwrap();
    assert_eq!(
        (
            totals.analyses[Kind::ShadowrunDate.algorithm()].eligible,
            totals.analyses[Kind::ShadowrunDate.algorithm()].completed
        ),
        (1, 1)
    );
    assert_eq!(
        (
            totals.analyses[Kind::CocDate.algorithm()].eligible,
            totals.analyses[Kind::CocDate.algorithm()].completed
        ),
        (3, 1)
    );
    assert!(
        stored::saved(&db.pool, "drivethru", "1", Kind::ShadowrunDate)
            .await
            .unwrap()
            .is_none()
    );
    db.release().await.unwrap();
}
#[tokio::test]
async fn rollback_out_of_order_recovery_refusal_and_stale_ownership_preserve_good_results() {
    let (_temp, paths, db) = seed().await;
    fill(&db, Kind::ShadowrunDate, false).await;
    let mut s = db.begin_earth(Kind::ShadowrunDate, true).await.unwrap();
    let first = db.earth_next_after(&s, 0).await.unwrap().unwrap();
    let second = db
        .earth_next_after(&s, first.document)
        .await
        .unwrap()
        .unwrap();
    db.classification_dispatch(&mut s, &first).await.unwrap();
    db.classification_dispatch(&mut s, &second).await.unwrap();
    db.classification_retry_wait(&s, &first, "Network connection failed")
        .await
        .unwrap();
    let response = Kind::ShadowrunDate
        .decode(envelope(unknown(Kind::ShadowrunDate)))
        .unwrap();
    sqlx::raw_sql("CREATE TRIGGER fail_earth BEFORE UPDATE ON earth_context BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").execute(&db.pool).await.unwrap();
    assert!(
        db.earth_commit(&mut s, &second, Some(&response), None)
            .await
            .is_err()
    );
    assert_eq!(s.processed, 0);
    check(&db, &s).await;
    sqlx::raw_sql("DROP TRIGGER fail_earth")
        .execute(&db.pool)
        .await
        .unwrap();
    db.earth_commit(&mut s, &second, Some(&response), None)
        .await
        .unwrap();
    check(&db, &s).await;
    assert!(
        db.earth_commit(&mut s, &second, Some(&response), None)
            .await
            .is_err()
    );
    db.release().await.unwrap();
    let next = Catalogue::connect(&paths).await.unwrap();
    assert!(
        db.earth_commit(&mut s, &first, Some(&response), None)
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT state FROM classification_attempts WHERE run_id=? AND document=?"
        )
        .bind(s.id)
        .bind(first.document)
        .fetch_one(&next.pool)
        .await
        .unwrap(),
        "interrupted"
    );
    let mut again = next.begin_earth(Kind::ShadowrunDate, true).await.unwrap();
    assert_eq!(
        next.earth_next_after(&again, 0)
            .await
            .unwrap()
            .unwrap()
            .document,
        first.document
    );
    let refused = earth_context::Response {
        model: "gpt-6-luna".into(),
        data: None,
        usage: Kind::ShadowrunDate
            .decode(envelope(known(Kind::ShadowrunDate)))
            .unwrap()
            .usage,
    };
    next.earth_commit(&mut again, &first, Some(&refused), None)
        .await
        .unwrap();
    assert!(
        stored::saved(&next.pool, "drivethru", "1", Kind::ShadowrunDate)
            .await
            .unwrap()
            .unwrap()
            .data
            .known()
    );
    check(&next, &again).await;
    next.finish_analysis(&mut again, "cancelled", "")
        .await
        .unwrap();
    next.release().await.unwrap();
    db.pool.close().await;
}
#[tokio::test]
async fn schema_eleven_migration_preserves_existing_data_and_builds_indexed_scope_only() {
    let (_temp, paths, db) = seed().await;
    let before = canonical(&db).await;
    let identity = db.catalogue;
    let mut score = db.begin_analysis().await.unwrap();
    db.analysis_batch(&mut score).await.unwrap();
    db.finish_analysis(&mut score, "completed", "")
        .await
        .unwrap();
    sqlx::raw_sql(include_str!("fixtures/schema_before_earth_context.sql"))
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE catalogue SET version=11")
        .execute(&db.pool)
        .await
        .unwrap();
    db.release().await.unwrap();
    db.pool.close().await;
    let db = Catalogue::connect(&paths).await.unwrap();
    assert_eq!(db.catalogue, identity);
    assert_eq!(canonical(&db).await, before);
    let o = overview::read(&mut db.pool.acquire().await.unwrap())
        .await
        .unwrap();
    assert_eq!(o.analyses[kaijucrawl::analysis::ALGORITHM].completed, 9);
    for kind in Kind::ALL {
        assert_eq!(
            (
                o.analyses[kind.algorithm()].eligible,
                o.analyses[kind.algorithm()].completed
            ),
            (3, 0)
        );
    }
    let plan=sqlx::query("EXPLAIN QUERY PLAN SELECT document FROM earth_scope WHERE system='coc' AND document>0 ORDER BY document LIMIT 1").fetch_all(&db.pool).await.unwrap();
    assert!(
        plan.iter()
            .any(|r| r.get::<String, _>("detail").contains("earth_scope_keyset"))
    );
    db.release().await.unwrap();
}
