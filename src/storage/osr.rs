//! Indexed OSR eligibility and bounded committed coverage deltas.
use crate::{
    analysis::Coverage,
    domain::Result,
    osr::{ALGORITHM, SYSTEM},
};
use sqlx::{Row, SqliteConnection};
use std::collections::BTreeMap;

pub(super) async fn book_coverage(
    conn: &mut SqliteConnection,
    target: &str,
    id: &str,
) -> Result<BTreeMap<String, Coverage>> {
    let row = sqlx::query("SELECT c.category,(SELECT json_group_array(group_key) FROM book_group_memberships m WHERE m.target=e.target AND m.product_id=e.product_id AND m.system=c.system) AS members FROM osr_scope e LEFT JOIN book_classifications c ON c.target=e.target AND c.product_id=e.product_id AND c.system=? AND c.algorithm=? WHERE e.target=? AND e.product_id=?")
        .bind(SYSTEM).bind(ALGORITHM).bind(target).bind(id).fetch_optional(&mut *conn).await?;
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
    Ok([(ALGORITHM.into(), value)].into())
}

pub(super) async fn coverage(conn: &mut SqliteConnection) -> Result<BTreeMap<String, Coverage>> {
    let rows = sqlx::query("SELECT c.category,count(*) AS n FROM osr_scope e LEFT JOIN book_classifications c ON c.target=e.target AND c.product_id=e.product_id AND c.system=? AND c.algorithm=? GROUP BY c.category")
        .bind(SYSTEM).bind(ALGORITHM).fetch_all(&mut *conn).await?;
    let mut value = Coverage::default();
    for row in rows {
        let n: i64 = row.get("n");
        value.eligible += n;
        if let Some(category) = row.get::<Option<String>, _>("category") {
            value.completed += n;
            value.counts.insert(category, n);
        }
    }
    let rows = sqlx::query("SELECT m.group_key,count(*) AS n FROM osr_scope e JOIN book_classifications c ON c.target=e.target AND c.product_id=e.product_id AND c.system=? AND c.algorithm=? JOIN book_group_memberships m ON m.target=c.target AND m.product_id=c.product_id AND m.system=c.system GROUP BY m.group_key")
        .bind(SYSTEM).bind(ALGORITHM).fetch_all(&mut *conn).await?;
    for row in rows {
        value.counts.insert(row.get("group_key"), row.get("n"));
    }
    Ok([(ALGORITHM.into(), value)].into())
}
