//! Synthetic Warhammer routing, multi-membership, context and migration contracts.
use kaijucrawl::{
    analysis::Status,
    decisions::{self, Decision},
    dnd_context::Response,
    paths::LocalPaths,
    responses,
    storage::{overview, sqlite::Catalogue, warhammer_context as stored},
    warhammer::Family,
    warhammer_context::Kind,
};
use serde_json::{Value, json};
use sqlx::Row;
fn decision(f: Family, family: &str, games: &[&str]) -> Decision {
    serde_json::from_value(json!({"model":decisions::MODEL,"answers":std::iter::once("family").chain(f.games().iter().map(|g|g.0)).map(|key|{
        let chosen=if key=="family"{family}else if games.contains(&key){"supported"}else{"uncertain"};
        json!({"type":"choice","name":key,"choice":chosen,"confidence":1.0,"probabilities":(["supported","not_supported","uncertain"].iter().map(|v|json!({"value":v,"probability":if *v==chosen{1.0}else{0.0}})).collect::<Vec<_>>())})
    }).collect::<Vec<_>>(),"usage":{"input_tokens":20,"output_tokens":0,"total_tokens":20,"input_tokens_details":{"cached_tokens":0,"cache_write_tokens":0}}})).unwrap()
}
fn response(k: Kind, value: Value) -> Response {
    k.decode_response(responses::Response {
        model: responses::MODEL.into(),
        value: Some(value),
        usage: decision(Family::Fantasy, "supported", &[]).usage,
    })
    .unwrap()
}
#[test]
fn family_and_exact_game_questions_support_multiple_unknown_negative_and_refusal() {
    for f in Family::ALL {
        f.request(json!({"title":"Synthetic","description":"Source"}))
            .validate()
            .unwrap();
        let games = [f.games()[0].0, f.games()[1].0];
        assert_eq!(
            f.answer(&decision(f, "supported", &games))
                .unwrap()
                .unwrap()
                .members,
            games
        );
        for (choice, category) in [
            ("supported", "unknown"),
            ("not_supported", "not_applicable"),
            ("uncertain", "uncertain"),
        ] {
            let answer = f.answer(&decision(f, choice, &[])).unwrap().unwrap();
            assert_eq!(answer.category, category);
            assert_eq!(answer.members, vec![category]);
        }
        assert!(f.answer(&decision(f, "not_supported", &games)).is_err());
        let mut refused = decision(f, "supported", &games);
        refused.answers[1] = decisions::Answer::Refusal {
            name: games[0].into(),
        };
        assert!(f.answer(&refused).unwrap().is_none());
        refused.answers.pop();
        assert!(f.answer(&refused).is_err());
    }
}
#[test]
fn all_four_context_contracts_preserve_fictional_dates_and_cached_escalation() {
    let evidence = json!({"title":"Synthetic","description":"Original full description"});
    for k in Kind::ALL {
        let mut bytes = k.encode(&evidence).unwrap();
        let original: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(original["store"], false);
        assert_eq!(original["prompt_cache_options"]["mode"], "explicit");
        assert!(original["input"][0]["content"][0]["prompt_cache_breakpoint"].is_object());
        assert!(original["input"][1]["content"][0]["prompt_cache_breakpoint"].is_null());
        for model in responses::Model::ALL {
            let now: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(now["model"], model.name());
            assert_eq!(now["input"], original["input"]);
            assert_eq!(responses::escalate(&mut bytes).unwrap(), model.next());
        }
        let value = if k.field() == "date" {
            json!({"in_universe_date":"late 123.456.M41–M42 / approximate"})
        } else {
            json!({"locations":["Altdorf","Calixis Sector"]})
        };
        assert!(response(k, value).data.unwrap().known());
        let null = if k.field() == "date" {
            json!({"in_universe_date":null})
        } else {
            json!({"locations":[]})
        };
        assert!(!response(k, null).data.unwrap().known());
        assert!(
            k.data(json!({"in_universe_date":1920,"locations":"wrong"}))
                .is_err()
        );
        assert!(k.encode(&json!({"title":"missing description"})).is_err());
    }
    assert!(
        Kind::FantasyDate
            .data(json!({"in_universe_date":"x".repeat(1025)}))
            .is_err()
    );
    assert!(
        Kind::FortyKLocations
            .data(json!({"locations":["x".repeat(161)]}))
            .is_err()
    );
}
async fn seed() -> (tempfile::TempDir, LocalPaths, Catalogue) {
    let temp = tempfile::tempdir().unwrap();
    let paths = LocalPaths::at(temp.path());
    let db = Catalogue::connect(&paths).await.unwrap();
    let intent = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO intents(id,target,product_id,payload,generation,finalized) VALUES(?,'drivethru','1','{}',?,1)").bind(intent).bind(db.generation).execute(&db.pool).await.unwrap();
    for (i, rules) in [
        json!(["Warhammer"]),
        json!([" Warhammer Fantasy Roleplay "]),
        json!(["WARHAMMER 40K ROLEPLAY"]),
        json!(["Warhammer: The Old World Roleplaying Game"]),
        json!(["Warhammer Age of Sigmar: Soulbound"]),
        json!(["Dark Heresy 2nd Edition"]),
        json!(["Wrath & Glory"]),
        json!(["D&D"]),
        json!(["Old World"]),
        json!(null),
    ]
    .into_iter()
    .enumerate()
    {
        sqlx::query("INSERT INTO products(target,product_id,title,description_text,record,parser_version,record_schema_version,archive,fetched_at,last_checked_at,intent) VALUES('drivethru',?,'Synthetic Warhammer in title only','Full fictional description',?,'drivethru-rust-1',2,'{}','2026-01-01','2026-01-01',?)").bind((i+1).to_string()).bind(json!({"rule_systems":rules,"rating":4.7,"raw_metadata":"DO_NOT_SEND","previous_analysis":"DO_NOT_SEND"}).to_string()).bind(intent).execute(&db.pool).await.unwrap();
    }
    (temp, paths, db)
}
async fn check(db: &Catalogue, s: &Status) {
    let mut actual = overview::read(&mut db.pool.acquire().await.unwrap())
        .await
        .unwrap();
    let mut expected = s.overview.clone().unwrap();
    for v in [&mut actual, &mut expected] {
        for c in v.analyses.values_mut() {
            c.counts.retain(|_, n| *n != 0);
        }
    }
    assert_eq!(
        serde_json::to_value(actual).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
}
async fn fill(db: &Catalogue, f: Family) -> Status {
    let mut s = db.begin_warhammer(f, false).await.unwrap();
    assert_eq!(s.total, 4);
    while let Some(b) = db.classification_next(&s).await.unwrap() {
        assert!(!b.evidence.as_ref().unwrap().contains("DO_NOT_SEND"));
        let (family, games) = if b.product_id == "1" {
            ("supported", vec![])
        } else {
            ("supported", vec![f.games()[0].0, f.games()[1].0])
        };
        db.classification_dispatch(&mut s, &b).await.unwrap();
        db.classification_commit(&mut s, &b, Some(&decision(f, family, &games)), None)
            .await
            .unwrap();
        check(db, &s).await;
    }
    db.finish_analysis(&mut s, "completed", "").await.unwrap();
    s
}
async fn contexts(db: &Catalogue) {
    for k in Kind::ALL {
        let mut s = db.begin_warhammer_context(k, false).await.unwrap();
        assert_eq!(s.total, 4);
        let mut after = 0;
        while let Some(b) = db.warhammer_context_next_after(&s, after).await.unwrap() {
            after = b.document;
            assert!(!b.evidence.as_ref().unwrap().contains("DO_NOT_SEND"));
            let value = match (k.field(), b.product_id.as_str()) {
                ("date", "1") => json!({"in_universe_date":null}),
                ("date", _) => json!({"in_universe_date":"2512 IC / late M41"}),
                ("locations", "1") => json!({"locations":[]}),
                _ => json!({"locations":["Synthetic Place"]}),
            };
            db.classification_dispatch(&mut s, &b).await.unwrap();
            db.warhammer_context_commit(&mut s, &b, Some(&response(k, value)), None)
                .await
                .unwrap();
            check(db, &s).await;
        }
        db.finish_analysis(&mut s, "completed", "").await.unwrap();
        let mut reused = db.begin_warhammer_context(k, false).await.unwrap();
        assert_eq!(reused.total, 0);
        db.finish_analysis(&mut reused, "completed", "")
            .await
            .unwrap();
    }
}
#[tokio::test]
async fn subsets_context_reuse_live_deltas_invalidation_and_keyset() {
    let (_temp, _paths, db) = seed().await;
    for f in Family::ALL {
        let s = fill(&db, f).await;
        let c = &s.overview.unwrap().analyses[f.algorithm()];
        assert_eq!(c.counts["unknown"], 1);
        assert_eq!(c.counts["identified"], 3);
    }
    contexts(&db).await;
    // Identical and still-positive reclassification retain already calculated context.
    let mut s = db.begin_warhammer(Family::Fantasy, true).await.unwrap();
    let b = db.classification_next(&s).await.unwrap().unwrap();
    db.classification_commit(
        &mut s,
        &b,
        Some(&decision(Family::Fantasy, "supported", &["wfrp_5e"])),
        None,
    )
    .await
    .unwrap();
    check(&db, &s).await;
    assert!(
        stored::saved(&db.pool, "drivethru", "1", Kind::FantasyDate)
            .await
            .unwrap()
            .is_some()
    );
    db.finish_analysis(&mut s, "cancelled", "").await.unwrap();
    // Leaving this family removes only this family's context, with immediate totals.
    let mut s = db.begin_warhammer(Family::Fantasy, true).await.unwrap();
    let b = db.classification_next(&s).await.unwrap().unwrap();
    db.classification_commit(
        &mut s,
        &b,
        Some(&decision(Family::Fantasy, "not_supported", &[])),
        None,
    )
    .await
    .unwrap();
    check(&db, &s).await;
    assert!(
        stored::saved(&db.pool, "drivethru", "1", Kind::FantasyDate)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        stored::saved(&db.pool, "drivethru", "1", Kind::FortyKDate)
            .await
            .unwrap()
            .is_some()
    );
    db.finish_analysis(&mut s, "completed", "").await.unwrap();
    sqlx::query("UPDATE products SET record=json_set(record,'$.rating',2.0) WHERE product_id='3'")
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(
        stored::saved(&db.pool, "drivethru", "3", Kind::FortyKDate)
            .await
            .unwrap()
            .is_some()
    );
    sqlx::query("UPDATE products SET description_text='Changed source' WHERE product_id='3'")
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(
        stored::saved(&db.pool, "drivethru", "3", Kind::FortyKDate)
            .await
            .unwrap()
            .is_none()
    );
    sqlx::query("UPDATE products SET record=json_set(record,'$.rule_systems',json_array('D&D')) WHERE product_id='1'").execute(&db.pool).await.unwrap();
    assert!(
        stored::saved(&db.pool, "drivethru", "1", Kind::FortyKDate)
            .await
            .unwrap()
            .is_none()
    );
    let plan=sqlx::query("EXPLAIN QUERY PLAN SELECT s.rowid FROM warhammer_scope e CROSS JOIN products s ON s.target=e.target AND s.product_id=e.product_id CROSS JOIN book_classifications c ON c.target=e.target AND c.product_id=e.product_id AND c.system=e.system WHERE e.system='warhammer_fantasy' AND c.algorithm='warhammer-fantasy-games-1' AND c.category IN ('identified','unknown') AND e.document>0 AND e.document<=100 ORDER BY e.document LIMIT 1").fetch_all(&db.pool).await.unwrap();
    assert!(plan.iter().any(|r| {
        r.get::<String, _>("detail")
            .contains("warhammer_scope_keyset")
    }));
    assert!(
        !plan
            .iter()
            .any(|r| r.get::<String, _>("detail").contains("SCAN s"))
    );
    db.release().await.unwrap();
}
#[tokio::test]
async fn stale_owner_changed_evidence_and_refusal_cannot_replace_good_context() {
    let (_temp, paths, db) = seed().await;
    for f in Family::ALL {
        fill(&db, f).await;
    }
    contexts(&db).await;
    let mut s = db
        .begin_warhammer_context(Kind::FantasyDate, true)
        .await
        .unwrap();
    let a = db
        .warhammer_context_next_after(&s, 0)
        .await
        .unwrap()
        .unwrap();
    let b = db
        .warhammer_context_next_after(&s, a.document)
        .await
        .unwrap()
        .unwrap();
    db.classification_dispatch(&mut s, &a).await.unwrap();
    db.classification_dispatch(&mut s, &b).await.unwrap();
    db.warhammer_context_commit(
        &mut s,
        &b,
        Some(&response(
            Kind::FantasyDate,
            json!({"in_universe_date":"2513 IC"}),
        )),
        None,
    )
    .await
    .unwrap();
    check(&db, &s).await;
    db.warhammer_context_commit(&mut s, &a, None, Some("Synthetic refusal"))
        .await
        .unwrap();
    check(&db, &s).await;
    assert!(
        stored::saved(&db.pool, "drivethru", "1", Kind::FantasyDate)
            .await
            .unwrap()
            .is_some()
    );
    let c = db
        .warhammer_context_next_after(&s, b.document)
        .await
        .unwrap()
        .unwrap();
    sqlx::query("UPDATE products SET title='Changed evidence' WHERE product_id=?")
        .bind(&c.product_id)
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(
        db.warhammer_context_commit(
            &mut s,
            &c,
            Some(&response(
                Kind::FantasyDate,
                json!({"in_universe_date":null})
            )),
            None
        )
        .await
        .is_err()
    );
    db.release().await.unwrap();
    let fresh = Catalogue::connect(&paths).await.unwrap();
    assert!(
        db.warhammer_context_commit(&mut s, &c, None, Some("stale"))
            .await
            .is_err()
    );
    fresh.release().await.unwrap();
}
#[tokio::test]
async fn schema_fifteen_upgrade_preserves_identity_captures_results_and_history() {
    let (_temp, paths, db) = seed().await;
    let identity = db.catalogue;
    let before: Vec<(i64, String, String, String)> =
        sqlx::query_as("SELECT rowid,record,archive,fetched_at FROM products ORDER BY rowid")
            .fetch_all(&db.pool)
            .await
            .unwrap();
    let mut score = db.begin_analysis().await.unwrap();
    db.finish_analysis(&mut score, "cancelled", "preserve history")
        .await
        .unwrap();
    sqlx::raw_sql(include_str!("fixtures/schema_before_warhammer.sql"))
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE catalogue SET version=15")
        .execute(&db.pool)
        .await
        .unwrap();
    db.release().await.unwrap();
    db.pool.close().await;
    let db = Catalogue::connect(&paths).await.unwrap();
    assert_eq!(db.catalogue, identity);
    let after: Vec<(i64, String, String, String)> =
        sqlx::query_as("SELECT rowid,record,archive,fetched_at FROM products ORDER BY rowid")
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(before, after);
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT detail FROM analysis_runs WHERE id=?")
            .bind(score.id)
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        "preserve history"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM warhammer_scope")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        8
    );
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&db.pool)
            .await
            .unwrap()
            .is_empty()
    );
    db.release().await.unwrap();
}
