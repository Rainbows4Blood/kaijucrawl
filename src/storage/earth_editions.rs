//! Bounded per-book deltas and cached aggregates for Rule System edition groups.
use crate::{analysis::Coverage, domain::Result, earth_editions::Kind};
use sqlx::{Row, SqliteConnection};
use std::collections::BTreeMap;

pub(super) async fn book_coverage(
    conn: &mut SqliteConnection,
    target: &str,
    id: &str,
) -> Result<BTreeMap<String, Coverage>> {
    let mut totals = BTreeMap::new();
    for kind in Kind::ALL {
        let row = sqlx::query("SELECT c.category,(SELECT json_group_array(group_key) FROM book_group_memberships m WHERE m.target=e.target AND m.product_id=e.product_id AND m.system=c.system) AS members FROM earth_scope e LEFT JOIN book_classifications c ON c.target=e.target AND c.product_id=e.product_id AND c.system=? AND c.algorithm=? WHERE e.target=? AND e.product_id=? AND e.system=?")
            .bind(kind.system()).bind(kind.algorithm()).bind(target).bind(id).bind(kind.scope()).fetch_optional(&mut *conn).await?;
        let mut value = Coverage {
            eligible: i64::from(row.is_some()),
            ..Default::default()
        };
        if let Some(row) = row
            && let Some(category) = row.get::<Option<String>, _>("category")
        {
            value.completed = 1;
            value.counts.insert(category, 1);
            for member in serde_json::from_str::<Vec<String>>(row.get("members"))? {
                value.counts.insert(member, 1);
            }
        }
        totals.insert(kind.algorithm().into(), value);
    }
    Ok(totals)
}

pub(super) async fn coverage(conn: &mut SqliteConnection) -> Result<BTreeMap<String, Coverage>> {
    let mut totals = BTreeMap::new();
    for kind in Kind::ALL {
        let rows = sqlx::query("SELECT c.category,count(*) AS n FROM earth_scope e LEFT JOIN book_classifications c ON c.target=e.target AND c.product_id=e.product_id AND c.system=? AND c.algorithm=? WHERE e.system=? GROUP BY c.category")
            .bind(kind.system()).bind(kind.algorithm()).bind(kind.scope()).fetch_all(&mut *conn).await?;
        let mut value = Coverage::default();
        for row in rows {
            let n: i64 = row.get("n");
            value.eligible += n;
            if let Some(category) = row.get::<Option<String>, _>("category") {
                value.completed += n;
                value.counts.insert(category, n);
            }
        }
        let rows = sqlx::query("SELECT m.group_key,count(*) AS n FROM earth_scope e JOIN book_classifications c ON c.target=e.target AND c.product_id=e.product_id AND c.system=? AND c.algorithm=? JOIN book_group_memberships m ON m.target=c.target AND m.product_id=c.product_id AND m.system=c.system WHERE e.system=? GROUP BY m.group_key")
            .bind(kind.system()).bind(kind.algorithm()).bind(kind.scope()).fetch_all(&mut *conn).await?;
        for row in rows {
            value.counts.insert(row.get("group_key"), row.get("n"));
        }
        totals.insert(kind.algorithm().into(), value);
    }
    Ok(totals)
}
