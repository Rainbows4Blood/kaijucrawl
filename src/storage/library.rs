//! Indexed game membership reads; no projection, archive IO or analysis on navigation.
use crate::{
    domain::{Error, Result},
    library::*,
    telemetry,
};
use sqlx::{QueryBuilder, Row, Sqlite, SqliteConnection, sqlite::SqliteRow};

fn source(q: &mut QueryBuilder<Sqlite>, game: Game, context: bool) {
    q.push(" FROM book_group_memberships m INDEXED BY group_members CROSS JOIN products p ON p.target=m.target AND p.product_id=m.product_id CROSS JOIN book_classifications c ON c.target=m.target AND c.product_id=m.product_id AND c.system=m.system CROSS JOIN warhammer_scope e ON e.target=m.target AND e.product_id=m.product_id AND e.system=m.system");
    if context {
        q.push(" LEFT JOIN book_classifications r ON r.target=p.target AND r.product_id=p.product_id AND r.system='book_role' AND r.algorithm=").push_bind(crate::classification::ALGORITHM);
        q.push(" LEFT JOIN warhammer_context d ON d.target=p.target AND d.product_id=p.product_id AND d.system=m.system AND d.kind='date' AND d.algorithm=").push_bind(crate::warhammer_context::Kind::FortyKDate.algorithm());
        q.push(" LEFT JOIN warhammer_context l ON l.target=p.target AND l.product_id=p.product_id AND l.system=m.system AND l.kind='locations' AND l.algorithm=").push_bind(crate::warhammer_context::Kind::FortyKLocations.algorithm());
    }
    q.push(" WHERE m.system='warhammer_40k' AND m.group_key=")
        .push_bind(game.key())
        .push(" AND c.algorithm=")
        .push_bind(crate::warhammer::Family::FortyK.algorithm())
        .push(" AND c.category='identified'");
}
// Source availability/publication values only, never capture or file-modification dates.
const COLUMNS: &str = "p.target,p.product_id,substr(p.title,1,4096) AS name,substr(coalesce(CASE WHEN json_type(p.record,'$.date_available')='text' THEN nullif(trim(json_extract(p.record,'$.date_available')),'') END,CASE WHEN json_type(p.record,'$.dates.dateAvailable')='text' THEN nullif(trim(json_extract(p.record,'$.dates.dateAvailable')),'') END,CASE WHEN json_type(p.record,'$.dates.publicationDate') IN ('text','integer') THEN nullif(trim(CAST(json_extract(p.record,'$.dates.publicationDate') AS TEXT)),'') END),1,4096) AS release_date,CASE r.category WHEN 'core_rulebook' THEN 'Core Rulebook' WHEN 'supplement' THEN 'Supplement' WHEN 'adventure' THEN 'Adventure' WHEN 'none' THEN 'None' END AS book_type,d.in_universe_date,coalesce(l.locations,'[]') AS locations,d.kind IS NOT NULL AS date_analyzed,l.kind IS NOT NULL AS locations_analyzed";
fn sort_key(sort: Sort) -> &'static str {
    match sort {
        Sort::Name => "name",
        Sort::BookType => "book_type",
        Sort::InUniverseDate => "in_universe_date",
        Sort::InUniverseLocation => {
            "(SELECT nullif(group_concat(value,'; '),'') FROM json_each(locations))"
        }
        Sort::ReleaseDate => {
            "CASE WHEN release_date GLOB '[0-9][0-9][0-9][0-9]' THEN release_date||'-01-01' WHEN release_date GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]' THEN release_date||'-01' ELSE coalesce(date(release_date),release_date) END"
        }
    }
}
fn strings(s: &str) -> Result<Vec<String>> {
    Ok(serde_json::from_str::<Vec<String>>(s)?
        .into_iter()
        .map(|v| telemetry::display(&v))
        .collect())
}
fn book(r: &SqliteRow) -> Result<Book> {
    Ok(Book {
        target: r.get("target"),
        product_id: r.get("product_id"),
        name: telemetry::display(r.get("name")),
        release_date: r
            .get::<Option<String>, _>("release_date")
            .map(|s| telemetry::display(&s)),
        book_type: r.get("book_type"),
        in_universe_date: r
            .get::<Option<String>, _>("in_universe_date")
            .map(|s| telemetry::display(&s)),
        locations: strings(r.get("locations"))?,
        date_analyzed: r.get("date_analyzed"),
        locations_analyzed: r.get("locations_analyzed"),
    })
}
pub async fn page(conn: &mut SqliteConnection, query: &Query) -> Result<Page> {
    if let Some(c) = &query.after
        && (c.game != query.game
            || c.sort != query.sort
            || c.descending != query.descending
            || c.key.as_ref().is_some_and(|s| s.len() > 65536)
            || c.target.len() > 128
            || c.product_id.len() > 128)
    {
        return Err(Error::Config(
            "This page belongs to a different game or sort; return to the first page".into(),
        ));
    }
    let mut count = QueryBuilder::new("SELECT count(*)");
    source(&mut count, query.game, false);
    let total = count.build_query_scalar().fetch_one(&mut *conn).await?;
    let mut q = QueryBuilder::new("WITH books AS (SELECT ");
    q.push(COLUMNS);
    source(&mut q, query.game, true);
    q.push("),ordered AS (SELECT *,")
        .push(sort_key(query.sort))
        .push(" AS sort_key FROM books) SELECT * FROM ordered");
    if let Some(c) = &query.after {
        q.push(" WHERE (sort_key IS NULL)>")
            .push_bind(c.key.is_none())
            .push(" OR ((sort_key IS NULL)=")
            .push_bind(c.key.is_none())
            .push(" AND (");
        if let Some(key) = &c.key {
            q.push("sort_key COLLATE NOCASE ")
                .push(if query.descending { "<" } else { ">" })
                .push(" ")
                .push_bind(key)
                .push(" OR (sort_key COLLATE NOCASE=")
                .push_bind(key)
                .push(" AND ");
        }
        q.push("(target,product_id)>(")
            .push_bind(&c.target)
            .push(",")
            .push_bind(&c.product_id)
            .push(")");
        if c.key.is_some() {
            q.push(")");
        }
        q.push("))");
    }
    q.push(" ORDER BY (sort_key IS NULL),sort_key COLLATE NOCASE ")
        .push(if query.descending { "DESC" } else { "ASC" })
        .push(",target,product_id LIMIT ")
        .push_bind((PAGE_SIZE + 1) as i64);
    let mut rows = q.build().fetch_all(&mut *conn).await?;
    let more = rows.len() > PAGE_SIZE;
    rows.truncate(PAGE_SIZE);
    let next = if more {
        rows.last().map(|r| Cursor {
            game: query.game,
            sort: query.sort,
            descending: query.descending,
            key: r.get("sort_key"),
            target: r.get("target"),
            product_id: r.get("product_id"),
        })
    } else {
        None
    };
    Ok(Page {
        books: rows.iter().map(book).collect::<Result<_>>()?,
        total,
        next,
    })
}
pub async fn details(
    conn: &mut SqliteConnection,
    game: Game,
    target: &str,
    id: &str,
) -> Result<Option<Details>> {
    if target.len() > 128 || id.len() > 128 {
        return Err(Error::Config("Invalid book identity".into()));
    }
    let mut q = QueryBuilder::new("SELECT ");
    q.push(COLUMNS).push(",substr(p.description_text,1,262144) AS description,length(p.description_text)>262144 AS truncated,substr(p.publisher,1,4096) AS publisher,p.page_count,p.fetched_at");
    for (key, column) in [
        ("authors", "authors"),
        ("rule_systems", "rules"),
        ("product_types", "types"),
    ] {
        q.push(",(SELECT json_group_array(value) FROM (SELECT substr(value,1,256) AS value FROM json_each(p.record,").push_bind(format!("$.{key}")).push(") WHERE type='text' LIMIT 100)) AS ").push(column);
    }
    q.push(",(SELECT json_group_array(group_key) FROM book_group_memberships g WHERE g.target=p.target AND g.product_id=p.product_id AND g.system=m.system AND group_key IN ('dark_heresy','dark_heresy_2e','rogue_trader','only_war','deathwatch','black_crusade','imperium_maledictum','wrath_glory')) AS games");
    source(&mut q, game, true);
    q.push(" AND p.target=")
        .push_bind(target)
        .push(" AND p.product_id=")
        .push_bind(id);
    q.build()
        .fetch_optional(conn)
        .await?
        .map(|r| {
            Ok(Details {
                book: book(&r)?,
                description: telemetry::display_limit(r.get("description"), 1024 * 1024),
                description_truncated: r.get("truncated"),
                publisher: r
                    .get::<Option<String>, _>("publisher")
                    .map(|s| telemetry::display(&s)),
                page_count: r.get("page_count"),
                authors: strings(r.get("authors"))?,
                rule_systems: strings(r.get("rules"))?,
                source_product_types: strings(r.get("types"))?,
                games: strings(r.get("games"))?,
                captured_at: r.get("fetched_at"),
            })
        })
        .transpose()
}
