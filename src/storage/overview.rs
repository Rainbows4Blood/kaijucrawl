//! Indexed saved-result aggregates, never model calls or catalogue-wide record reads.
use crate::{
    analysis::{Coverage, Overview},
    classification::system::System,
    domain::*,
};
use sqlx::{Row, SqliteConnection};

impl super::sqlite::Catalogue {
    /// Recheck the prerequisite after acquiring ownership, before any paid work.
    pub async fn require_discovery_complete(&self) -> Result<()> {
        let mut tx = self.transaction().await?;
        let overview = read(&mut tx).await?;
        self.commit(tx).await?;
        overview.require_discovery_complete()
    }
}

/// A bounded contribution from one book, read before/after a fenced analysis write.
/// Includes dependent results removed by prerequisite triggers. No evidence is loaded.
pub(super) async fn book(conn: &mut SqliteConnection, target: &str, id: &str) -> Result<Overview> {
    let row = sqlx::query(
        "WITH b(target,product_id) AS(VALUES(?,?)) SELECT
        (SELECT category FROM book_classifications c WHERE c.target=b.target AND c.product_id=b.product_id AND c.system='book_role' AND c.algorithm=?) AS role,
        (SELECT category FROM book_classifications c WHERE c.target=b.target AND c.product_id=b.product_id AND c.system='solo_play' AND c.algorithm=?) AS solo,
        (SELECT category FROM book_classifications c WHERE c.target=b.target AND c.product_id=b.product_id AND c.system='dnd' AND c.algorithm=?) AS dnd,
        (SELECT category FROM book_classifications c WHERE c.target=b.target AND c.product_id=b.product_id AND c.system='dnd_editions' AND c.algorithm=?) AS editions,
        (SELECT min_players IS NOT NULL OR max_players IS NOT NULL FROM adventure_players p WHERE p.target=b.target AND p.product_id=b.product_id AND p.algorithm=?) AS players,
        (SELECT json_group_array(group_key) FROM book_group_memberships m WHERE m.target=b.target AND m.product_id=b.product_id AND m.system='dnd_editions' AND group_key NOT IN ('root','identified','unknown')) AS members,
        (SELECT category FROM book_classifications c WHERE c.target=b.target AND c.product_id=b.product_id AND c.system='dnd_settings' AND c.algorithm=?) AS settings,
        (SELECT json_group_array(group_key) FROM book_group_memberships m WHERE m.target=b.target AND m.product_id=b.product_id AND m.system='dnd_settings' AND group_key NOT IN ('root','identified','unknown','any')) AS settings_members,
        (SELECT json_group_object(kind,CASE WHEN min_level IS NOT NULL OR max_level IS NOT NULL OR coalesce(json_array_length(locations),0)>0 OR in_universe_date IS NOT NULL THEN 1 ELSE 0 END) FROM dnd_context p WHERE p.target=b.target AND p.product_id=b.product_id AND ((kind='levels' AND algorithm=?) OR (kind='locations' AND algorithm=?) OR (kind='date' AND algorithm=?))) AS context
        FROM b",
    )
    .bind(target).bind(id)
    .bind(crate::classification::ALGORITHM).bind(crate::solo::ALGORITHM)
    .bind(crate::dnd::ALGORITHM).bind(crate::dnd::EDITIONS_ALGORITHM)
    .bind(crate::players::ALGORITHM)
    .bind(crate::dnd_settings::ALGORITHM)
    .bind(crate::dnd_context::Kind::Levels.algorithm()).bind(crate::dnd_context::Kind::Locations.algorithm()).bind(crate::dnd_context::Kind::Date.algorithm())
    .fetch_one(&mut *conn).await?;
    let role: Option<String> = row.get("role");
    let dnd: Option<String> = row.get("dnd");
    let adventure = role.as_deref() == Some("adventure");
    let compatible = matches!(dnd.as_deref(), Some("dnd_only" | "dnd_and_others"));
    let mut result = Overview::default();
    for (algorithm, category) in [
        (crate::classification::ALGORITHM, role.as_deref()),
        (crate::solo::ALGORITHM, row.get::<Option<&str>, _>("solo")),
        (crate::dnd::ALGORITHM, dnd.as_deref()),
        (
            crate::dnd_settings::ALGORITHM,
            row.get::<Option<&str>, _>("settings")
                .filter(|_| compatible),
        ),
        (
            crate::dnd::EDITIONS_ALGORITHM,
            row.get::<Option<&str>, _>("editions")
                .filter(|_| compatible),
        ),
    ] {
        result.analyses.insert(
            algorithm.into(),
            Coverage {
                completed: i64::from(category.is_some()),
                counts: category.map(|c| [(c.into(), 1)].into()).unwrap_or_default(),
                ..Default::default()
            },
        );
    }
    let players: Option<bool> = row.get::<Option<bool>, _>("players").filter(|_| adventure);
    result.analyses.insert(
        crate::players::ALGORITHM.into(),
        Coverage {
            eligible: i64::from(adventure),
            completed: i64::from(players.is_some()),
            prerequisite_pending: i64::from(role.is_none()),
            counts: players
                .map(|known| [(if known { "known" } else { "unknown" }.into(), 1)].into())
                .unwrap_or_default(),
        },
    );
    for (algorithm, column) in [
        (crate::dnd::EDITIONS_ALGORITHM, "members"),
        (crate::dnd_settings::ALGORITHM, "settings_members"),
    ] {
        if let Some(totals) = result.analyses.get_mut(algorithm) {
            totals.eligible = i64::from(compatible);
            totals.prerequisite_pending = i64::from(dnd.is_none());
            if totals.completed != 0 {
                for key in serde_json::from_str::<Vec<String>>(row.get(column))? {
                    totals.counts.insert(key, 1);
                }
            }
        }
    }
    let contexts: std::collections::BTreeMap<String, i64> =
        serde_json::from_str(row.get("context"))?;
    for kind in crate::dnd_context::Kind::ALL {
        let known = contexts.get(kind.key()).filter(|_| compatible);
        result.analyses.insert(
            kind.algorithm().into(),
            Coverage {
                eligible: i64::from(compatible),
                completed: i64::from(known.is_some()),
                prerequisite_pending: i64::from(dnd.is_none()),
                counts: known
                    .map(|n| [(if *n == 1 { "known" } else { "unknown" }.into(), 1)].into())
                    .unwrap_or_default(),
            },
        );
    }
    result
        .analyses
        .extend(super::earth_context::book_coverage(conn, target, id).await?);
    result
        .analyses
        .extend(super::earth_editions::book_coverage(conn, target, id).await?);
    result
        .analyses
        .extend(super::osr::book_coverage(conn, target, id).await?);
    result
        .analyses
        .extend(super::osr_levels::book_coverage(conn, target, id).await?);
    result
        .analyses
        .extend(super::warhammer::book_coverage(conn, target, id).await?);
    Ok(result)
}

