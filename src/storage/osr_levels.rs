//! Indexed OSR Adventure subset, per-system results and fenced committed coverage.
use super::{
    classification::{Candidate, hash},
    sqlite::Catalogue,
};
use crate::{
    analysis::Status,
    classification::Progress,
    domain::*,
    osr_levels::{self, Data, Response, Saved, State},
};
use chrono::Utc;
use sqlx::{QueryBuilder, Row, Sqlite, SqlitePool};

async fn contribution(
    conn: &mut sqlx::SqliteConnection,
    book: Option<(&str, &str)>,
) -> Result<std::collections::BTreeMap<String, crate::analysis::Coverage>> {
    let mut q=QueryBuilder::<Sqlite>::new("SELECT coalesce(sum(r.category='adventure' AND o.category='identified' AND EXISTS(SELECT 1 FROM book_group_memberships m WHERE m.target=e.target AND m.product_id=e.product_id AND m.system='osr' AND m.group_key NOT IN ('root','identified','not_osr'))),0) AS eligible,count(p.product_id) AS completed,
        coalesce(sum((r.category IS NULL AND (o.category IS NULL OR o.category='identified')) OR (r.category='adventure' AND o.category IS NULL)),0) AS pending,
        coalesce(sum(p.known_count>0),0) AS known,coalesce(sum(p.unknown_count>0),0) AS unknown,coalesce(sum(p.not_applicable_count>0),0) AS not_applicable
        FROM osr_scope e
        LEFT JOIN book_classifications o ON o.target=e.target AND o.product_id=e.product_id AND o.system='osr' AND o.algorithm='osr-systems-1'
        LEFT JOIN book_classifications r ON r.target=e.target AND r.product_id=e.product_id AND r.system='book_role' AND r.algorithm='primary-book-role-1'
        LEFT JOIN osr_levels p ON p.target=e.target AND p.product_id=e.product_id AND p.algorithm='osr-levels-1' AND r.category='adventure' AND o.category='identified' AND EXISTS(SELECT 1 FROM book_group_memberships m WHERE m.target=e.target AND m.product_id=e.product_id AND m.system='osr' AND m.group_key NOT IN ('root','identified','not_osr'))");
    if let Some((target, id)) = book {
        q.push(" WHERE e.target=")
            .push_bind(target)
            .push(" AND e.product_id=")
            .push_bind(id);
    }
    let r = q.build().fetch_one(conn).await?;
    Ok([(
        osr_levels::ALGORITHM.into(),
        crate::analysis::Coverage {
            eligible: r.get("eligible"),
            completed: r.get("completed"),
            prerequisite_pending: r.get("pending"),
            counts: [
                ("known".into(), r.get("known")),
                ("unknown".into(), r.get("unknown")),
                ("not_applicable".into(), r.get("not_applicable")),
            ]
            .into(),
        },
    )]
    .into())
}
pub(super) async fn coverage(
    conn: &mut sqlx::SqliteConnection,
) -> Result<std::collections::BTreeMap<String, crate::analysis::Coverage>> {
    contribution(conn, None).await
}
pub(super) async fn book_coverage(
    conn: &mut sqlx::SqliteConnection,
    target: &str,
    id: &str,
) -> Result<std::collections::BTreeMap<String, crate::analysis::Coverage>> {
    contribution(conn, Some((target, id))).await
}

const ELIGIBLE: &str = " FROM osr_levels_eligible s WHERE 1=1";
const MISSING: &str = " AND NOT EXISTS(SELECT 1 FROM osr_levels p WHERE p.target=s.target AND p.product_id=s.product_id AND p.algorithm='osr-levels-1')";
pub async fn saved(pool: &SqlitePool, target: &str, id: &str) -> Result<Option<Saved>> {
    let mut tx = pool.begin().await?;
    let row=sqlx::query("SELECT p.* FROM osr_levels p JOIN osr_levels_eligible e ON e.target=p.target AND e.product_id=p.product_id WHERE p.target=? AND p.product_id=? AND p.algorithm=?")
        .bind(target).bind(id).bind(osr_levels::ALGORITHM).fetch_optional(&mut *tx).await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let rows=sqlx::query("SELECT system,game,status,min_level,max_level FROM osr_level_ranges WHERE target=? AND product_id=? ORDER BY system,game").bind(target).bind(id).fetch_all(&mut *tx).await?;
    let ranges = rows
        .into_iter()
        .map(|r| -> Result<_> {
            Ok(osr_levels::Range {
                system: r.get("system"),
                game: Some(r.get::<String, _>("game")).filter(|s| !s.is_empty()),
                status: serde_json::from_value(serde_json::Value::String(r.get("status")))?,
                min_level: r.get("min_level"),
                max_level: r.get("max_level"),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    tx.commit().await?;
    Ok(Some(Saved {
        data: Data { ranges },
        algorithm: row.get("algorithm"),
        model: row.get("model"),
        input_hash: row.get("input_hash"),
        calculated_at: row.get("calculated_at"),
    }))
}
impl Catalogue {
    pub async fn begin_osr_levels(&self, reanalyze: bool) -> Result<Status> {
        let mut tx = self.transaction().await?;
        let mut count = QueryBuilder::<Sqlite>::new("SELECT count(*)");
        count.push(ELIGIBLE);
        let selected: i64 = count.build_query_scalar().fetch_one(&mut *tx).await?;
        let mut count = QueryBuilder::<Sqlite>::new("SELECT count(*)");
        count.push(ELIGIBLE);
        if !reanalyze {
            count.push(MISSING);
        }
        let total: i64 = count.build_query_scalar().fetch_one(&mut *tx).await?;
        let upper: i64 = sqlx::query_scalar("SELECT coalesce(max(rowid),0) FROM products")
            .fetch_one(&mut *tx)
            .await?;
        let progress = Progress {
            workers: crate::config::logical_cpu_count(),
            selected,
            already_classified: selected - total,
            reclassify: reanalyze,
            ..Default::default()
        };
        let started = Utc::now().to_rfc3339();
        let id:i64=sqlx::query_scalar("INSERT INTO analysis_runs(algorithm,state,scope,started_at,total,upper_id,classification) VALUES(?,'running','osr_adventures',?,?,?,?) RETURNING id")
            .bind(osr_levels::ALGORITHM).bind(&started).bind(total).bind(upper).bind(serde_json::to_string(&progress)?).fetch_one(&mut *tx).await?;
        let overview = super::overview::read(&mut tx).await?;
        self.commit(tx).await?;
        Ok(Status {
            overview: Some(overview),
            id: Some(id),
            algorithm: osr_levels::ALGORITHM.into(),
            state: "running".into(),
            scope: "osr_adventures".into(),
            total,
            started_at: Some(started),
            classification: Some(progress),
            ..Default::default()
        })
    }
    pub async fn osr_levels_next_after(
        &self,
        status: &Status,
        after: i64,
    ) -> Result<Option<Candidate>> {
        if status.algorithm != osr_levels::ALGORITHM {
            return Err(Error::Config("Not an OSR level run".into()));
        }
        self.verify().await?;
        let run=sqlx::query("SELECT upper_id,classification FROM analysis_runs WHERE id=? AND algorithm=? AND state='running'")
            .bind(status.id).bind(osr_levels::ALGORITHM).fetch_optional(&self.pool).await?.ok_or_else(||Error::Config("OSR level analysis is no longer running".into()))?;
        let progress: Progress = serde_json::from_str(run.get("classification"))?;
        let mut sql = QueryBuilder::<Sqlite>::new(
            "SELECT s.document AS id,s.target,s.product_id,(SELECT CASE WHEN length(CAST(evidence AS BLOB))<=122880 THEN evidence END FROM osr_level_source e WHERE e.target=s.target AND e.product_id=s.product_id) AS evidence",
        );
        sql.push(ELIGIBLE);
        if !progress.reclassify {
            sql.push(MISSING);
        }
        sql.push(" AND s.document>")
            .push_bind(after)
            .push(" AND s.document<=")
            .push_bind(run.get::<i64, _>("upper_id"))
            .push(" ORDER BY s.document LIMIT 1");
        Ok(sql
            .build()
            .fetch_optional(&self.pool)
            .await?
            .map(|r| Candidate {
                document: r.get("id"),
                target: r.get("target"),
                product_id: r.get("product_id"),
                evidence: r.get("evidence"),
            }))
    }
    pub async fn osr_levels_commit(
        &self,
        status: &mut Status,
        book: &Candidate,
        response: Option<&Response>,
        failure: Option<&str>,
    ) -> Result<()> {
        let mut tx = self.transaction().await?;
        if status.algorithm != osr_levels::ALGORITHM {
            return Err(Error::Config("Not an OSR level run".into()));
        }
        let prior: Option<String> = sqlx::query_scalar(
            "SELECT state FROM classification_attempts WHERE run_id=? AND document=?",
        )
        .bind(status.id)
        .bind(book.document)
        .fetch_optional(&mut *tx)
        .await?;
        if prior.as_deref().is_some_and(|s| s != "in_flight") {
            return Err(Error::Config("OSR level attempt already completed".into()));
        }
        let mut p = status
            .classification
            .clone()
            .ok_or_else(|| Error::Config("Not an online analysis run".into()))?;
        let data = response.and_then(|r| r.data.as_ref());
        let mut overview = status.overview.clone();
        let before = if overview.is_some() && data.is_some() {
            Some(super::overview::book(&mut tx, &book.target, &book.product_id).await?)
        } else {
            None
        };
        if let Some(r) = response {
            p.input_tokens = p.input_tokens.saturating_add(r.usage.input_tokens);
            p.output_tokens = p.output_tokens.saturating_add(r.usage.output_tokens);
            p.cached_tokens = p
                .cached_tokens
                .saturating_add(r.usage.input_tokens_details.cached_tokens);
            p.cache_write_tokens = p
                .cache_write_tokens
                .saturating_add(r.usage.input_tokens_details.cache_write_tokens);
        }
        if let Some(data) = data {
            let current: Option<String> = sqlx::query_scalar(
                "SELECT evidence FROM osr_level_source WHERE target=? AND product_id=?",
            )
            .bind(&book.target)
            .bind(&book.product_id)
            .fetch_optional(&mut *tx)
            .await?;
            if current.is_none() || current.as_deref() != book.evidence.as_deref() {
                return Err(Error::Config("OSR evidence, Adventure role or supported systems changed during analysis; run again".into()));
            }
            let evidence: serde_json::Value =
                serde_json::from_str(current.as_deref().unwrap_or_default())?;
            data.validate(
                &osr_levels::systems(&evidence).map_err(|e| Error::Config(e.to_string()))?,
            )
            .map_err(|e| Error::Config(e.to_string()))?;
            sqlx::query("INSERT INTO osr_levels(target,product_id,algorithm,model,input_hash,calculated_at,run_id,known_count,unknown_count,not_applicable_count) VALUES(?,?,?,?,?,?,?,?,?,?) ON CONFLICT(target,product_id) DO UPDATE SET algorithm=excluded.algorithm,model=excluded.model,input_hash=excluded.input_hash,calculated_at=excluded.calculated_at,run_id=excluded.run_id,known_count=excluded.known_count,unknown_count=excluded.unknown_count,not_applicable_count=excluded.not_applicable_count")
                .bind(&book.target).bind(&book.product_id).bind(osr_levels::ALGORITHM).bind(response.map(|r|r.model.as_str()).unwrap_or(crate::responses::MODEL)).bind(hash(book.evidence.as_deref().unwrap_or_default())).bind(Utc::now().to_rfc3339()).bind(status.id).bind(data.count(State::Known)).bind(data.count(State::Unknown)).bind(data.count(State::NotApplicable)).execute(&mut *tx).await?;
            sqlx::query("DELETE FROM osr_level_ranges WHERE target=? AND product_id=?")
                .bind(&book.target)
                .bind(&book.product_id)
                .execute(&mut *tx)
                .await?;
            for range in &data.ranges {
                sqlx::query("INSERT INTO osr_level_ranges(target,product_id,system,game,status,min_level,max_level) VALUES(?,?,?,?,?,?,?)")
                    .bind(&book.target).bind(&book.product_id).bind(&range.system).bind(range.game.as_deref().unwrap_or_default()).bind(range.status.key()).bind(range.min_level).bind(range.max_level).execute(&mut *tx).await?;
            }
            sqlx::query("UPDATE analysis_revision SET revision=revision+1 WHERE singleton=1")
                .execute(&mut *tx)
                .await?;
            if data.known() {
                p.context_known += 1;
            } else {
                p.context_unknown += 1;
            }
        }
        let good = data.is_some();
        let detail = failure.unwrap_or(if good {
            ""
        } else {
            "Model refused this book; no OSR level applied"
        });
        sqlx::query("INSERT INTO classification_attempts(run_id,document,target,product_id,input_hash,state,detail) VALUES(?,?,?,?,?,?,?) ON CONFLICT(run_id,document) DO UPDATE SET state=excluded.state,detail=excluded.detail")
            .bind(status.id).bind(book.document).bind(&book.target).bind(&book.product_id).bind(book.evidence.as_deref().map(hash)).bind(if good{"analyzed"}else{"unsuccessful"}).bind(detail).execute(&mut *tx).await?;
        let changed=sqlx::query("UPDATE analysis_runs SET last_id=max(last_id,?),processed=processed+1,scored=scored+?,skipped=skipped+?,classification=?,detail=? WHERE id=? AND state='running' AND algorithm=?")
            .bind(book.document).bind(i64::from(good)).bind(i64::from(!good)).bind(serde_json::to_string(&p)?).bind(detail).bind(status.id).bind(osr_levels::ALGORITHM).execute(&mut *tx).await?.rows_affected();
        if changed != 1 {
            return Err(Error::Config(
                "OSR level analysis is no longer running".into(),
            ));
        }
        if let (Some(totals), Some(before)) = (&mut overview, before) {
            let after = super::overview::book(&mut tx, &book.target, &book.product_id).await?;
            super::overview::replace_book(totals, before, after);
        }
        self.commit(tx).await?;
        status.overview = overview;
        status.processed += 1;
        status.scored += i64::from(good);
        status.skipped += i64::from(!good);
        status.classification = Some(p);
        status.detail = detail.into();
        Ok(())
    }
}
