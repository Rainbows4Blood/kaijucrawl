//! Bounded, fenced analysis writes. Canonical captures and archives are never changed.
use super::sqlite::Catalogue;
use crate::{
    analysis::{self, Score, Status},
    domain::*,
};
use chrono::Utc;
use sqlx::{QueryBuilder, Row, Sqlite, SqlitePool};

pub const BATCH_SIZE: i64 = 500;

pub async fn latest(pool: &SqlitePool) -> Result<Status> {
    let Some(r) = sqlx::query("SELECT * FROM analysis_runs ORDER BY id DESC LIMIT 1")
        .fetch_optional(pool)
        .await?
    else {
        return Ok(Status::default());
    };
    Ok(Status {
        discovery: None,
        overview: None,
        classification: r
            .get::<Option<String>, _>("classification")
            .map(|s| serde_json::from_str(&s))
            .transpose()?,
        id: Some(r.get("id")),
        algorithm: r.get("algorithm"),
        state: r.get("state"),
        scope: r.get("scope"),
        query: r
            .get::<Option<String>, _>("query")
            .map(|s| serde_json::from_str(&s))
            .transpose()?,
        total: r.get("total"),
        processed: r.get("processed"),
        scored: r.get("scored"),
        skipped: r.get("skipped"),
        started_at: Some(r.get("started_at")),
        finished_at: r.get("finished_at"),
        detail: r.get("detail"),
    })
}

pub async fn score(pool: &SqlitePool, target: &str, id: &str) -> Result<Option<Score>> {
    Ok(sqlx::query(
        "SELECT * FROM relevance_scores WHERE target=? AND product_id=? AND algorithm=?",
    )
    .bind(target)
    .bind(id)
    .bind(analysis::ALGORITHM)
    .fetch_optional(pool)
    .await?
    .map(|r| Score {
        value: r
            .get::<Option<i64>, _>("score")
            .map(|v| v as f64 / analysis::SCALE as f64),
        algorithm: r.get("algorithm"),
        rating: r.get("input_rating"),
        count: r.get("input_count"),
        calculated_at: r.get("calculated_at"),
    }))
}

impl Catalogue {
    pub async fn begin_analysis(&self) -> Result<Status> {
        self.begin_relevance(true).await
    }

    /// Discovery keeps current scores by default, including completed unknowns.
    pub async fn begin_relevance(&self, rerun_all: bool) -> Result<Status> {
        let mut tx = self.transaction().await?;
        let total: i64 = sqlx::query_scalar("SELECT count(*) FROM products s WHERE ? OR NOT EXISTS(SELECT 1 FROM relevance_scores r WHERE r.target=s.target AND r.product_id=s.product_id AND r.algorithm=?)")
            .bind(rerun_all).bind(analysis::ALGORITHM)
            .fetch_one(&mut *tx)
            .await?;
        let upper: i64 = sqlx::query_scalar("SELECT coalesce(max(rowid),0) FROM products")
            .fetch_one(&mut *tx)
            .await?;
        let started = Utc::now().to_rfc3339();
        let scope = if rerun_all { "all" } else { "all_missing" };
        let id:i64=sqlx::query_scalar("INSERT INTO analysis_runs(algorithm,state,scope,query,started_at,total,upper_id) VALUES(?,'running',?,?,?,?,?) RETURNING id")
            .bind(analysis::ALGORITHM).bind(scope).bind(None::<String>).bind(&started).bind(total).bind(upper).fetch_one(&mut *tx).await?;
        let overview = super::overview::read(&mut tx).await?;
        self.commit(tx).await?;
        Ok(Status {
            overview: Some(overview),
            id: Some(id),
            state: "running".into(),
            scope: scope.into(),
            query: None,
            total,
            started_at: Some(started),
            ..Default::default()
        })
    }

