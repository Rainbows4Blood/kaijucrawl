//! Fixed Rule System subset extraction with independent typed results and fenced progress.
use super::{
    classification::{Candidate, hash},
    sqlite::Catalogue,
};
use crate::analysis::Coverage;
use crate::{
    analysis::Status,
    classification::Progress,
    domain::*,
    earth_context::{Data, Kind, Response, Saved},
};
use chrono::Utc;
use sqlx::{QueryBuilder, Row, Sqlite, SqlitePool};
use std::collections::BTreeMap;

pub(super) async fn book_coverage(
    conn: &mut sqlx::SqliteConnection,
    target: &str,
    id: &str,
) -> Result<BTreeMap<String, Coverage>> {
    let rows=sqlx::query("SELECT c.system,p.kind,p.algorithm,(p.location IS NOT NULL OR p.date_start IS NOT NULL OR p.date_end IS NOT NULL) AS known FROM earth_scope c LEFT JOIN earth_context p ON p.target=c.target AND p.product_id=c.product_id AND p.system=c.system WHERE c.target=? AND c.product_id=?").bind(target).bind(id).fetch_all(conn).await?;
    Ok(Kind::ALL
        .into_iter()
        .map(|kind| {
            let eligible = rows
                .iter()
                .any(|r| r.get::<&str, _>("system") == kind.system());
            let current = rows.iter().find(|r| {
                r.get::<&str, _>("system") == kind.system()
                    && r.get::<Option<&str>, _>("kind") == Some(kind.field())
                    && r.get::<Option<&str>, _>("algorithm") == Some(kind.algorithm())
            });
            (
                kind.algorithm().into(),
                Coverage {
                    eligible: i64::from(eligible),
                    completed: i64::from(current.is_some()),
                    counts: current
                        .map(|r| {
                            [(
                                if r.get::<bool, _>("known") {
                                    "known"
                                } else {
                                    "unknown"
                                }
                                .into(),
                                1,
                            )]
                            .into()
                        })
                        .unwrap_or_default(),
                    ..Default::default()
                },
            )
        })
        .collect())
}
pub(super) async fn coverage(
    conn: &mut sqlx::SqliteConnection,
) -> Result<BTreeMap<String, Coverage>> {
    let mut totals = BTreeMap::new();
    for kind in Kind::ALL {
        let eligible: i64 = sqlx::query_scalar("SELECT count(*) FROM earth_scope WHERE system=?")
            .bind(kind.system())
            .fetch_one(&mut *conn)
            .await?;
        let r=sqlx::query("SELECT count(*) AS completed,coalesce(sum(location IS NOT NULL OR date_start IS NOT NULL OR date_end IS NOT NULL),0) AS known FROM earth_context WHERE system=? AND kind=? AND algorithm=?").bind(kind.system()).bind(kind.field()).bind(kind.algorithm()).fetch_one(&mut *conn).await?;
        let completed: i64 = r.get("completed");
        let known: i64 = r.get("known");
        totals.insert(
            kind.algorithm().into(),
            Coverage {
                eligible,
                completed,
                counts: [
                    ("known".into(), known),
                    ("unknown".into(), completed - known),
                ]
                .into(),
                ..Default::default()
            },
        );
    }
    Ok(totals)
}

