use kaijucrawl::{
    library::{Game, PAGE_SIZE, Query, Sort},
    paths::LocalPaths,
    storage::{library, sqlite::Catalogue},
};
use serde_json::json;
use sqlx::Row;
async fn seed() -> (tempfile::TempDir, LocalPaths, Catalogue) {
    let dir = tempfile::tempdir().unwrap();
    let paths = LocalPaths::at(dir.path());
    let db = Catalogue::connect(&paths).await.unwrap();
    let intent = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO intents(id,target,product_id,payload,generation,finalized) VALUES(?,'drivethru','1','{}',?,1)").bind(intent).bind(db.generation).execute(&db.pool).await.unwrap();
    let mut run = db.begin_classification(false).await.unwrap();
    let run_id = run.id.unwrap();
    for i in 0..113 {
        let id = format!("{i:03}");
        let known = i % 4 != 0;
        let record = json!({"rule_systems":["Warhammer 40k Roleplay"],"date_available":if known{Some(format!("202{}-02-{:02}T00:00:00Z",i%3,i%6+1))}else{None},"authors":["Synthetic Author"],"product_types":["Source Tag"],"raw_metadata":{"secret":"DO_NOT_RETURN"},"description_html":"DO_NOT_RETURN"});
        let title = if i % 2 == 0 { "Alpha <script>" } else { "beta" };
        sqlx::query("INSERT INTO products(target,product_id,title,description_text,record,parser_version,record_schema_version,archive,fetched_at,last_checked_at,intent) VALUES('drivethru',?,?,?,?,'drivethru-rust-1',2,'{}','2026-01-01','2026-01-01',?)").bind(&id).bind(title).bind(format!("{}\u{1b}[31mEnd <script>alert(1)</script>","Long saved description. ".repeat(300))).bind(record.to_string()).bind(intent).execute(&db.pool).await.unwrap();
        let category = if i == 112 { "unknown" } else { "identified" };
        sqlx::query("INSERT INTO book_classifications VALUES('drivethru',?,'warhammer_40k',?,'warhammer-40k-games-1','synthetic',1,'[]','hash','2026-01-01',?)").bind(&id).bind(category).bind(run_id).execute(&db.pool).await.unwrap();
        for group in if i == 112 {
            vec!["unknown"]
        } else if i == 111 {
            vec!["rogue_trader"]
        } else {
            vec!["dark_heresy", "rogue_trader"]
        } {
            sqlx::query(
                "INSERT INTO book_group_memberships VALUES('drivethru',?,'warhammer_40k',?)",
            )
            .bind(&id)
            .bind(group)
            .execute(&db.pool)
            .await
            .unwrap();
        }
        if known {
            sqlx::query("INSERT INTO book_classifications VALUES('drivethru',?,'book_role',?,'primary-book-role-1','synthetic',1,'[]','hash','2026-01-01',?)").bind(&id).bind(if i%2==0{"adventure"}else{"core_rulebook"}).bind(run_id).execute(&db.pool).await.unwrap();
            for (kind, algo, locations, date) in [
                (
                    "locations",
                    "warhammer-40k-locations-1",
                    Some(json!([if i % 3 == 0 { "Zeta" } else { "Alpha" }, "Sector"]).to_string()),
                    None,
                ),
                (
                    "date",
                    "warhammer-40k-date-1",
                    None,
                    Some(format!("{} M41", i % 3)),
                ),
            ] {
                sqlx::query("INSERT INTO warhammer_context VALUES('drivethru',?,'warhammer_40k',?,?,?,?, 'synthetic','hash','2026-01-01',?)").bind(&id).bind(kind).bind(locations).bind(date).bind(algo).bind(run_id).execute(&db.pool).await.unwrap();
            }
        }
    }
    db.finish_analysis(&mut run, "completed", "").await.unwrap();
    (dir, paths, db)
}
fn query(game: Game, sort: Sort, descending: bool) -> Query {
    Query {
        game,
        sort,
        descending,
        after: None,
    }
}
#[test]
fn accepts_only_named_game_tabs_and_sort_contracts() {
    for (key, _) in kaijucrawl::warhammer::Family::FortyK.games() {
        let game: Game = serde_json::from_value(json!(key)).unwrap();
        assert_eq!(game.key(), *key);
    }
    for bad in [
        "unknown",
        "uncertain",
        "not_applicable",
        "identified",
        "wfrp_1e",
        "' OR 1=1 --",
    ] {
        assert!(serde_json::from_value::<Game>(json!(bad)).is_err());
    }
    assert!(
        serde_json::from_value::<Query>(
            json!({"game":"dark_heresy","sort":"DROP TABLE products","after":null})
        )
        .is_err()
    );
}
#[tokio::test]
async fn all_sorts_are_global_stable_and_paginated_with_nulls_last_in_both_directions() {
    let (_dir, _paths, db) = seed().await;
    for sort in [
        Sort::Name,
        Sort::ReleaseDate,
        Sort::BookType,
        Sort::InUniverseDate,
        Sort::InUniverseLocation,
    ] {
        for desc in [false, true] {
            let mut q = query(Game::DarkHeresy, sort, desc);
            let mut books = Vec::new();
            loop {
                let p = library::page(&mut db.pool.acquire().await.unwrap(), &q)
                    .await
                    .unwrap();
                assert_eq!(p.total, 111);
                assert!(p.books.len() <= PAGE_SIZE);
                books.extend(p.books);
                q.after = p.next;
                if q.after.is_none() {
                    break;
                }
            }
            assert_eq!(books.len(), 111);
            assert_eq!(
                books
                    .iter()
                    .map(|b| &b.product_id)
                    .collect::<std::collections::BTreeSet<_>>()
                    .len(),
                111
            );
            let value = |b: &kaijucrawl::library::Book| match sort {
                Sort::Name => Some(b.name.to_lowercase()),
                Sort::ReleaseDate => b.release_date.as_ref().map(|d| d[..10].to_string()),
                Sort::BookType => b.book_type.as_ref().map(|v| v.to_lowercase()),
                Sort::InUniverseDate => b.in_universe_date.clone(),
                Sort::InUniverseLocation => {
                    (!b.locations.is_empty()).then(|| b.locations.join("; ").to_lowercase())
                }
            };
            for pair in books.windows(2) {
                let a = value(&pair[0]);
                let b = value(&pair[1]);
                match (a, b) {
                    (None, Some(_)) => panic!("missing value before known"),
                    (Some(_), None) => {}
                    (Some(a), Some(b)) if a != b => {
                        assert!(if desc { a > b } else { a < b }, "sort {sort:?}")
                    }
                    _ => assert!(pair[0].product_id < pair[1].product_id),
                }
            }
        }
    }
    db.release().await.unwrap();
}
#[tokio::test]
async fn current_memberships_bound_details_and_never_expose_raw_metadata_or_recompute() {
    let (_dir, _paths, db) = seed().await;
    let mut conn = db.pool.acquire().await.unwrap();
    let before: i64 = sqlx::query_scalar("SELECT total_changes()")
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    let d = library::details(&mut conn, Game::DarkHeresy, "drivethru", "001")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(d.book.book_type.as_deref(), Some("Core Rulebook"));
    assert_eq!(d.games.len(), 2);
    assert!(d.description.len() > 4096);
    assert!(!d.description.contains('\u{1b}'));
    assert!(!d.description.contains("[31m"));
    assert!(d.description.contains("<script>"));
    assert!(!serde_json::to_string(&d).unwrap().contains("DO_NOT_RETURN"));
    assert!(
        library::details(&mut conn, Game::DarkHeresy, "drivethru", "111")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        library::details(&mut conn, Game::DarkHeresy, "drivethru", "112")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        library::page(&mut conn, &query(Game::RogueTrader, Sort::Name, false))
            .await
            .unwrap()
            .total,
        112
    );
    assert_eq!(
        library::page(&mut conn, &query(Game::OnlyWar, Sort::Name, false))
            .await
            .unwrap()
            .total,
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT total_changes()")
            .fetch_one(&mut *conn)
            .await
            .unwrap(),
        before
    );
    drop(conn);
    // Capture time is not a release date; a partial publication date is preserved.
    sqlx::query("UPDATE products SET record=json_set(record,'$.dates.publicationDate','1999') WHERE product_id='000'").execute(&db.pool).await.unwrap();
    let d = library::details(
        &mut db.pool.acquire().await.unwrap(),
        Game::DarkHeresy,
        "drivethru",
        "000",
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(d.book.release_date.as_deref(), Some("1999"));
    sqlx::query("UPDATE book_classifications SET algorithm='old-version' WHERE system='warhammer_40k' AND product_id='001'").execute(&db.pool).await.unwrap();
    assert!(
        library::details(
            &mut db.pool.acquire().await.unwrap(),
            Game::DarkHeresy,
            "drivethru",
            "001"
        )
        .await
        .unwrap()
        .is_none()
    );
    let plan=sqlx::query("EXPLAIN QUERY PLAN SELECT p.title FROM book_group_memberships m INDEXED BY group_members CROSS JOIN products p ON p.target=m.target AND p.product_id=m.product_id WHERE m.system='warhammer_40k' AND m.group_key='dark_heresy'").fetch_all(&db.pool).await.unwrap();
    assert!(
        plan.iter()
            .any(|r| r.get::<String, _>("detail").contains("group_members"))
    );
    assert!(
        !plan
            .iter()
            .any(|r| r.get::<String, _>("detail").contains("SCAN p"))
    );
    db.release().await.unwrap();
}
#[tokio::test]
async fn cursors_cannot_cross_groups_or_sort_orders_and_reads_survive_release() {
    let (_dir, paths, db) = seed().await;
    let mut q = query(Game::DarkHeresy, Sort::Name, false);
    q.after = library::page(&mut db.pool.acquire().await.unwrap(), &q)
        .await
        .unwrap()
        .next;
    q.game = Game::RogueTrader;
    assert!(
        library::page(&mut db.pool.acquire().await.unwrap(), &q)
            .await
            .is_err()
    );
    q.game = Game::DarkHeresy;
    q.descending = true;
    assert!(
        library::page(&mut db.pool.acquire().await.unwrap(), &q)
            .await
            .is_err()
    );
    db.release().await.unwrap();
    let owner = Catalogue::connect(&paths).await.unwrap();
    assert_eq!(
        library::page(
            &mut db.pool.acquire().await.unwrap(),
            &query(Game::DarkHeresy, Sort::Name, false)
        )
        .await
        .unwrap()
        .total,
        111
    );
    owner.release().await.unwrap();
}