    /// A committed batch publishes both its results and counters, or neither.
    pub async fn analysis_batch(&self, status: &mut Status) -> Result<bool> {
        let id = status
            .id
            .ok_or_else(|| Error::Config("No analysis run to process".into()))?;
        let mut tx = self.transaction().await?;
        let run=sqlx::query("SELECT last_id,upper_id,query,scope FROM analysis_runs WHERE id=? AND state='running' AND algorithm=?")
            .bind(id).bind(analysis::ALGORITHM).fetch_optional(&mut *tx).await?
            .ok_or_else(||Error::Config("Analysis is no longer running".into()))?;
        if run.get::<Option<String>, _>("query").is_some() {
            return Err(Error::Config(
                "Historical filtered runs cannot be resumed; start a new analysis".into(),
            ));
        }
        let mut b = QueryBuilder::<Sqlite>::new(
            "SELECT s.rowid AS id,s.target,s.product_id,r.algorithm IS NOT NULL AS had_score,r.score IS NOT NULL AS known_score,CASE WHEN json_type(s.record,'$.rating') IN ('integer','real','text') THEN CAST(json_extract(s.record,'$.rating') AS TEXT) END AS rating,CASE WHEN json_type(s.record,'$.rating_count') IN ('integer','real','text') THEN CAST(json_extract(s.record,'$.rating_count') AS TEXT) END AS votes FROM products s LEFT JOIN relevance_scores r ON r.target=s.target AND r.product_id=s.product_id AND r.algorithm=",
        );
        b.push_bind(analysis::ALGORITHM);
        b.push(" WHERE s.rowid>")
            .push_bind(run.get::<i64, _>("last_id"))
            .push(" AND s.rowid<=")
            .push_bind(run.get::<i64, _>("upper_id"));
        if run.get::<String, _>("scope") == "all_missing" {
            b.push(" AND r.algorithm IS NULL");
        }
        b.push(" ORDER BY s.rowid LIMIT ").push_bind(BATCH_SIZE);
        let rows = b.build().fetch_all(&mut *tx).await?;
        let now = Utc::now().to_rfc3339();
        let mut scored = 0;
        let mut overview = status.overview.clone();
        if !rows.is_empty() {
            let mut save = QueryBuilder::<Sqlite>::new(
                "INSERT INTO relevance_scores(target,product_id,algorithm,score,input_rating,input_count,calculated_at,run_id) ",
            );
            save.push_values(&rows, |mut values, r| {
                let rating = r
                    .get::<Option<String>, _>("rating")
                    .and_then(|s| s.parse::<f64>().ok())
                    .filter(|r| r.is_finite() && (1.0..=5.0).contains(r));
                let votes = r
                    .get::<Option<String>, _>("votes")
                    .and_then(|s| s.parse::<f64>().ok())
                    .filter(|n| {
                        n.is_finite() && n.fract() == 0.0 && (1.0..=1_000_000_000.0).contains(n)
                    })
                    .map(|n| n as i64);
                let score = analysis::relevance(rating, votes);
                if let Some(totals) = &mut overview {
                    super::overview::replace_score(
                        totals,
                        r.get::<bool, _>("had_score")
                            .then(|| r.get::<bool, _>("known_score")),
                        score.is_some(),
                    );
                }
                if score.is_some() {
                    scored += 1;
                }
                values
                    .push_bind(r.get::<String, _>("target"))
                    .push_bind(r.get::<String, _>("product_id"))
                    .push_bind(analysis::ALGORITHM)
                    .push_bind(score)
                    .push_bind(rating)
                    .push_bind(votes)
                    .push_bind(now.clone())
                    .push_bind(id);
            });
            save.push(" ON CONFLICT(target,product_id) DO UPDATE SET algorithm=excluded.algorithm,score=excluded.score,input_rating=excluded.input_rating,input_count=excluded.input_count,calculated_at=excluded.calculated_at,run_id=excluded.run_id");
            save.build().execute(&mut *tx).await?;
            sqlx::query("UPDATE analysis_revision SET revision=revision+1 WHERE singleton=1")
                .execute(&mut *tx)
                .await?;
        }
        let processed = rows.len() as i64;
        let done = processed < BATCH_SIZE || status.processed + processed >= status.total;
        let last = rows
            .last()
            .map_or(run.get::<i64, _>("last_id"), |r| r.get("id"));
        sqlx::query("UPDATE analysis_runs SET last_id=?,processed=processed+?,scored=scored+?,skipped=skipped+?,state=?,finished_at=? WHERE id=?")
            .bind(last).bind(processed).bind(scored).bind(processed-scored).bind(if done{"completed"}else{"running"}).bind(if done{Some(&now)}else{None}).bind(id).execute(&mut *tx).await?;
        self.commit(tx).await?;
        status.overview = overview;
        status.processed += processed;
        status.scored += scored;
        status.skipped += processed - scored;
        if done {
            status.state = "completed".into();
            status.finished_at = Some(now);
        }
        Ok(done)
    }

    pub async fn finish_analysis(
        &self,
        status: &mut Status,
        state: &str,
        detail: &str,
    ) -> Result<()> {
        let mut tx = self.transaction().await?;
        let now = Utc::now().to_rfc3339();
        sqlx::query("UPDATE analysis_runs SET state=?,finished_at=?,detail=?,classification=? WHERE id=? AND state='running'")
            .bind(state).bind(&now).bind(detail).bind(status.classification.as_ref().map(serde_json::to_string).transpose()?).bind(status.id).execute(&mut *tx).await?;
        sqlx::query("UPDATE classification_attempts SET state=?,detail='No result committed; a dispatched request may still be billed.' WHERE run_id=? AND state IN ('in_flight','retry_wait')")
            .bind(state).bind(status.id).execute(&mut *tx).await?;
        self.commit(tx).await?;
        status.state = state.into();
        status.finished_at = Some(now);
        status.detail = detail.into();
        Ok(())
    }
}