/// Called on a private clone, published only after its result transaction commits.
pub(super) fn replace_book(totals: &mut Overview, before: Overview, after: Overview) {
    for (delta, sign) in [(before, -1), (after, 1)] {
        for (algorithm, change) in delta.analyses {
            let total = totals.analyses.entry(algorithm).or_default();
            total.eligible += sign * change.eligible;
            total.completed += sign * change.completed;
            total.prerequisite_pending += sign * change.prerequisite_pending;
            for (key, value) in change.counts {
                *total.counts.entry(key).or_default() += sign * value;
            }
        }
    }
}

/// Score batches already read each old score in their bounded selection query.
pub(super) fn replace_score(totals: &mut Overview, before: Option<bool>, known: bool) {
    let score = totals
        .analyses
        .entry(crate::analysis::ALGORITHM.into())
        .or_default();
    if let Some(old) = before {
        *score
            .counts
            .entry(if old { "scored" } else { "unknown" }.into())
            .or_default() -= 1;
    } else {
        score.completed += 1;
    }
    *score
        .counts
        .entry(if known { "scored" } else { "unknown" }.into())
        .or_default() += 1;
}

pub async fn read(conn: &mut SqliteConnection) -> Result<Overview> {
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM products")
        .fetch_one(&mut *conn)
        .await?;
    let mut overview = Overview {
        catalogue_total: total,
        ..Default::default()
    };
    let row = sqlx::query(
        "SELECT count(*) AS done,count(score) AS known FROM relevance_scores WHERE algorithm=?",
    )
    .bind(crate::analysis::ALGORITHM)
    .fetch_one(&mut *conn)
    .await?;
    let done: i64 = row.get("done");
    let known: i64 = row.get("known");
    overview.analyses.insert(
        crate::analysis::ALGORITHM.into(),
        Coverage {
            eligible: total,
            completed: done,
            counts: [("scored".into(), known), ("unknown".into(), done - known)].into(),
            ..Default::default()
        },
    );
    for system in [
        System::BookRole,
        System::SoloPlay,
        System::Dnd,
        System::DndEditions,
        System::DndSettings,
    ] {
        let rows=sqlx::query("SELECT c.category,count(*) AS n FROM book_classifications c WHERE c.system=? AND c.algorithm=? AND (?=0 OR EXISTS(SELECT 1 FROM book_classifications p WHERE p.target=c.target AND p.product_id=c.product_id AND p.system='dnd' AND p.algorithm=? AND p.category IN ('dnd_only','dnd_and_others'))) GROUP BY c.category")
            .bind(system.key()).bind(system.algorithm()).bind(system.needs_dnd()).bind(crate::dnd::ALGORITHM).fetch_all(&mut *conn).await?;
        let counts: std::collections::BTreeMap<String, i64> = rows
            .into_iter()
            .map(|r| (r.get("category"), r.get("n")))
            .collect();
        overview.analyses.insert(
            system.algorithm().into(),
            Coverage {
                eligible: total,
                completed: counts.values().sum(),
                counts,
                ..Default::default()
            },
        );
    }
    let roles = &overview.analyses[crate::classification::ALGORITHM];
    let adventures = roles.counts.get("adventure").copied().unwrap_or_default();
    let role_pending = total - roles.completed;
    let row=sqlx::query("SELECT count(*) AS done,coalesce(sum(p.min_players IS NOT NULL OR p.max_players IS NOT NULL),0) AS known FROM adventure_players p JOIN book_classifications c ON c.target=p.target AND c.product_id=p.product_id AND c.system='book_role' AND c.algorithm=? AND c.category='adventure' WHERE p.algorithm=?")
        .bind(crate::classification::ALGORITHM).bind(crate::players::ALGORITHM).fetch_one(&mut *conn).await?;
    let done: i64 = row.get("done");
    let known: i64 = row.get("known");
    overview.analyses.insert(
        crate::players::ALGORITHM.into(),
        Coverage {
            eligible: adventures,
            completed: done,
            prerequisite_pending: role_pending,
            counts: [("known".into(), known), ("unknown".into(), done - known)].into(),
        },
    );
    let compatibility = &overview.analyses[crate::dnd::ALGORITHM];
    let eligible = compatibility
        .counts
        .get("dnd_only")
        .copied()
        .unwrap_or_default()
        + compatibility
            .counts
            .get("dnd_and_others")
            .copied()
            .unwrap_or_default();
    let prerequisite_pending = total - compatibility.completed;
    for system in [System::DndEditions, System::DndSettings] {
        let totals = overview
            .analyses
            .get_mut(system.algorithm())
            .ok_or_else(|| Error::Config("Missing D&D totals".into()))?;
        totals.eligible = eligible;
        totals.prerequisite_pending = prerequisite_pending;
        let rows=sqlx::query("SELECT m.group_key,count(*) AS n FROM book_group_memberships m JOIN book_classifications c ON c.target=m.target AND c.product_id=m.product_id AND c.system=m.system JOIN book_classifications p ON p.target=c.target AND p.product_id=c.product_id AND p.system='dnd' AND p.algorithm=? AND p.category IN ('dnd_only','dnd_and_others') WHERE m.system=? AND c.algorithm=? AND m.group_key NOT IN ('unknown','identified','root','any') GROUP BY m.group_key")
            .bind(crate::dnd::ALGORITHM).bind(system.key()).bind(system.algorithm()).fetch_all(&mut *conn).await?;
        for r in rows {
            totals.counts.insert(r.get("group_key"), r.get("n"));
        }
    }
    for kind in crate::dnd_context::Kind::ALL {
        let row=sqlx::query("SELECT count(*) AS done,coalesce(sum(p.min_level IS NOT NULL OR p.max_level IS NOT NULL OR coalesce(json_array_length(p.locations),0)>0 OR p.in_universe_date IS NOT NULL),0) AS known FROM dnd_context p JOIN book_classifications c ON c.target=p.target AND c.product_id=p.product_id AND c.system='dnd' AND c.algorithm=? AND c.category IN ('dnd_only','dnd_and_others') WHERE p.kind=? AND p.algorithm=?")
            .bind(crate::dnd::ALGORITHM).bind(kind.key()).bind(kind.algorithm()).fetch_one(&mut *conn).await?;
        let done: i64 = row.get("done");
        let known: i64 = row.get("known");
        overview.analyses.insert(
            kind.algorithm().into(),
            Coverage {
                eligible,
                completed: done,
                prerequisite_pending,
                counts: [("known".into(), known), ("unknown".into(), done - known)].into(),
            },
        );
    }
    overview
        .analyses
        .extend(super::earth_context::coverage(conn).await?);
    overview
        .analyses
        .extend(super::earth_editions::coverage(conn).await?);
    overview.analyses.extend(super::osr::coverage(conn).await?);
    overview
        .analyses
        .extend(super::osr_levels::coverage(conn).await?);
    overview
        .analyses
        .extend(super::warhammer::coverage(conn).await?);
    Ok(overview)
}
