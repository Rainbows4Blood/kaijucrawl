//! Synthetic D&D setting/context contracts, migrations and persistence; no live API.
use kaijucrawl::{
    analysis::Status,
    decisions::{self, Decision},
    dnd_context::{Data, Kind},
    dnd_settings,
    paths::LocalPaths,
    storage::{classification, dnd_context as stored, overview, sqlite::Catalogue},
};
use serde_json::{Value, json};
use sqlx::Row;

fn settings(names: &[&str]) -> Decision {
    let answers:Vec<_>=dnd_settings::SETTINGS.iter().map(|s|s.0).chain(["any"]).map(|key|{
  let choice=if names.contains(&key){"applies"}else{"uncertain"};
  let probabilities = ["applies","not_applicable","uncertain"].map(|v|json!({"value":v,"probability":if v==choice{1}else{0}}));
  json!({"type":"choice","name":key,"choice":choice,"confidence":1,"probabilities":probabilities})
 }).collect();
    serde_json::from_value(json!({"model":decisions::MODEL,"answers":answers,"usage":{"input_tokens":2000,"output_tokens":0,"total_tokens":2000,"input_tokens_details":{"cached_tokens":0,"cache_write_tokens":0}}})).unwrap()
}
fn response(data: Value) -> Value {
    json!({"status":"completed","model":"gpt-6-luna","error":null,"incomplete_details":null,"output":[{"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":data.to_string()}]}],"usage":{"input_tokens":3000,"output_tokens":40,"total_tokens":3040,"input_tokens_details":{"cached_tokens":2500,"cache_write_tokens":0}}})
}
fn unknown(kind: Kind) -> Value {
    match kind {
        Kind::Levels => json!({"min_level":null,"max_level":null}),
        Kind::Locations => json!({"locations":[]}),
        Kind::Date => json!({"in_universe_date":null}),
    }
}
fn known(kind: Kind) -> Value {
    match kind {
        Kind::Levels => json!({"min_level":21,"max_level":36}),
        Kind::Locations => json!({"locations":["Stonebridge","Aster"]}),
        Kind::Date => json!({"in_universe_date":"circa 1492 DR"}),
    }
}

#[test]
fn settings_are_multilabel_with_distinct_fallbacks_and_al_qadim_parent() {
    let input = json!({"title":"Synthetic","description":"Substantive setting material"});
    let request = dnd_settings::request(input);
    request.validate().unwrap();
    assert_eq!(request.questions.len(), 15);
    let result = dnd_settings::answer(&settings(&["al_qadim", "ravenloft"]))
        .unwrap()
        .unwrap();
    assert_eq!(result.category, "identified");
    assert_eq!(
        result.members,
        vec!["forgotten_realms", "ravenloft", "al_qadim"]
    );
    assert_eq!(result.probabilities.as_array().unwrap().len(), 15);
    for (names, category) in [(&[][..], "unknown"), (&["any"][..], "any")] {
        let result = dnd_settings::answer(&settings(names)).unwrap().unwrap();
        assert_eq!(result.category, category);
        assert_eq!(result.members, vec![category]);
    }
    let conflict = dnd_settings::answer(&settings(&["eberron", "any"]))
        .unwrap()
        .unwrap();
    assert_eq!(conflict.members, vec!["eberron"]);
    let mut malformed = serde_json::to_value(settings(&[])).unwrap();
    malformed["answers"][2] = json!({"type":"refusal","name":"blackmoor"});
    assert!(
        dnd_settings::answer(&serde_json::from_value(malformed.clone()).unwrap())
            .unwrap()
            .is_none()
    );
    malformed["answers"][3]["name"] = json!("bad");
    assert!(dnd_settings::answer(&serde_json::from_value(malformed).unwrap()).is_err());
}

