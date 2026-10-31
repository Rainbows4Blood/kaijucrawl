//! Family-specific totals: narrow indexed aggregates and bounded per-book deltas.
use crate::{analysis::Coverage, domain::Result, warhammer::Family, warhammer_context::Kind};
use sqlx::{QueryBuilder, Row, Sqlite, SqliteConnection};
use std::collections::BTreeMap;

fn scope(q: &mut QueryBuilder<Sqlite>, family: Family, book: Option<(&str, &str)>) {
    q.push(" WHERE e.system=").push_bind(family.key());
    if let Some((target, id)) = book {
        q.push(" AND e.target=")
            .push_bind(target)
            .push(" AND e.product_id=")
            .push_bind(id);
    }
}
async fn totals(
    conn: &mut SqliteConnection,
    book: Option<(&str, &str)>,
) -> Result<BTreeMap<String, Coverage>> {
    let mut totals = BTreeMap::new();
    for family in Family::ALL {
        let mut q = QueryBuilder::new(
            "SELECT c.category,count(*) AS n FROM warhammer_scope e LEFT JOIN book_classifications c ON c.target=e.target AND c.product_id=e.product_id AND c.system=e.system AND c.algorithm=",
        );
        q.push_bind(family.algorithm());
        scope(&mut q, family, book);
        q.push(" GROUP BY c.category");
        let mut games = Coverage::default();
        let mut confirmed = 0;
        for r in q.build().fetch_all(&mut *conn).await? {
            let n: i64 = r.get("n");
            games.eligible += n;
            if let Some(category) = r.get::<Option<String>, _>("category") {
                games.completed += n;
                if matches!(category.as_str(), "identified" | "unknown") {
                    confirmed += n;
                }
                games.counts.insert(category, n);
            }
        }
        let mut q = QueryBuilder::new(
            "SELECT m.group_key,count(*) AS n FROM warhammer_scope e CROSS JOIN book_classifications c ON c.target=e.target AND c.product_id=e.product_id AND c.system=e.system CROSS JOIN book_group_memberships m ON m.target=c.target AND m.product_id=c.product_id AND m.system=c.system",
        );
        scope(&mut q, family, book);
        q.push(" AND c.algorithm=").push_bind(family.algorithm()).push(" AND m.group_key NOT IN ('root','identified','unknown','uncertain','not_applicable') GROUP BY m.group_key");
        for r in q.build().fetch_all(&mut *conn).await? {
            games.counts.insert(r.get("group_key"), r.get("n"));
        }
        for kind in Kind::ALL.into_iter().filter(|k| k.family() == family) {
            let mut q = QueryBuilder::new(
                "SELECT count(*) AS done,coalesce(sum(coalesce(json_array_length(p.locations),0)>0 OR p.in_universe_date IS NOT NULL),0) AS known FROM warhammer_scope e CROSS JOIN book_classifications c ON c.target=e.target AND c.product_id=e.product_id AND c.system=e.system CROSS JOIN warhammer_context p ON p.target=c.target AND p.product_id=c.product_id AND p.system=c.system",
            );
            scope(&mut q, family, book);
            q.push(" AND c.algorithm=")
                .push_bind(family.algorithm())
                .push(" AND c.category IN ('identified','unknown') AND p.kind=")
                .push_bind(kind.field())
                .push(" AND p.algorithm=")
                .push_bind(kind.algorithm());
            let r = q.build().fetch_one(&mut *conn).await?;
            let done: i64 = r.get("done");
            let known: i64 = r.get("known");
            totals.insert(
                kind.algorithm().into(),
                Coverage {
                    eligible: confirmed,
                    completed: done,
                    prerequisite_pending: games.eligible - games.completed,
                    counts: [("known".into(), known), ("unknown".into(), done - known)].into(),
                },
            );
        }
        totals.insert(family.algorithm().into(), games);
    }
    Ok(totals)
}
pub(super) async fn coverage(conn: &mut SqliteConnection) -> Result<BTreeMap<String, Coverage>> {
    totals(conn, None).await
}
pub(super) async fn book_coverage(
    conn: &mut SqliteConnection,
    target: &str,
    id: &str,
) -> Result<BTreeMap<String, Coverage>> {
    totals(conn, Some((target, id))).await
}
