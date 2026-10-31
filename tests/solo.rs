//! Independent classifications over synthetic catalogues, with no external requests.
use kaijucrawl::{
    analysis::Status,
    decisions::{self, Decision},
    domain::CATALOGUE_VERSION,
    paths::LocalPaths,
    solo::{self, Category},
    storage::{classification as stored, sqlite::Catalogue},
};
use serde_json::json;

async fn seeded(n: i64) -> (tempfile::TempDir, LocalPaths, Catalogue) {
    let temp = tempfile::tempdir().unwrap();
    let paths = LocalPaths::at(temp.path());
    let db = Catalogue::connect(&paths).await.unwrap();
    let intent = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO intents(id,target,product_id,payload,generation,finalized) VALUES(?,'drivethru','1','{}',?,1)").bind(intent).bind(db.generation).execute(&db.pool).await.unwrap();
    sqlx::query("WITH RECURSIVE n(i) AS(VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<?) INSERT INTO products(target,product_id,title,description_text,record,parser_version,record_schema_version,archive,fetched_at,last_checked_at,intent) SELECT 'drivethru',CAST(i AS TEXT),'Synthetic book '||i,'A complete fictional description',json_object('rating',4.7,'rating_count',1000,'product_types',json_array('Unreliable source taxonomy'),'min_players',99,'book_role','DO_NOT_SEND','raw_metadata','DO_NOT_SEND'),'drivethru-rust-1',2,'{}','2026-01-01','2026-01-01',? FROM n").bind(n).bind(intent).execute(&db.pool).await.unwrap();
    let mut run = db.begin_classification(false).await.unwrap();
    sqlx::query("INSERT INTO book_classifications(target,product_id,system,category,algorithm,model,confidence,probabilities,input_hash,calculated_at,run_id) SELECT target,product_id,'book_role',CASE product_id WHEN '1' THEN 'adventure' WHEN '2' THEN 'supplement' WHEN '3' THEN 'none' ELSE 'core_rulebook' END,'primary-book-role-1','gpt-6-luna',1,'[]','synthetic','2026-01-01',? FROM products WHERE CAST(product_id AS INTEGER)<=4").bind(run.id).execute(&db.pool).await.unwrap();
    sqlx::query("INSERT INTO book_group_memberships SELECT target,product_id,system,category FROM book_classifications").execute(&db.pool).await.unwrap();
    sqlx::query("INSERT INTO adventure_players(target,product_id,min_players,max_players,algorithm,model,input_hash,calculated_at,run_id) VALUES('drivethru','1',1,1,'adventure-players-1','gpt-6-luna','synthetic','2026-01-01',?)").bind(run.id).execute(&db.pool).await.unwrap();
    sqlx::query("INSERT INTO relevance_scores VALUES('drivethru','1','weighted-rating-1',4666667,4.7,1000,'2026-01-01',?)").bind(run.id).execute(&db.pool).await.unwrap();
    db.finish_analysis(&mut run, "completed", "").await.unwrap();
    (temp, paths, db)
}
fn answer(category: Category, refusal: bool) -> Decision {
    serde_json::from_value(json!({"model":decisions::MODEL,"answers":[if refusal {json!({"type":"refusal","name":"solo_play"})} else {json!({"type":"choice","name":"solo_play","choice":category.key(),"confidence":1.0,"probabilities":(["solo_adventure","gamebook","neither"].map(|v|json!({"value":v,"probability":if v==category.key(){1.0}else{0.0}})))})}],"usage":{"input_tokens":400,"input_tokens_details":{"cached_tokens":0,"cache_write_tokens":0},"output_tokens":0,"total_tokens":400}})).unwrap()
}
async fn save(db: &Catalogue, s: &mut Status, category: Category) {
    let b = db.classification_next(s).await.unwrap().unwrap();
    db.classification_dispatch(s, &b).await.unwrap();
    db.classification_commit(s, &b, Some(&answer(category, false)), None)
        .await
        .unwrap();
}
async fn canonical(db: &Catalogue) -> Vec<(String, String, String, String)> {
    sqlx::query_as("SELECT product_id,record,fetched_at,archive FROM products ORDER BY product_id")
        .fetch_all(&db.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn all_books_use_only_source_evidence_and_each_system_keeps_its_own_results() {
    let (_temp, _paths, db) = seeded(72).await;
    let before = canonical(&db).await;
    let mut s = db.begin_solo_classification(false).await.unwrap();
    assert_eq!((s.total, s.scope.as_str()), (72, "all"));
    assert!(s.query.is_none());
    let mut after = 0;
    while let Some(b) = db.classification_next_after(&s, after).await.unwrap() {
        after = b.document;
        let evidence: serde_json::Value =
            serde_json::from_str(b.evidence.as_ref().unwrap()).unwrap();
        for key in [
            "book_role",
            "min_players",
            "max_players",
            "relevance_score",
            "raw_metadata",
            "solo_play",
        ] {
            assert!(evidence.get(key).is_none());
        }
        assert!(!evidence.to_string().contains("DO_NOT_SEND"));
        assert_eq!(
            evidence["source_product_types"],
            json!(["Unreliable source taxonomy"])
        );
        let request = solo::request(evidence);
        request.validate().unwrap();
        let a = answer(
            match b.document % 3 {
                0 => Category::Neither,
                1 => Category::SoloAdventure,
                _ => Category::Gamebook,
            },
            false,
        );
        db.classification_dispatch(&mut s, &b).await.unwrap();
        db.classification_commit(&mut s, &b, Some(&a), None)
            .await
            .unwrap();
        assert!(
            db.classification_commit(&mut s, &b, Some(&a), None)
                .await
                .is_err()
        );
    }
    let p = s.classification.as_ref().unwrap();
    assert_eq!(
        (
            s.scored,
            p.solo_adventure,
            p.gamebook,
            p.neither,
            p.requests
        ),
        (72, 24, 24, 24, 72)
    );
    assert_eq!(p.adventure, 0);
    db.finish_analysis(&mut s, "completed", "").await.unwrap();
    assert_eq!(canonical(&db).await, before);
    let mut again = db.begin_solo_classification(false).await.unwrap();
    assert_eq!(again.total, 0);
    assert_eq!(
        again.classification.as_ref().unwrap().already_classified,
        72
    );
    db.finish_analysis(&mut again, "completed", "")
        .await
        .unwrap();
    assert_eq!(
        stored::saved(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .unwrap()
            .category,
        kaijucrawl::classification::Category::Adventure
    );
    assert_eq!(
        kaijucrawl::storage::players::saved(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .unwrap()
            .counts
            .min_players,
        Some(1)
    );
    assert!(
        kaijucrawl::storage::analysis::score(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        stored::saved_solo(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .unwrap()
            .category,
        Category::SoloAdventure
    );
    let totals = kaijucrawl::storage::overview::read(&mut db.pool.acquire().await.unwrap())
        .await
        .unwrap();
    for category in ["solo_adventure", "gamebook", "neither"] {
        assert_eq!(totals.analyses[solo::ALGORITHM].counts[category], 24);
    }
    let mut repeat = db.begin_solo_classification(true).await.unwrap();
    save(&db, &mut repeat, Category::Neither).await;
    assert_eq!(
        stored::saved_solo(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .unwrap()
            .category,
        Category::Neither
    );
    db.finish_analysis(&mut repeat, "cancelled", "")
        .await
        .unwrap();
    db.release().await.unwrap();
}

#[tokio::test]
async fn migration_preserves_prior_analyses_and_invalidation_depends_only_on_source() {
    let (_temp, paths, db) = seeded(5).await;
    let identity = db.catalogue;
    let before = canonical(&db).await;
    sqlx::raw_sql(include_str!("fixtures/schema_before_solo.sql"))
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE catalogue SET version=7")
        .execute(&db.pool)
        .await
        .unwrap();
    db.release().await.unwrap();
    let db = Catalogue::connect(&paths).await.unwrap();
    assert_eq!(db.catalogue, identity);
    assert_eq!(canonical(&db).await, before);
    assert_eq!(
        sqlx::query_scalar::<_, i32>("SELECT version FROM catalogue")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        CATALOGUE_VERSION
    );
    assert!(
        stored::saved(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        kaijucrawl::storage::players::saved(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        kaijucrawl::storage::analysis::score(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        stored::saved_solo(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .is_none()
    );
    let mut s = db.begin_solo_classification(false).await.unwrap();
    save(&db, &mut s, Category::Gamebook).await;
    save(&db, &mut s, Category::Neither).await;
    db.finish_analysis(&mut s, "cancelled", "").await.unwrap();
    sqlx::query("UPDATE adventure_players SET min_players=2,max_players=3")
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE book_classifications SET category='supplement' WHERE system='book_role' AND product_id='1'").execute(&db.pool).await.unwrap();
    assert_eq!(
        stored::saved_solo(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .unwrap()
            .category,
        Category::Gamebook
    );
    sqlx::query("UPDATE products SET record=json_set(record,'$.price','9.99','$.rating',3,'$.min_players',77)").execute(&db.pool).await.unwrap();
    assert!(
        stored::saved_solo(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .is_some()
    );
    sqlx::query(
        "UPDATE products SET description_text='Changed original description' WHERE product_id='1'",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    assert!(
        stored::saved_solo(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        stored::saved(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .is_none()
    );
    sqlx::query("DELETE FROM products WHERE product_id='2'")
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(
        stored::saved_solo(&db.pool, "drivethru", "2")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM book_group_memberships WHERE system='solo_play'"
        )
        .fetch_one(&db.pool)
        .await
        .unwrap(),
        0
    );
    db.release().await.unwrap();
}

#[tokio::test]
async fn refusals_rollback_and_recovery_preserve_good_results_and_unfinished_gaps() {
    let (_temp, paths, db) = seeded(5).await;
    let mut s = db.begin_solo_classification(false).await.unwrap();
    let first = db.classification_next_after(&s, 0).await.unwrap().unwrap();
    let second = db
        .classification_next_after(&s, first.document)
        .await
        .unwrap()
        .unwrap();
    db.classification_dispatch(&mut s, &first).await.unwrap();
    db.classification_dispatch(&mut s, &second).await.unwrap();
    db.classification_retry_wait(&s, &first, "Network connection failed")
        .await
        .unwrap();
    db.classification_commit(
        &mut s,
        &second,
        Some(&answer(Category::Neither, false)),
        None,
    )
    .await
    .unwrap();
    db.release().await.unwrap();
    let next = Catalogue::connect(&paths).await.unwrap();
    assert!(db.classification_redispatch(&mut s, &first).await.is_err());
    let mut resumed = next.begin_solo_classification(false).await.unwrap();
    assert_eq!(resumed.total, 4);
    assert_eq!(
        next.classification_next(&resumed)
            .await
            .unwrap()
            .unwrap()
            .document,
        first.document
    );
    let third = next
        .classification_next_after(&resumed, second.document)
        .await
        .unwrap()
        .unwrap();
    next.classification_dispatch(&mut resumed, &third)
        .await
        .unwrap();
    sqlx::raw_sql("CREATE TRIGGER fail_solo BEFORE UPDATE ON analysis_runs WHEN new.processed>old.processed BEGIN SELECT RAISE(ABORT,'synthetic'); END").execute(&next.pool).await.unwrap();
    assert!(
        next.classification_commit(
            &mut resumed,
            &third,
            Some(&answer(Category::Gamebook, false)),
            None
        )
        .await
        .is_err()
    );
    assert_eq!(resumed.processed, 0);
    assert!(
        stored::saved_solo(&next.pool, "drivethru", &third.product_id)
            .await
            .unwrap()
            .is_none()
    );
    sqlx::raw_sql("DROP TRIGGER fail_solo")
        .execute(&next.pool)
        .await
        .unwrap();
    next.finish_analysis(&mut resumed, "cancelled", "")
        .await
        .unwrap();
    let mut forced = next.begin_solo_classification(true).await.unwrap();
    let book = next
        .classification_next_after(&forced, first.document)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(book.document, second.document);
    next.classification_dispatch(&mut forced, &book)
        .await
        .unwrap();
    next.classification_commit(
        &mut forced,
        &book,
        Some(&answer(Category::Neither, true)),
        None,
    )
    .await
    .unwrap();
    assert_eq!((forced.scored, forced.skipped), (0, 1));
    assert_eq!(forced.classification.as_ref().unwrap().neither, 0);
    assert_eq!(
        stored::saved_solo(&next.pool, "drivethru", &second.product_id)
            .await
            .unwrap()
            .unwrap()
            .category,
        Category::Neither
    );
    next.finish_analysis(&mut forced, "cancelled", "")
        .await
        .unwrap();
    next.release().await.unwrap();
}