#[test]
fn strict_extraction_keeps_unknowns_calendars_and_levels_above_twenty() {
    for kind in Kind::ALL {
        let none = kind.decode(response(unknown(kind))).unwrap();
        assert!(!none.data.unwrap().known());
        let yes = kind.decode(response(known(kind))).unwrap();
        assert!(yes.data.unwrap().known());
        assert_eq!(yes.usage.input_tokens_details.cached_tokens, 2500);
        let mut refusal = response(unknown(kind));
        refusal["output"][0]["content"][0] =
            json!({"type":"refusal","refusal":"Do not retain this provider text"});
        assert!(kind.decode(refusal).unwrap().data.is_none());
        for bad in [
            json!({}),
            json!({"other":null}),
            json!({"locations":[3]}),
            json!({"min_level":2,"max_level":1}),
        ] {
            assert!(kind.decode(response(bad)).is_err());
        }
        let mut incomplete = response(known(kind));
        incomplete["status"] = json!("incomplete");
        assert!(kind.decode(incomplete).is_err());
        let mut invalid_usage = response(known(kind));
        invalid_usage["usage"]["total_tokens"] = json!(1);
        assert!(kind.decode(invalid_usage).is_err());
    }
    for value in [
        json!({"min_level":-1,"max_level":20}),
        json!({"min_level":1.5,"max_level":20}),
        json!({"min_level":1}),
        json!({"min_level":1,"max_level":20,"extra":true}),
    ] {
        assert!(Kind::Levels.data(value).is_err());
    }
    assert!(
        Kind::Levels
            .data(json!({"min_level":0,"max_level":0}))
            .unwrap()
            .known()
    );
    assert_eq!(
        Kind::Levels
            .data(json!({"min_level":null,"max_level":36}))
            .unwrap(),
        Data::Levels {
            min_level: None,
            max_level: Some(36)
        }
    );
    assert_eq!(
        Kind::Locations
            .data(json!({"locations":[" Aster ","aster","Stonebridge"]}))
            .unwrap(),
        Data::Locations {
            locations: vec!["Aster".into(), "Stonebridge".into()]
        }
    );
    for value in [
        json!({"locations":[""]}),
        json!({"locations":["bad\u{001b}"]}),
        json!({"locations":vec!["x";65]}),
        json!({"locations":["x".repeat(161)]}),
    ] {
        assert!(Kind::Locations.data(value).is_err());
    }
    for value in [
        json!({"in_universe_date":1492}),
        json!({"in_universe_date":""}),
        json!({"in_universe_date":"x".repeat(1025)}),
    ] {
        assert!(Kind::Date.data(value).is_err());
    }
    assert_eq!(
        Kind::Date
            .data(json!({"in_universe_date":"570–576 CY"}))
            .unwrap(),
        Data::Date {
            in_universe_date: Some("570–576 CY".into())
        }
    );
}

