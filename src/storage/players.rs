//! Adventure-only extraction, with fenced progress and independently persisted nullable results.
use super::{
    classification::{Candidate, hash},
    sqlite::Catalogue,
};
use crate::{
    analysis::Status,
    classification::Progress,
    domain::*,
    players::{self, Counts, Response, Saved},
};
use chrono::Utc;
use sqlx::{QueryBuilder, Row, Sqlite, SqlitePool};

const ELIGIBLE: &str = " FROM products s JOIN book_classifications c ON c.target=s.target AND c.product_id=s.product_id WHERE c.system='book_role' AND c.algorithm='primary-book-role-1' AND c.category='adventure'";
const MISSING: &str = " AND NOT EXISTS(SELECT 1 FROM adventure_players p WHERE p.target=s.target AND p.product_id=s.product_id AND p.algorithm='adventure-players-1')";

pub async fn saved(pool: &SqlitePool, target: &str, id: &str) -> Result<Option<Saved>> {
    Ok(sqlx::query("SELECT p.* FROM adventure_players p JOIN book_classifications c ON c.target=p.target AND c.product_id=p.product_id AND c.system=p.system WHERE p.target=? AND p.product_id=? AND p.algorithm=? AND c.category='adventure' AND c.algorithm=?")
        .bind(target).bind(id).bind(players::ALGORITHM).bind(crate::classification::ALGORITHM)
        .fetch_optional(pool).await?.map(|r|Saved{counts:Counts{min_players:r.get("min_players"),max_players:r.get("max_players")},algorithm:r.get("algorithm"),model:r.get("model"),input_hash:r.get("input_hash"),calculated_at:r.get("calculated_at")}))
}
impl Catalogue {
    pub async fn begin_players(&self, reanalyze: bool) -> Result<Status> {
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
        let id:i64=sqlx::query_scalar("INSERT INTO analysis_runs(algorithm,state,scope,started_at,total,upper_id,classification) VALUES(?,'running','adventures',?,?,?,?) RETURNING id")
            .bind(players::ALGORITHM).bind(&started).bind(total).bind(upper).bind(serde_json::to_string(&progress)?).fetch_one(&mut *tx).await?;
        let overview = super::overview::read(&mut tx).await?;
        self.commit(tx).await?;
        Ok(Status {
            overview: Some(overview),
            id: Some(id),
            algorithm: players::ALGORITHM.into(),
            state: "running".into(),
            scope: "adventures".into(),
            total,
            started_at: Some(started),
            classification: Some(progress),
            ..Default::default()
        })
    }
    pub async fn players_next_after(
        &self,
        status: &Status,
        after: i64,
    ) -> Result<Option<Candidate>> {
        self.verify().await?;
        let run=sqlx::query("SELECT upper_id,classification FROM analysis_runs WHERE id=? AND algorithm=? AND state='running'")
            .bind(status.id).bind(players::ALGORITHM).fetch_optional(&self.pool).await?.ok_or_else(||Error::Config("Player analysis is no longer running".into()))?;
        let progress: Progress = serde_json::from_str(run.get("classification"))?;
        let mut sql = QueryBuilder::<Sqlite>::new(
            "SELECT s.rowid AS id,s.target,s.product_id,(SELECT CASE WHEN length(CAST(evidence AS BLOB))<=122880 THEN evidence END FROM player_source e WHERE e.target=s.target AND e.product_id=s.product_id) AS evidence",
        );
        sql.push(ELIGIBLE);
        if !progress.reclassify {
            sql.push(MISSING);
        }
        sql.push(" AND s.rowid>")
            .push_bind(after)
            .push(" AND s.rowid<=")
            .push_bind(run.get::<i64, _>("upper_id"))
            .push(" ORDER BY s.rowid LIMIT 1");
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
    pub async fn players_commit(
        &self,
        status: &mut Status,
        book: &Candidate,
        response: Option<&Response>,
        failure: Option<&str>,
    ) -> Result<()> {
        let mut tx = self.transaction().await?;
        let prior: Option<String> = sqlx::query_scalar(
            "SELECT state FROM classification_attempts WHERE run_id=? AND document=?",
        )
        .bind(status.id)
        .bind(book.document)
        .fetch_optional(&mut *tx)
        .await?;
        if prior.as_deref().is_some_and(|s| s != "in_flight") {
            return Err(Error::Config(
                "Player analysis attempt already completed".into(),
            ));
        }
        let mut p = status
            .classification
            .clone()
            .ok_or_else(|| Error::Config("Not an online analysis run".into()))?;
        let counts = response.and_then(|r| r.counts.as_ref());
        let mut overview = status.overview.clone();
        let before = if overview.is_some() && counts.is_some() {
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
        if let Some(counts) = counts {
            counts
                .validate()
                .map_err(|e| Error::Config(e.to_string()))?;
            let current:Option<String>=sqlx::query_scalar("SELECT e.evidence FROM player_source e JOIN book_classifications c ON c.target=e.target AND c.product_id=e.product_id WHERE e.target=? AND e.product_id=? AND c.system='book_role' AND c.algorithm=? AND c.category='adventure'")
                .bind(&book.target).bind(&book.product_id).bind(crate::classification::ALGORITHM).fetch_optional(&mut *tx).await?;
            if current.is_none() || current.as_deref() != book.evidence.as_deref() {
                return Err(Error::Config(
                    "Adventure evidence or role changed during analysis; run again".into(),
                ));
            }
            sqlx::query("INSERT INTO adventure_players(target,product_id,min_players,max_players,algorithm,model,input_hash,calculated_at,run_id) VALUES(?,?,?,?,?,?,?,?,?) ON CONFLICT(target,product_id) DO UPDATE SET min_players=excluded.min_players,max_players=excluded.max_players,algorithm=excluded.algorithm,model=excluded.model,input_hash=excluded.input_hash,calculated_at=excluded.calculated_at,run_id=excluded.run_id")
                .bind(&book.target).bind(&book.product_id).bind(counts.min_players).bind(counts.max_players).bind(players::ALGORITHM).bind(response.map(|r|r.model.as_str()).unwrap_or(players::MODEL)).bind(hash(book.evidence.as_deref().unwrap_or_default())).bind(Utc::now().to_rfc3339()).bind(status.id).execute(&mut *tx).await?;
            sqlx::query("UPDATE analysis_revision SET revision=revision+1 WHERE singleton=1")
                .execute(&mut *tx)
                .await?;
            if counts.known() {
                p.known_players += 1;
            } else {
                p.unknown_players += 1;
            }
        }
        let good = counts.is_some();
        let detail = failure.unwrap_or(if good {
            ""
        } else {
            "Model refused this book; no player counts applied"
        });
        sqlx::query("INSERT INTO classification_attempts(run_id,document,target,product_id,input_hash,state,detail) VALUES(?,?,?,?,?,?,?) ON CONFLICT(run_id,document) DO UPDATE SET state=excluded.state,detail=excluded.detail")
            .bind(status.id).bind(book.document).bind(&book.target).bind(&book.product_id).bind(book.evidence.as_deref().map(hash)).bind(if good{"analyzed"}else{"unsuccessful"}).bind(detail).execute(&mut *tx).await?;
        let changed=sqlx::query("UPDATE analysis_runs SET last_id=max(last_id,?),processed=processed+1,scored=scored+?,skipped=skipped+?,classification=?,detail=? WHERE id=? AND state='running' AND algorithm=?")
            .bind(book.document).bind(i64::from(good)).bind(i64::from(!good)).bind(serde_json::to_string(&p)?).bind(detail).bind(status.id).bind(players::ALGORITHM).execute(&mut *tx).await?.rows_affected();
        if changed != 1 {
            return Err(Error::Config("Player analysis is no longer running".into()));
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