const ELIGIBLE: &str = " FROM products s JOIN earth_scope c ON c.target=s.target AND c.product_id=s.product_id WHERE c.system=";
fn missing(q: &mut QueryBuilder<Sqlite>, kind: Kind) {
    q.push(" AND NOT EXISTS(SELECT 1 FROM earth_context p WHERE p.target=s.target AND p.product_id=s.product_id AND p.system=").push_bind(kind.system()).push(" AND p.kind=").push_bind(kind.field()).push(" AND p.algorithm=").push_bind(kind.algorithm()).push(")");
}
pub async fn saved(pool: &SqlitePool, target: &str, id: &str, kind: Kind) -> Result<Option<Saved>> {
    let row=sqlx::query("SELECT p.* FROM earth_context p JOIN earth_scope c ON c.target=p.target AND c.product_id=p.product_id AND c.system=p.system WHERE p.target=? AND p.product_id=? AND p.system=? AND p.kind=? AND p.algorithm=?")
        .bind(target).bind(id).bind(kind.system()).bind(kind.field()).bind(kind.algorithm()).fetch_optional(pool).await?;
    row.map(|r| {
        let data = if kind.field() == "location" {
            Data::Location {
                location: r.get("location"),
            }
        } else {
            Data::Date {
                range: crate::earth_context::DateRange {
                    start: r.try_get("date_start")?,
                    end: r.try_get("date_end")?,
                    precision: serde_json::from_value(serde_json::Value::String(
                        r.get("precision"),
                    ))?,
                    basis: serde_json::from_value(serde_json::Value::String(r.get("basis")))?,
                },
            }
        };
        data.validate(kind)
            .map_err(|_| Error::Config("Invalid saved system context".into()))?;
        Ok(Saved {
            data,
            algorithm: r.get("algorithm"),
            model: r.get("model"),
            input_hash: r.get("input_hash"),
            calculated_at: r.get("calculated_at"),
        })
    })
    .transpose()
}