#[test]
fn responses_cache_only_stable_instructions_and_keep_full_source_evidence() {
    let evidence = json!({"title":"Synthetic book","description":"For character levels 3–5. In Stonebridge, 1492 DR.","rule_systems":["Noisy source taxonomy"]});
    for kind in Kind::ALL {
        let wire: Value = serde_json::from_slice(&kind.encode(&evidence).unwrap()).unwrap();
        let other: Value = serde_json::from_slice(
            &kind
                .encode(&json!({"title":"Another","description":"No details"}))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(wire["model"], "gpt-6-luna");
        assert_eq!(wire["store"], false);
        assert_eq!(wire["text"]["format"]["strict"], true);
        assert_eq!(wire["prompt_cache_options"]["mode"], "explicit");
        assert_eq!(wire["prompt_cache_key"], kind.algorithm());
        assert_eq!(wire["input"][0], other["input"][0]);
        assert!(
            wire["input"][0]["content"][0]["text"]
                .as_str()
                .unwrap()
                .len()
                > 8000
        );
        assert_eq!(
            wire["input"][0]["content"][0]["prompt_cache_breakpoint"]["mode"],
            "explicit"
        );
        assert!(
            wire["input"][1]["content"][0]
                .get("prompt_cache_breakpoint")
                .is_none()
        );
        assert_eq!(wire["input"][1]["content"][0]["text"], evidence.to_string());
        assert!(
            kind.encode(&json!({"title":"Large","description":"x".repeat(128*1024)}))
                .is_err()
        );
    }
}

async fn seed() -> (tempfile::TempDir, LocalPaths, Catalogue) {
    let temp = tempfile::tempdir().unwrap();
    let paths = LocalPaths::at(temp.path());
    let db = Catalogue::connect(&paths).await.unwrap();
    let intent = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO intents(id,target,product_id,payload,generation,finalized) VALUES(?,'drivethru','1','{}',?,1)").bind(intent).bind(db.generation).execute(&db.pool).await.unwrap();
    for id in 1..=6 {
        sqlx::query("INSERT INTO products(target,product_id,title,description_text,record,parser_version,record_schema_version,archive,fetched_at,last_checked_at,intent) VALUES('drivethru',?,'Synthetic D&D book','A complete fictional adventure in Stonebridge for levels 21–36, circa 1492 DR.',?,'drivethru-rust-1',2,'{}','2026-01-01','2026-01-01',?)").bind(id.to_string()).bind(json!({"rating":4.7,"rating_count":1000,"min_players":99,"dnd_settings":"DO_NOT_SEND","raw_metadata":"DO_NOT_SEND"}).to_string()).bind(intent).execute(&db.pool).await.unwrap();
    }
    let mut run = db.begin_dnd(false, false).await.unwrap();
    sqlx::query("INSERT INTO book_classifications(target,product_id,system,category,algorithm,model,confidence,probabilities,input_hash,calculated_at,run_id) SELECT target,product_id,'dnd',CASE WHEN product_id='5' THEN 'no' WHEN product_id='4' THEN 'dnd_and_others' ELSE 'dnd_only' END,'dnd-compatibility-1','gpt-6-luna',1,'[]','synthetic','2026-01-01',? FROM products WHERE product_id!='6'").bind(run.id).execute(&db.pool).await.unwrap();
    db.finish_analysis(&mut run, "completed", "").await.unwrap();
    (temp, paths, db)
}
async fn canonical(db: &Catalogue) -> Vec<(i64, String, String, String)> {
    sqlx::query_as("SELECT rowid,record,archive,fetched_at FROM products ORDER BY rowid")
        .fetch_all(&db.pool)
        .await
        .unwrap()
}
async fn totals_match(db: &Catalogue, run: &Status) {
    let actual = overview::read(&mut db.pool.acquire().await.unwrap())
        .await
        .unwrap();
    let expected = run.overview.as_ref().unwrap();
    assert_eq!(actual.catalogue_total, expected.catalogue_total);
    for (key, a) in actual.analyses {
        let b = &expected.analyses[&key];
        assert_eq!(
            (a.eligible, a.completed, a.prerequisite_pending),
            (b.eligible, b.completed, b.prerequisite_pending),
            "{key}"
        );
        for count in a.counts.keys().chain(b.counts.keys()) {
            assert_eq!(
                a.counts.get(count).copied().unwrap_or(0),
                b.counts.get(count).copied().unwrap_or(0),
                "{key} {count}"
            );
        }
    }
}
async fn fill(db: &Catalogue, kind: Kind, again: bool) {
    let mut run = db.begin_context(kind, again).await.unwrap();
    let mut after = 0;
    while let Some(book) = db.context_next_after(&run, after).await.unwrap() {
        after = book.document;
        assert!(!book.evidence.as_ref().unwrap().contains("DO_NOT_SEND"));
        db.classification_dispatch(&mut run, &book).await.unwrap();
        let value = if book.product_id == "2" {
            unknown(kind)
        } else {
            known(kind)
        };
        db.context_commit(
            &mut run,
            &book,
            Some(&kind.decode(response(value)).unwrap()),
            None,
        )
        .await
        .unwrap();
        totals_match(db, &run).await;
    }
    db.finish_analysis(&mut run, "completed", "").await.unwrap();
}

#[tokio::test]
async fn all_four_analyses_use_positive_dnd_scope_and_preserve_results_across_restart() {
    let (_temp, paths, db) = seed().await;
    let before = canonical(&db).await;
    let mut run = db.begin_settings(false).await.unwrap();
    assert_eq!(run.total, 4);
    let mut after = 0;
    while let Some(book) = db.classification_next_after(&run, after).await.unwrap() {
        after = book.document;
        let names = match book.product_id.as_str() {
            "1" => vec!["al_qadim", "ravenloft"],
            "2" => vec!["any"],
            "3" => vec![],
            _ => vec!["eberron"],
        };
        db.classification_dispatch(&mut run, &book).await.unwrap();
        db.classification_commit(&mut run, &book, Some(&settings(&names)), None)
            .await
            .unwrap();
        totals_match(&db, &run).await;
    }
    assert_eq!(
        run.overview.as_ref().unwrap().analyses[dnd_settings::ALGORITHM].counts["forgotten_realms"],
        1
    );
    db.finish_analysis(&mut run, "completed", "").await.unwrap();
    for kind in Kind::ALL {
        fill(&db, kind, false).await;
        assert!(
            !stored::saved(&db.pool, "drivethru", "2", kind)
                .await
                .unwrap()
                .unwrap()
                .data
                .known()
        );
        assert!(
            stored::saved(&db.pool, "drivethru", "5", kind)
                .await
                .unwrap()
                .is_none()
        );
    }
    assert_eq!(canonical(&db).await, before);
    let identity = db.catalogue;
    db.release().await.unwrap();
    db.pool.close().await;
    let db = Catalogue::connect(&paths).await.unwrap();
    assert_eq!(db.catalogue, identity);
    assert_eq!(canonical(&db).await, before);
    let mut roles = db.begin_settings(false).await.unwrap();
    assert_eq!(roles.total, 0);
    db.finish_analysis(&mut roles, "completed", "")
        .await
        .unwrap();
    for kind in Kind::ALL {
        let mut run = db.begin_context(kind, false).await.unwrap();
        assert_eq!(run.total, 0);
        assert_eq!(
            run.overview.as_ref().unwrap().analyses[kind.algorithm()].prerequisite_pending,
            1
        );
        db.finish_analysis(&mut run, "completed", "").await.unwrap();
        fill(&db, kind, true).await;
    }
    let saved = classification::saved_settings(&db.pool, "drivethru", "2")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.category, "any");
    assert_eq!(saved.algorithm, dnd_settings::ALGORITHM);
    db.release().await.unwrap();
}

