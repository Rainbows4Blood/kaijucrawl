//! Upgrade a populated schema 9, including an incomplete Search backfill.
//! All data is synthetic; no network or production files are accessed.
use kaijucrawl::{
    analysis, classification,
    domain::CATALOGUE_VERSION,
    paths::LocalPaths,
    storage::{analysis as stored, overview, sqlite::Catalogue},
};
use serde_json::json;
use sqlx::Row;

// Snapshot every stored field, including rowids, without normalizing their values.
async fn rows(db: &Catalogue, table: &'static str) -> Vec<Vec<String>> {
    // Audited: table names below are test constants and columns come only from
    // our synthetic migration schema, never external catalogue/input data.
    let columns = sqlx::query("SELECT name FROM pragma_table_info(?)")
        .bind(table)
        .fetch_all(&db.pool)
        .await
        .unwrap();
    let expressions = columns
        .iter()
        .map(|c| format!("quote(\"{}\")", c.get::<String, _>("name")))
        .collect::<Vec<_>>()
        .join(",");
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {expressions} FROM {table}"
    )))
    .fetch_all(&db.pool)
    .await
    .unwrap()
    .into_iter()
    .map(|r| (0..columns.len()).map(|i| r.get::<String, _>(i)).collect())
    .collect()
}

#[tokio::test]
async fn retires_only_search_and_continues_analyses_with_original_rowids() {
    let temp = tempfile::tempdir().unwrap();
    let paths = LocalPaths::at(temp.path());
    let db = Catalogue::connect(&paths).await.unwrap();
    let identity = db.catalogue;
    let intent = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO intents(id,target,product_id,payload,generation,finalized) VALUES(?,'drivethru','1','{}',?,1)")
        .bind(intent).bind(db.generation).execute(&db.pool).await.unwrap();
    for (rowid, id) in [(3, "1"), (17, "2"), (101, "3")] {
        sqlx::query("INSERT INTO products(rowid,target,product_id,title,description_text,price,record,parser_version,record_schema_version,archive,fetched_at,last_checked_at,intent) VALUES(?,'drivethru',?,'Synthetic book','Complete fictional description','12.34567890123456789',?,'drivethru-rust-1',2,'{}','2026-01-01','2026-01-01',?)")
            .bind(rowid).bind(id).bind(json!({"rating":4.7,"rating_count":1000,"authors":["Synthetic Author"],"raw_metadata":{"preserved":true}}).to_string()).bind(intent).execute(&db.pool).await.unwrap();
    }
    let mut weighted = db.begin_analysis().await.unwrap();
    assert!(db.analysis_batch(&mut weighted).await.unwrap());
    for (id, system, category, algorithm, probabilities) in [
        (
            "1",
            "book_role",
            "adventure",
            classification::ALGORITHM,
            json!([]),
        ),
        (
            "2",
            "book_role",
            "supplement",
            classification::ALGORITHM,
            json!([]),
        ),
        (
            "2",
            "solo_play",
            "gamebook",
            kaijucrawl::solo::ALGORITHM,
            json!([]),
        ),
        (
            "1",
            "dnd",
            "dnd_only",
            kaijucrawl::dnd::ALGORITHM,
            json!([]),
        ),
        (
            "1",
            "dnd_editions",
            "identified",
            kaijucrawl::dnd::EDITIONS_ALGORITHM,
            json!([{"edition":"bx","status":"supported","confidence":1}]),
        ),
    ] {
        sqlx::query("INSERT INTO book_classifications VALUES('drivethru',?,?,?,?, 'gpt-6-luna',1,?,'synthetic','2026-01-02',?)")
            .bind(id).bind(system).bind(category).bind(algorithm).bind(probabilities.to_string()).bind(weighted.id).execute(&db.pool).await.unwrap();
    }
    sqlx::raw_sql("INSERT INTO book_group_memberships SELECT target,product_id,system,category FROM book_classifications; INSERT INTO book_group_memberships VALUES('drivethru','1','dnd_editions','bx');")
        .execute(&db.pool).await.unwrap();
    sqlx::query("INSERT INTO adventure_players(target,product_id,min_players,max_players,algorithm,model,input_hash,calculated_at,run_id) VALUES('drivethru','1',3,NULL,'adventure-players-1','gpt-6-luna','synthetic','2026-01-02',?)")
        .bind(weighted.id).execute(&db.pool).await.unwrap();
    // Old query JSON is historical evidence, including fields no current UI understands.
    let historical = json!({"text":"old selection","sort":"price","retired_filter":{"value":true}});
    sqlx::query("UPDATE analysis_runs SET scope='filtered',query=? WHERE id=?")
        .bind(historical.to_string())
        .bind(weighted.id)
        .execute(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        stored::latest(&db.pool).await.unwrap().query,
        Some(historical)
    );
    let mut pending = db.begin_classification(false).await.unwrap();
    let book = db.classification_next(&pending).await.unwrap().unwrap();
    assert_eq!(book.document, 101);
    db.classification_dispatch(&mut pending, &book)
        .await
        .unwrap();
    let totals = overview::read(&mut db.pool.acquire().await.unwrap())
        .await
        .unwrap();
    sqlx::raw_sql(include_str!("fixtures/schema_before_dnd_context.sql"))
        .execute(&db.pool)
        .await
        .unwrap();
    // Rebuild the real v9 Search objects, with only part of the catalogue indexed.
    sqlx::raw_sql(include_str!("../migrations/004_search.sql"))
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::raw_sql("INSERT INTO search_documents SELECT * FROM search_source WHERE id<=17; UPDATE search_progress SET last_id=17,ready=0; UPDATE catalogue SET version=9;").execute(&db.pool).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM search_fts WHERE search_fts MATCH 'fictional'"
        )
        .fetch_one(&db.pool)
        .await
        .unwrap(),
        2
    );
    let preserved = [
        "products",
        "intents",
        "jobs",
        "analysis_groups",
        "book_group_memberships",
        "book_classifications",
        "relevance_scores",
        "adventure_players",
        "analysis_revision",
    ];
    let mut before = Vec::new();
    for table in preserved {
        before.push(rows(&db, table).await);
    }

    let history: String = sqlx::query_scalar("SELECT query FROM analysis_runs WHERE id=?")
        .bind(weighted.id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    db.release().await.unwrap();
    db.pool.close().await;

    let db = Catalogue::connect(&paths).await.unwrap();
    assert_eq!(db.catalogue, identity);
    assert_eq!(
        sqlx::query_scalar::<_, i32>("SELECT version FROM catalogue")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        CATALOGUE_VERSION
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM sqlite_master WHERE name GLOB 'search_*'"
        )
        .fetch_one(&db.pool)
        .await
        .unwrap(),
        0
    );
    for (table, expected) in preserved.into_iter().zip(before) {
        let after = rows(&db, table).await;
        if table == "analysis_groups" {
            for row in expected {
                assert!(after.contains(&row));
            }
        } else {
            assert_eq!(after, expected, "{table}");
        }
    }
    let ids: Vec<i64> = sqlx::query_scalar("SELECT rowid FROM products ORDER BY rowid")
        .fetch_all(&db.pool)
        .await
        .unwrap();
    assert_eq!(ids, vec![3, 17, 101]);
    assert_eq!(
        serde_json::to_value(
            overview::read(&mut db.pool.acquire().await.unwrap())
                .await
                .unwrap()
        )
        .unwrap(),
        serde_json::to_value(totals).unwrap()
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT query FROM analysis_runs WHERE id=?")
            .bind(weighted.id)
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        history
    );
    assert_eq!(stored::latest(&db.pool).await.unwrap().state, "interrupted");
    let attempt = sqlx::query("SELECT document,state FROM classification_attempts WHERE run_id=?")
        .bind(pending.id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(attempt.get::<i64, _>("document"), 101);
    assert_eq!(attempt.get::<String, _>("state"), "interrupted");

    // Explicit reruns skip saved results and include the unindexed book/gaps.
    for kind in ["role", "solo", "dnd", "editions", "players", "weighted"] {
        let mut run = match kind {
            "role" => db.begin_classification(false).await.unwrap(),
            "solo" => db.begin_solo_classification(false).await.unwrap(),
            "dnd" => db.begin_dnd(false, false).await.unwrap(),
            "editions" => db.begin_dnd(true, false).await.unwrap(),
            "players" => db.begin_players(false).await.unwrap(),
            _ => db.begin_analysis().await.unwrap(),
        };
        assert!(run.query.is_none());
        match kind {
            "role" => {
                assert_eq!(run.total, 1);
                assert_eq!(
                    db.classification_next(&run)
                        .await
                        .unwrap()
                        .unwrap()
                        .document,
                    101
                );
            }
            "solo" | "dnd" => assert_eq!(run.total, 2),
            "editions" | "players" => assert_eq!(run.total, 0),
            _ => {
                assert_eq!(run.total, 3);
                assert!(db.analysis_batch(&mut run).await.unwrap());
                assert_eq!(run.scored, 3);
            }
        }
        db.finish_analysis(&mut run, "completed", "").await.unwrap();
    }
    // Canonical mutations no longer maintain search objects; analysis invalidation stays active.
    sqlx::query(
        "UPDATE products SET description_text='Changed fictional evidence' WHERE product_id='1'",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    assert!(
        kaijucrawl::storage::players::saved(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        kaijucrawl::storage::classification::saved_dnd(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        stored::score(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .is_some()
    );
    sqlx::query("DELETE FROM products WHERE product_id='3'")
        .execute(&db.pool)
        .await
        .unwrap();
    let mut fresh = db.begin_analysis().await.unwrap();
    assert_eq!(fresh.total, 2);
    assert_eq!(fresh.algorithm, analysis::ALGORITHM);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM sqlite_master WHERE name GLOB 'search_*'"
        )
        .fetch_one(&db.pool)
        .await
        .unwrap(),
        0
    );
    db.finish_analysis(&mut fresh, "cancelled", "")
        .await
        .unwrap();
    db.release().await.unwrap();
    db.pool.close().await;
}