impl Catalogue {
    pub async fn begin_earth(&self, kind: Kind, reanalyze: bool) -> Result<Status> {
        let mut tx = self.transaction().await?;
        let mut count = QueryBuilder::<Sqlite>::new("SELECT count(*)");
        count.push(ELIGIBLE).push_bind(kind.system());
        let selected: i64 = count.build_query_scalar().fetch_one(&mut *tx).await?;
        let mut count = QueryBuilder::<Sqlite>::new("SELECT count(*)");
        count.push(ELIGIBLE).push_bind(kind.system());
        if !reanalyze {
            missing(&mut count, kind);
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
        let id:i64=sqlx::query_scalar("INSERT INTO analysis_runs(algorithm,state,scope,started_at,total,upper_id,classification) VALUES(?,'running',?,?,?,?,?) RETURNING id")
            .bind(kind.algorithm()).bind(kind.system()).bind(&started).bind(total).bind(upper).bind(serde_json::to_string(&progress)?).fetch_one(&mut *tx).await?;
        let overview = super::overview::read(&mut tx).await?;
        self.commit(tx).await?;
        Ok(Status {
            overview: Some(overview),
            id: Some(id),
            algorithm: kind.algorithm().into(),
            state: "running".into(),
            scope: kind.system().into(),
            total,
            started_at: Some(started),
            classification: Some(progress),
            ..Default::default()
        })
    }
    pub async fn earth_next_after(&self, status: &Status, after: i64) -> Result<Option<Candidate>> {
        let kind = Kind::from_algorithm(&status.algorithm)
            .ok_or_else(|| Error::Config("Not a System context run".into()))?;
        self.verify().await?;
        let run=sqlx::query("SELECT upper_id,classification FROM analysis_runs WHERE id=? AND algorithm=? AND state='running'")
            .bind(status.id).bind(kind.algorithm()).fetch_optional(&self.pool).await?.ok_or_else(||Error::Config("System context analysis is no longer running".into()))?;
        let progress: Progress = serde_json::from_str(run.get("classification"))?;
        let mut sql = QueryBuilder::<Sqlite>::new(
            "SELECT c.document AS id,s.target,s.product_id,(SELECT CASE WHEN length(CAST(evidence AS BLOB))<=122880 THEN evidence END FROM classification_source e WHERE e.target=s.target AND e.product_id=s.product_id) AS evidence",
        );
        sql.push(ELIGIBLE).push_bind(kind.system());
        if !progress.reclassify {
            missing(&mut sql, kind);
        }
        sql.push(" AND c.document>")
            .push_bind(after)
            .push(" AND c.document<=")
            .push_bind(run.get::<i64, _>("upper_id"))
            .push(" ORDER BY c.document LIMIT 1");
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
    pub async fn earth_commit(
        &self,
        status: &mut Status,
        book: &Candidate,
        response: Option<&Response>,
        failure: Option<&str>,
    ) -> Result<()> {
        let mut tx = self.transaction().await?;
        let kind = Kind::from_algorithm(&status.algorithm)
            .ok_or_else(|| Error::Config("Not a System context run".into()))?;
        let prior: Option<String> = sqlx::query_scalar(
            "SELECT state FROM classification_attempts WHERE run_id=? AND document=?",
        )
        .bind(status.id)
        .bind(book.document)
        .fetch_optional(&mut *tx)
        .await?;
        if prior.as_deref().is_some_and(|s| s != "in_flight") {
            return Err(Error::Config(
                "System context attempt already completed".into(),
            ));
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
            data.validate(kind)
                .map_err(|e| Error::Config(e.to_string()))?;
            let current:Option<String>=sqlx::query_scalar("SELECT e.evidence FROM classification_source e JOIN earth_scope c ON c.target=e.target AND c.product_id=e.product_id WHERE e.target=? AND e.product_id=? AND c.system=?")
                .bind(&book.target).bind(&book.product_id).bind(kind.system()).fetch_optional(&mut *tx).await?;
            if current.is_none() || current.as_deref() != book.evidence.as_deref() {
                return Err(Error::Config(
                    "System evidence or eligibility changed during analysis; run again".into(),
                ));
            }
            let (location, start, end, precision, basis) = match data {
                Data::Location { location } => (location.as_deref(), None, None, None, None),
                Data::Date { range } => (
                    None,
                    range.start,
                    range.end,
                    Some(serde_json::to_value(range.precision)?),
                    Some(serde_json::to_value(range.basis)?),
                ),
            };
            sqlx::query("INSERT INTO earth_context(target,product_id,system,kind,location,date_start,date_end,precision,basis,algorithm,model,input_hash,calculated_at,run_id) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(target,product_id,system,kind) DO UPDATE SET location=excluded.location,date_start=excluded.date_start,date_end=excluded.date_end,precision=excluded.precision,basis=excluded.basis,algorithm=excluded.algorithm,model=excluded.model,input_hash=excluded.input_hash,calculated_at=excluded.calculated_at,run_id=excluded.run_id")
                .bind(&book.target).bind(&book.product_id).bind(kind.system()).bind(kind.field()).bind(location).bind(start).bind(end).bind(precision.as_ref().and_then(|v|v.as_str())).bind(basis.as_ref().and_then(|v|v.as_str())).bind(kind.algorithm()).bind(response.map(|r|r.model.as_str()).unwrap_or(crate::earth_context::MODEL)).bind(hash(book.evidence.as_deref().unwrap_or_default())).bind(Utc::now().to_rfc3339()).bind(status.id).execute(&mut *tx).await?;
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
            "Model refused this book; no System context applied"
        });
        sqlx::query("INSERT INTO classification_attempts(run_id,document,target,product_id,input_hash,state,detail) VALUES(?,?,?,?,?,?,?) ON CONFLICT(run_id,document) DO UPDATE SET state=excluded.state,detail=excluded.detail")
            .bind(status.id).bind(book.document).bind(&book.target).bind(&book.product_id).bind(book.evidence.as_deref().map(hash)).bind(if good{"analyzed"}else{"unsuccessful"}).bind(detail).execute(&mut *tx).await?;
        let changed=sqlx::query("UPDATE analysis_runs SET last_id=max(last_id,?),processed=processed+1,scored=scored+?,skipped=skipped+?,classification=?,detail=? WHERE id=? AND state='running' AND algorithm=?")
            .bind(book.document).bind(i64::from(good)).bind(i64::from(!good)).bind(serde_json::to_string(&p)?).bind(detail).bind(status.id).bind(kind.algorithm()).execute(&mut *tx).await?.rows_affected();
        if changed != 1 {
            return Err(Error::Config(
                "System context analysis is no longer running".into(),
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