#[tokio::test]
async fn changed_prerequisite_or_source_invalidates_dependents_but_other_results_do_not() {
    let (_temp, _paths, db) = seed().await;
    for kind in Kind::ALL {
        fill(&db, kind, false).await;
    }
    let mut run = db.begin_settings(false).await.unwrap();
    let book = db.classification_next(&run).await.unwrap().unwrap();
    db.classification_commit(&mut run, &book, Some(&settings(&["al_qadim"])), None)
        .await
        .unwrap();
    db.finish_analysis(&mut run, "completed", "").await.unwrap();
    sqlx::query("UPDATE products SET record=json_set(record,'$.rating',2,'$.min_players',4,'$.price','9.99')").execute(&db.pool).await.unwrap();
    sqlx::query("UPDATE book_classifications SET category='dnd_and_others' WHERE system='dnd' AND product_id='1'").execute(&db.pool).await.unwrap();
    for kind in Kind::ALL {
        assert!(
            stored::saved(&db.pool, "drivethru", "1", kind)
                .await
                .unwrap()
                .is_some()
        );
    }
    let mut rerun = db.begin_settings(true).await.unwrap();
    db.classification_commit(&mut rerun, &book, Some(&settings(&["any"])), None)
        .await
        .unwrap();
    totals_match(&db, &rerun).await;
    db.finish_analysis(&mut rerun, "completed", "")
        .await
        .unwrap();
    for kind in Kind::ALL {
        assert!(
            stored::saved(&db.pool, "drivethru", "1", kind)
                .await
                .unwrap()
                .is_some()
        );
    }
    // Exercise the real commit delta as prerequisite triggers remove all four results.
    let mut dnd_run = db.begin_dnd(false, true).await.unwrap();
    let probabilities = ["dnd_only", "dnd_and_others", "no"]
        .map(|value| json!({"value":value,"probability":if value=="no" {1} else {0}}));
    let no: Decision = serde_json::from_value(json!({"model":decisions::MODEL,"answers":[{"type":"choice","name":"dnd","choice":"no","confidence":1,"probabilities":probabilities}],"usage":{"input_tokens":1,"output_tokens":0,"total_tokens":1,"input_tokens_details":{"cached_tokens":0,"cache_write_tokens":0}}})).unwrap();
    db.classification_commit(&mut dnd_run, &book, Some(&no), None)
        .await
        .unwrap();
    totals_match(&db, &dnd_run).await;
    for algorithm in Kind::ALL
        .map(Kind::algorithm)
        .into_iter()
        .chain([dnd_settings::ALGORITHM])
    {
        assert_eq!(
            dnd_run.overview.as_ref().unwrap().analyses[algorithm].eligible,
            3
        );
    }
    db.finish_analysis(&mut dnd_run, "completed", "")
        .await
        .unwrap();
    sqlx::query("UPDATE products SET description_text='Changed evidence' WHERE product_id='2'")
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE book_classifications SET algorithm='old-dnd-policy' WHERE system='dnd' AND product_id='3'").execute(&db.pool).await.unwrap();
    sqlx::query("DELETE FROM products WHERE product_id='4'")
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(
        classification::saved_settings(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM dnd_context")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        0
    );
    db.release().await.unwrap();
}

#[tokio::test]
async fn failures_roll_back_live_totals_and_cancelled_gaps_survive_takeover() {
    let (_temp, paths, db) = seed().await;
    fill(&db, Kind::Levels, false).await;
    let mut run = db.begin_context(Kind::Levels, true).await.unwrap();
    let first = db.context_next_after(&run, 0).await.unwrap().unwrap();
    let second = db
        .context_next_after(&run, first.document)
        .await
        .unwrap()
        .unwrap();
    db.classification_dispatch(&mut run, &first).await.unwrap();
    db.classification_dispatch(&mut run, &second).await.unwrap();
    db.classification_retry_wait(&run, &first, "Network connection failed")
        .await
        .unwrap();
    let answer = Kind::Levels
        .decode(response(json!({"min_level":3,"max_level":5})))
        .unwrap();
    sqlx::raw_sql("CREATE TRIGGER fail_context BEFORE UPDATE ON dnd_context BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").execute(&db.pool).await.unwrap();
    assert!(
        db.context_commit(&mut run, &second, Some(&answer), None)
            .await
            .is_err()
    );
    assert_eq!(run.processed, 0);
    totals_match(&db, &run).await;
    sqlx::raw_sql("DROP TRIGGER fail_context")
        .execute(&db.pool)
        .await
        .unwrap();
    db.context_commit(&mut run, &second, Some(&answer), None)
        .await
        .unwrap();
    totals_match(&db, &run).await;
    assert!(
        db.context_commit(&mut run, &second, Some(&answer), None)
            .await
            .is_err()
    );
    db.release().await.unwrap();
    let successor = Catalogue::connect(&paths).await.unwrap();
    assert!(
        db.context_commit(&mut run, &first, Some(&answer), None)
            .await
            .is_err()
    );
    let attempt = sqlx::query(
        "SELECT document,state FROM classification_attempts WHERE run_id=? AND document=?",
    )
    .bind(run.id)
    .bind(first.document)
    .fetch_one(&successor.pool)
    .await
    .unwrap();
    assert_eq!(attempt.get::<String, _>("state"), "interrupted");
    let mut next = successor.begin_context(Kind::Levels, true).await.unwrap();
    assert_eq!(
        successor
            .context_next_after(&next, 0)
            .await
            .unwrap()
            .unwrap()
            .document,
        first.document
    );
    let mut refused = response(unknown(Kind::Levels));
    refused["output"][0]["content"][0] = json!({"type":"refusal"});
    successor
        .context_commit(
            &mut next,
            &first,
            Some(&Kind::Levels.decode(refused).unwrap()),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        stored::saved(&successor.pool, "drivethru", "1", Kind::Levels)
            .await
            .unwrap()
            .unwrap()
            .data,
        Data::Levels {
            min_level: Some(21),
            max_level: Some(36)
        }
    );
    totals_match(&successor, &next).await;
    successor
        .finish_analysis(&mut next, "cancelled", "")
        .await
        .unwrap();
    successor.release().await.unwrap();
    db.pool.close().await;
}

#[tokio::test]
async fn schema_ten_upgrade_preserves_captures_and_previous_analyses_without_requests() {
    let (_temp, paths, db) = seed().await;
    let identity = db.catalogue;
    let before = canonical(&db).await;
    let mut score = db.begin_analysis().await.unwrap();
    db.analysis_batch(&mut score).await.unwrap();
    sqlx::raw_sql(include_str!("fixtures/schema_before_dnd_context.sql"))
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE catalogue SET version=10")
        .execute(&db.pool)
        .await
        .unwrap();
    db.release().await.unwrap();
    db.pool.close().await;
    let db = Catalogue::connect(&paths).await.unwrap();
    assert_eq!(identity, db.catalogue);
    assert_eq!(canonical(&db).await, before);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM relevance_scores")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        6
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM dnd_context")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        0
    );
    let totals = overview::read(&mut db.pool.acquire().await.unwrap())
        .await
        .unwrap();
    for kind in Kind::ALL {
        assert_eq!(totals.analyses[kind.algorithm()].eligible, 4);
        assert_eq!(totals.analyses[kind.algorithm()].completed, 0);
    }
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT parent_key FROM analysis_groups WHERE system='dnd_settings' AND key='al_qadim'"
        )
        .fetch_one(&db.pool)
        .await
        .unwrap(),
        "forgotten_realms"
    );
    db.release().await.unwrap();
}
