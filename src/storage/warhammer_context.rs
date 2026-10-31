//! Fixed Warhammer subset extraction with independent typed results and fenced progress.
use super::{
    classification::{Candidate, hash},
    sqlite::Catalogue,
};
use crate::{
    analysis::Status,
    classification::Progress,
    dnd_context::{Data, Response, Saved},
    domain::*,
    warhammer_context::Kind,
};
use chrono::Utc;
use sqlx::{QueryBuilder, Row, Sqlite, SqlitePool};

fn eligible(q: &mut QueryBuilder<Sqlite>, kind: Kind) {
    q.push(" FROM warhammer_scope e CROSS JOIN products s ON s.target=e.target AND s.product_id=e.product_id CROSS JOIN book_classifications c ON c.target=e.target AND c.product_id=e.product_id AND c.system=e.system WHERE e.system=").push_bind(kind.family().key()).push(" AND c.algorithm=").push_bind(kind.family().algorithm()).push(" AND c.category IN ('identified','unknown')");
}
fn missing(q: &mut QueryBuilder<Sqlite>, kind: Kind) {
    q.push(" AND NOT EXISTS(SELECT 1 FROM warhammer_context p WHERE p.target=s.target AND p.product_id=s.product_id AND p.system=").push_bind(kind.family().key()).push(" AND p.kind=").push_bind(kind.field()).push(" AND p.algorithm=").push_bind(kind.algorithm()).push(")");
}
pub async fn saved(pool: &SqlitePool, target: &str, id: &str, kind: Kind) -> Result<Option<Saved>> {
    let row=sqlx::query("SELECT p.* FROM warhammer_context p JOIN book_classifications c ON c.target=p.target AND c.product_id=p.product_id AND c.system=p.system WHERE p.target=? AND p.product_id=? AND p.system=? AND p.kind=? AND p.algorithm=? AND c.algorithm=? AND c.category IN ('identified','unknown')")
 .bind(target).bind(id).bind(kind.family().key()).bind(kind.field()).bind(kind.algorithm()).bind(kind.family().algorithm()).fetch_optional(pool).await?;
    row.map(|r| {
 let value=if kind.field()=="locations" {serde_json::json!({"locations":serde_json::from_str::<serde_json::Value>(r.get("locations"))?})} else {serde_json::json!({"in_universe_date":r.get::<Option<String>,_>("in_universe_date")})};
 Ok(Saved {data:kind.data(value).map_err(|_|Error::Config("Invalid saved Warhammer context".into()))?,algorithm:r.get("algorithm"),model:r.get("model"),input_hash:r.get("input_hash"),calculated_at:r.get("calculated_at")})
 }).transpose()
}
impl Catalogue {
    pub async fn begin_warhammer_context(&self, kind: Kind, reanalyze: bool) -> Result<Status> {
        let mut tx = self.transaction().await?;
        let mut count = QueryBuilder::<Sqlite>::new("SELECT count(*)");
        eligible(&mut count, kind);
        let selected: i64 = count.build_query_scalar().fetch_one(&mut *tx).await?;
        let mut count = QueryBuilder::<Sqlite>::new("SELECT count(*)");
        eligible(&mut count, kind);
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
            .bind(kind.algorithm()).bind(kind.family().key()).bind(&started).bind(total).bind(upper).bind(serde_json::to_string(&progress)?).fetch_one(&mut *tx).await?;
        let overview = super::overview::read(&mut tx).await?;
        self.commit(tx).await?;
        Ok(Status {
            overview: Some(overview),
            id: Some(id),
            algorithm: kind.algorithm().into(),
            state: "running".into(),
            scope: kind.family().key().into(),
            total,
            started_at: Some(started),
            classification: Some(progress),
            ..Default::default()
        })
    }
    pub async fn warhammer_context_next_after(
        &self,
        status: &Status,
        after: i64,
    ) -> Result<Option<Candidate>> {
        let kind = Kind::from_algorithm(&status.algorithm)
            .ok_or_else(|| Error::Config("Not a Warhammer context run".into()))?;
        self.verify().await?;
        let run=sqlx::query("SELECT upper_id,classification FROM analysis_runs WHERE id=? AND algorithm=? AND state='running'")
            .bind(status.id).bind(kind.algorithm()).fetch_optional(&self.pool).await?.ok_or_else(||Error::Config("Warhammer context analysis is no longer running".into()))?;
        let progress: Progress = serde_json::from_str(run.get("classification"))?;
        let mut sql = QueryBuilder::<Sqlite>::new(
            "SELECT s.rowid AS id,s.target,s.product_id,(SELECT CASE WHEN length(CAST(evidence AS BLOB))<=122880 THEN evidence END FROM classification_source e WHERE e.target=s.target AND e.product_id=s.product_id) AS evidence",
        );
        eligible(&mut sql, kind);
        if !progress.reclassify {
            missing(&mut sql, kind);
        }
        sql.push(" AND e.document>")
            .push_bind(after)
            .push(" AND e.document<=")
            .push_bind(run.get::<i64, _>("upper_id"))
            .push(" ORDER BY e.document LIMIT 1");
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
    pub async fn warhammer_context_commit(
        &self,
        status: &mut Status,
        book: &Candidate,
        response: Option<&Response>,
        failure: Option<&str>,
    ) -> Result<()> {
        let mut tx = self.transaction().await?;
        let kind = Kind::from_algorithm(&status.algorithm)
            .ok_or_else(|| Error::Config("Not a Warhammer context run".into()))?;
        let prior: Option<String> = sqlx::query_scalar(
            "SELECT state FROM classification_attempts WHERE run_id=? AND document=?",
        )
        .bind(status.id)
        .bind(book.document)
        .fetch_optional(&mut *tx)
        .await?;
        if prior.as_deref().is_some_and(|s| s != "in_flight") {
            return Err(Error::Config(
                "Warhammer context attempt already completed".into(),
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
            if data.kind() != kind.parser() {
                return Err(Error::Config(
                    "Warhammer context result kind mismatch".into(),
                ));
            }
            data.validate().map_err(|e| Error::Config(e.to_string()))?;
            let current:Option<String>=sqlx::query_scalar("SELECT e.evidence FROM classification_source e JOIN warhammer_scope w ON w.target=e.target AND w.product_id=e.product_id JOIN book_classifications c ON c.target=w.target AND c.product_id=w.product_id AND c.system=w.system WHERE e.target=? AND e.product_id=? AND w.system=? AND c.algorithm=? AND c.category IN ('identified','unknown')")
                .bind(&book.target).bind(&book.product_id).bind(kind.family().key()).bind(kind.family().algorithm()).fetch_optional(&mut *tx).await?;
            if current.is_none() || current.as_deref() != book.evidence.as_deref() {
                return Err(Error::Config(
                    "Warhammer evidence or eligibility changed during analysis; run again".into(),
                ));
            }
            let (locations, date) = match data {
                Data::Locations { locations } => (Some(serde_json::to_string(locations)?), None),
                Data::Date { in_universe_date } => (None, in_universe_date.as_deref()),
                _ => return Err(Error::Config("Warhammer result kind mismatch".into())),
            };
            sqlx::query("INSERT INTO warhammer_context(target,product_id,system,kind,locations,in_universe_date,algorithm,model,input_hash,calculated_at,run_id) VALUES(?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(target,product_id,system,kind) DO UPDATE SET locations=excluded.locations,in_universe_date=excluded.in_universe_date,algorithm=excluded.algorithm,model=excluded.model,input_hash=excluded.input_hash,calculated_at=excluded.calculated_at,run_id=excluded.run_id")
                .bind(&book.target).bind(&book.product_id).bind(kind.family().key()).bind(kind.field()).bind(locations).bind(date).bind(kind.algorithm()).bind(response.map(|r|r.model.as_str()).unwrap_or(crate::dnd_context::MODEL)).bind(hash(book.evidence.as_deref().unwrap_or_default())).bind(Utc::now().to_rfc3339()).bind(status.id).execute(&mut *tx).await?;
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
            "Model refused this book; no Warhammer context applied"
        });
        sqlx::query("INSERT INTO classification_attempts(run_id,document,target,product_id,input_hash,state,detail) VALUES(?,?,?,?,?,?,?) ON CONFLICT(run_id,document) DO UPDATE SET state=excluded.state,detail=excluded.detail")
            .bind(status.id).bind(book.document).bind(&book.target).bind(&book.product_id).bind(book.evidence.as_deref().map(hash)).bind(if good{"analyzed"}else{"unsuccessful"}).bind(detail).execute(&mut *tx).await?;
        let changed=sqlx::query("UPDATE analysis_runs SET last_id=max(last_id,?),processed=processed+1,scored=scored+?,skipped=skipped+?,classification=?,detail=? WHERE id=? AND state='running' AND algorithm=?")
            .bind(book.document).bind(i64::from(good)).bind(i64::from(!good)).bind(serde_json::to_string(&p)?).bind(detail).bind(status.id).bind(kind.algorithm()).execute(&mut *tx).await?.rows_affected();
        if changed != 1 {
            return Err(Error::Config(
                "Warhammer context analysis is no longer running".into(),
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
