//! Fenced classification checkpoints and groups, with no transactions across HTTP.
use super::sqlite::Catalogue;
use crate::{
    analysis::Status,
    classification::{Classification, Progress, system::System},
    decisions::Decision,
    domain::*,
};
use chrono::Utc;
use sha2::{Digest, Sha256};
use sqlx::{QueryBuilder, Row, Sqlite, SqlitePool};
const EVIDENCE_LIMIT: i64 = 120 * 1024;
pub struct Candidate {
    pub document: i64,
    pub target: String,
    pub product_id: String,
    pub evidence: Option<String>,
}
fn source(b: &mut QueryBuilder<Sqlite>, system: System) {
    if matches!(system, System::Osr) {
        b.push(
            " FROM osr_scope e JOIN products s ON s.target=e.target AND s.product_id=e.product_id",
        );
    } else if matches!(system, System::Warhammer(_)) {
        b.push(" FROM warhammer_scope e CROSS JOIN products s ON s.target=e.target AND s.product_id=e.product_id");
    } else if system.rule_system().is_some() {
        b.push(" FROM earth_scope e JOIN products s ON s.target=e.target AND s.product_id=e.product_id");
    } else {
        b.push(" FROM products s");
    }
}
fn scope(b: &mut QueryBuilder<Sqlite>, system: System) {
    if let System::Warhammer(f) = system {
        b.push(" AND e.system=").push_bind(f.key());
    }
    if let Some(scope) = system.rule_system() {
        b.push(" AND e.system=").push_bind(scope);
    }
    if system.needs_dnd() {
        b.push(" AND EXISTS(SELECT 1 FROM book_classifications prerequisite WHERE prerequisite.target=s.target AND prerequisite.product_id=s.product_id AND prerequisite.system='dnd' AND prerequisite.algorithm=")
            .push_bind(crate::dnd::ALGORITHM).push(" AND prerequisite.category IN ('dnd_only','dnd_and_others'))");
    }
}
fn eligible(b: &mut QueryBuilder<Sqlite>, system: System, reclassify: bool) {
    scope(b, system);
    if !reclassify {
        b.push(" AND NOT EXISTS(SELECT 1 FROM book_classifications c WHERE c.target=s.target AND c.product_id=s.product_id AND c.system=")
            .push_bind(system.key()).push(" AND c.algorithm=").push_bind(system.algorithm()).push(")");
    }
}
pub fn hash(input: &str) -> String {
    hex::encode(Sha256::digest(input.as_bytes()))
}
pub async fn saved(pool: &SqlitePool, target: &str, id: &str) -> Result<Option<Classification>> {
    saved_for(pool, target, id, System::BookRole).await
}
pub async fn saved_solo(
    pool: &SqlitePool,
    target: &str,
    id: &str,
) -> Result<Option<crate::solo::Classification>> {
    saved_for(pool, target, id, System::SoloPlay).await
}
pub async fn saved_dnd(
    pool: &SqlitePool,
    target: &str,
    id: &str,
) -> Result<Option<crate::dnd::Classification>> {
    saved_for(pool, target, id, System::Dnd).await
}
pub async fn saved_settings(
    pool: &SqlitePool,
    target: &str,
    id: &str,
) -> Result<Option<Classification<String>>> {
    saved_for(pool, target, id, System::DndSettings).await
}
pub async fn saved_editions(
    pool: &SqlitePool,
    target: &str,
    id: &str,
) -> Result<Option<crate::dnd::Editions>> {
    saved_editions_for(pool, target, id, System::DndEditions).await
}
pub async fn saved_earth_editions(
    pool: &SqlitePool,
    target: &str,
    id: &str,
    kind: crate::earth_editions::Kind,
) -> Result<Option<crate::dnd::Editions>> {
    saved_editions_for(pool, target, id, System::EarthEditions(kind)).await
}
async fn saved_editions_for(
    pool: &SqlitePool,
    target: &str,
    id: &str,
    system: System,
) -> Result<Option<crate::dnd::Editions>> {
    let row: Option<Classification<String>> = saved_for(pool, target, id, system).await?;
    row.map(|r| {
        let assessments = r.probabilities;
        let editions = assessments
            .as_array()
            .ok_or_else(|| Error::Config("Invalid saved edition assessments".into()))?
            .iter()
            .filter(|a| a["status"] == "supported")
            .filter_map(|a| a["edition"].as_str().map(str::to_owned))
            .collect();
        Ok(crate::dnd::Editions {
            editions,
            assessments,
            algorithm: r.algorithm,
            model: r.model,
            input_hash: r.input_hash,
            calculated_at: r.calculated_at,
        })
    })
    .transpose()
}
async fn saved_for<C: serde::de::DeserializeOwned>(
    pool: &SqlitePool,
    target: &str,
    id: &str,
    system: System,
) -> Result<Option<Classification<C>>> {
    let row=sqlx::query("SELECT * FROM book_classifications WHERE target=? AND product_id=? AND system=? AND algorithm=?")
        .bind(target).bind(id).bind(system.key()).bind(system.algorithm()).fetch_optional(pool).await?;
    row.map(|r| {
        Ok(Classification {
            category: serde_json::from_value(serde_json::Value::String(r.get("category")))?,
            algorithm: r.get("algorithm"),
            model: r.get("model"),
            confidence: r.get("confidence"),
            probabilities: serde_json::from_str(r.get("probabilities"))?,
            input_hash: r.get("input_hash"),
            calculated_at: r.get("calculated_at"),
        })
    })
    .transpose()
}
impl Catalogue {
    pub async fn begin_classification(&self, reclassify: bool) -> Result<Status> {
        self.begin_classification_system(System::BookRole, reclassify)
            .await
    }
    pub async fn begin_solo_classification(&self, reclassify: bool) -> Result<Status> {
        self.begin_classification_system(System::SoloPlay, reclassify)
            .await
    }
    pub async fn begin_dnd(&self, editions: bool, reclassify: bool) -> Result<Status> {
        self.begin_classification_system(
            if editions {
                System::DndEditions
            } else {
                System::Dnd
            },
            reclassify,
        )
        .await
    }
    pub async fn begin_warhammer(
        &self,
        family: crate::warhammer::Family,
        reclassify: bool,
    ) -> Result<Status> {
        self.begin_classification_system(System::Warhammer(family), reclassify)
            .await
    }
    pub async fn begin_osr(&self, reclassify: bool) -> Result<Status> {
        self.begin_classification_system(System::Osr, reclassify)
            .await
    }
    pub async fn begin_settings(&self, reclassify: bool) -> Result<Status> {
        self.begin_classification_system(System::DndSettings, reclassify)
            .await
    }
    pub async fn begin_earth_editions(
        &self,
        kind: crate::earth_editions::Kind,
        reclassify: bool,
    ) -> Result<Status> {
        self.begin_classification_system(System::EarthEditions(kind), reclassify)
            .await
    }
    async fn begin_classification_system(
        &self,
        system: System,
        reclassify: bool,
    ) -> Result<Status> {
        let mut tx = self.transaction().await?;
        let mut count = QueryBuilder::<Sqlite>::new("SELECT count(*)");
        source(&mut count, system);
        count.push(" WHERE 1=1");
        scope(&mut count, system);
        let selected: i64 = count.build_query_scalar().fetch_one(&mut *tx).await?;
        let mut count = QueryBuilder::<Sqlite>::new("SELECT count(*)");
        source(&mut count, system);
        count.push(" WHERE 1=1");
        eligible(&mut count, system, reclassify);
        let total: i64 = count.build_query_scalar().fetch_one(&mut *tx).await?;
        let upper: i64 = sqlx::query_scalar("SELECT coalesce(max(rowid),0) FROM products")
            .fetch_one(&mut *tx)
            .await?;
        let started = Utc::now().to_rfc3339();
        let scope = system
            .rule_system()
            .unwrap_or(if let System::Warhammer(f) = system {
                f.key()
            } else if system.needs_dnd() {
                "dnd"
            } else if matches!(system, System::Osr) {
                "osr"
            } else {
                "all"
            });
        let progress = Progress {
            workers: crate::config::logical_cpu_count(),
            selected,
            already_classified: selected - total,
            reclassify,
            ..Default::default()
        };
        let id:i64=sqlx::query_scalar("INSERT INTO analysis_runs(algorithm,state,scope,query,started_at,total,upper_id,classification) VALUES(?,'running',?,?,?,?,?,?) RETURNING id")
            .bind(system.algorithm()).bind(scope).bind(None::<String>).bind(&started).bind(total).bind(upper).bind(serde_json::to_string(&progress)?).fetch_one(&mut *tx).await?;
        let overview = super::overview::read(&mut tx).await?;
        self.commit(tx).await?;
        Ok(Status {
            overview: Some(overview),
            id: Some(id),
            algorithm: system.algorithm().into(),
            state: "running".into(),
            scope: scope.into(),
            query: None,
            total,
            started_at: Some(started),
            classification: Some(progress),
            ..Default::default()
        })
    }
    pub async fn classification_next(&self, status: &Status) -> Result<Option<Candidate>> {
        self.classification_candidate(status, None).await
    }
    /// The admission cursor is independent of out-of-order completion checkpoints.
    pub async fn classification_next_after(
        &self,
        status: &Status,
        after: i64,
    ) -> Result<Option<Candidate>> {
        self.classification_candidate(status, Some(after)).await
    }
    async fn classification_candidate(
        &self,
        status: &Status,
        after: Option<i64>,
    ) -> Result<Option<Candidate>> {
        let system = System::from_algorithm(&status.algorithm)?;
        self.verify().await?;
        let run=sqlx::query("SELECT last_id,upper_id,query,classification FROM analysis_runs WHERE id=? AND algorithm=? AND state='running'")
            .bind(status.id).bind(system.algorithm()).fetch_optional(&self.pool).await?.ok_or_else(||Error::Config("Classification is no longer running".into()))?;
        if run.get::<Option<String>, _>("query").is_some() {
            return Err(Error::Config(
                "Historical filtered runs cannot be resumed; start a new analysis".into(),
            ));
        }
        let progress: Progress = serde_json::from_str(run.get("classification"))?;
        let mut b = QueryBuilder::<Sqlite>::new(
            "SELECT s.rowid AS id,s.target,s.product_id,CASE WHEN length(CAST(c.evidence AS BLOB))<=",
        );
        b.push_bind(EVIDENCE_LIMIT)
            .push(" THEN c.evidence END AS evidence");
        source(&mut b, system);
        b.push(" JOIN classification_source c ON c.target=s.target AND c.product_id=s.product_id");
        b.push(" WHERE 1=1");
        eligible(&mut b, system, progress.reclassify);
        let cursor = if system.rule_system().is_some()
            || matches!(system, System::Osr | System::Warhammer(_))
        {
            "e.document"
        } else {
            "s.rowid"
        };
        b.push(" AND ")
            .push(cursor)
            .push(">")
            .push_bind(after.unwrap_or_else(|| run.get::<i64, _>("last_id")))
            .push(" AND ")
            .push(cursor)
            .push("<=")
            .push_bind(run.get::<i64, _>("upper_id"))
            .push(" ORDER BY ")
            .push(cursor)
            .push(" LIMIT 1");
        Ok(b.build()
            .fetch_optional(&self.pool)
            .await?
            .map(|r| Candidate {
                document: r.get("id"),
                target: r.get("target"),
                product_id: r.get("product_id"),
                evidence: r.get("evidence"),
            }))
    }
    /// A durable dispatch record makes possible billed-but-uncommitted requests visible.
    pub async fn classification_dispatch(
        &self,
        status: &mut Status,
        book: &Candidate,
    ) -> Result<()> {
        self.dispatch_classification(status, book, false).await
    }
    pub async fn classification_redispatch(
        &self,
        status: &mut Status,
        book: &Candidate,
    ) -> Result<()> {
        self.dispatch_classification(status, book, true).await
    }
    /// A retry retains the book's checkpoint, but every HTTP dispatch is counted.
    async fn dispatch_classification(
        &self,
        status: &mut Status,
        book: &Candidate,
        retry: bool,
    ) -> Result<()> {
        let mut tx = self.transaction().await?;
        let mut progress = status
            .classification
            .clone()
            .ok_or_else(|| Error::Config("Not a classification run".into()))?;
        progress.requests = progress.requests.saturating_add(1);
        if retry {
            let changed = sqlx::query("UPDATE classification_attempts SET state='in_flight',detail='' WHERE run_id=? AND document=? AND state='retry_wait' AND input_hash=?")
                .bind(status.id).bind(book.document).bind(book.evidence.as_deref().map(hash)).execute(&mut *tx).await?.rows_affected();
            if changed != 1 {
                return Err(Error::Config("Classification is not awaiting retry".into()));
            }
            progress.retries = progress.retries.saturating_add(1);
        } else {
            sqlx::query("INSERT INTO classification_attempts(run_id,document,target,product_id,input_hash,state) VALUES(?,?,?,?,?,'in_flight')")
                .bind(status.id).bind(book.document).bind(&book.target).bind(&book.product_id).bind(book.evidence.as_deref().map(hash)).execute(&mut *tx).await?;
        }
        let changed=sqlx::query("UPDATE analysis_runs SET classification=? WHERE id=? AND state='running' AND algorithm=?")
            .bind(serde_json::to_string(&progress)?).bind(status.id).bind(&status.algorithm).execute(&mut *tx).await?.rows_affected();
        if changed != 1 {
            return Err(Error::Config("Classification is no longer running".into()));
        }
        self.commit(tx).await?;
        status.classification = Some(progress);
        Ok(())
    }
    /// Account a rejected response before redispatch; retain no generated content.
    pub async fn classification_response_error(
        &self,
        status: &mut Status,
        book: &Candidate,
        usage: Option<&crate::decisions::Usage>,
        invalid_model: Option<crate::responses::Model>,
    ) -> Result<()> {
        let mut tx = self.transaction().await?;
        let mut p = status
            .classification
            .clone()
            .ok_or_else(|| Error::Config("Not an online analysis run".into()))?;
        if let Some(u) = usage {
            p.input_tokens = p.input_tokens.saturating_add(u.input_tokens);
            p.output_tokens = p.output_tokens.saturating_add(u.output_tokens);
            p.cached_tokens = p
                .cached_tokens
                .saturating_add(u.input_tokens_details.cached_tokens);
            p.cache_write_tokens = p
                .cache_write_tokens
                .saturating_add(u.input_tokens_details.cache_write_tokens);
        }
        if let Some(model) = invalid_model {
            p.last_escalation = Some(if let Some(next) = model.next() {
                p.model_escalations = p.model_escalations.saturating_add(1);
                format!(
                    "Book #{}: invalid output from {}; escalating to {}.",
                    book.product_id,
                    model.name(),
                    next.name()
                )
            } else {
                p.exhausted_responses = p.exhausted_responses.saturating_add(1);
                format!(
                    "Book #{}: all four models returned invalid output; marked unsuccessful.",
                    book.product_id
                )
            });
        }
        let changed=sqlx::query("UPDATE analysis_runs SET classification=? WHERE id=? AND algorithm=? AND state='running' AND EXISTS(SELECT 1 FROM classification_attempts WHERE run_id=? AND document=? AND state='in_flight')")
            .bind(serde_json::to_string(&p)?).bind(status.id).bind(&status.algorithm).bind(status.id).bind(book.document).execute(&mut *tx).await?.rows_affected();
        if changed != 1 {
            return Err(Error::Config("Analysis request is no longer active".into()));
        }
        self.commit(tx).await?;
        status.classification = Some(p);
        Ok(())
    }
    pub async fn classification_retry_wait(
        &self,
        status: &Status,
        book: &Candidate,
        detail: &'static str,
    ) -> Result<()> {
        // Callers supply only fixed, value-free reasons, never provider error bodies.
        let mut tx = self.transaction().await?;
        let changed = sqlx::query("UPDATE classification_attempts SET state='retry_wait',detail=? WHERE run_id=? AND document=? AND state='in_flight' AND EXISTS(SELECT 1 FROM analysis_runs WHERE id=? AND state='running' AND algorithm=?)")
            .bind(detail).bind(status.id).bind(book.document).bind(status.id).bind(&status.algorithm).execute(&mut *tx).await?.rows_affected();
        if changed != 1 {
            return Err(Error::Config(
                "Classification request is no longer active".into(),
            ));
        }
        self.commit(tx).await
    }
    /// Commit one result, memberships, attempt and progress together. Refused or
    /// oversized books retain any previous good classification and remain visible failures.
    pub async fn classification_commit(
        &self,
        status: &mut Status,
        book: &Candidate,
        decision: Option<&Decision>,
        failure: Option<&str>,
    ) -> Result<()> {
        let system = System::from_algorithm(&status.algorithm)?;
        let mut tx = self.transaction().await?;
        let previous: Option<String> = sqlx::query_scalar(
            "SELECT state FROM classification_attempts WHERE run_id=? AND document=?",
        )
        .bind(status.id)
        .bind(book.document)
        .fetch_optional(&mut *tx)
        .await?;
        if previous.as_deref().is_some_and(|s| s != "in_flight") {
            return Err(Error::Config(
                "Classification attempt already completed".into(),
            ));
        }
        let mut progress = status
            .classification
            .clone()
            .ok_or_else(|| Error::Config("Not a classification run".into()))?;
        let mut chosen = None;
        if let Some(d) = decision {
            progress.input_tokens += d.usage.input_tokens;
            progress.output_tokens += d.usage.output_tokens;
            progress.cached_tokens += d.usage.input_tokens_details.cached_tokens;
            progress.cache_write_tokens += d.usage.input_tokens_details.cache_write_tokens;
            chosen = system.answer(d).map_err(|e| Error::Config(e.to_string()))?;
        }
        let now = Utc::now().to_rfc3339();
        let mut overview = status.overview.clone();
        let before = if overview.is_some() && chosen.is_some() {
            Some(super::overview::book(&mut tx, &book.target, &book.product_id).await?)
        } else {
            None
        };
        if let Some(selection) = &chosen {
            let category = selection.category;
            let confidence = selection.confidence;
            let probabilities = &selection.probabilities;
            if system.needs_dnd() {
                let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM book_classifications WHERE target=? AND product_id=? AND system='dnd' AND algorithm=? AND category IN ('dnd_only','dnd_and_others'))")
                    .bind(&book.target).bind(&book.product_id).bind(crate::dnd::ALGORITHM).fetch_one(&mut *tx).await?;
                if !valid {
                    return Err(Error::Config(
                        "D&D eligibility changed during analysis; run again".into(),
                    ));
                }
            }
            if let System::Warhammer(f) = system {
                let valid: bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM warhammer_scope WHERE target=? AND product_id=? AND system=?)").bind(&book.target).bind(&book.product_id).bind(f.key()).fetch_one(&mut *tx).await?;
                if !valid {
                    return Err(Error::Config(
                        "Warhammer eligibility changed during analysis; run again".into(),
                    ));
                }
            }
            if matches!(system, System::Osr) {
                let valid: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM osr_scope WHERE target=? AND product_id=?)",
                )
                .bind(&book.target)
                .bind(&book.product_id)
                .fetch_one(&mut *tx)
                .await?;
                if !valid {
                    return Err(Error::Config(
                        "OSR eligibility changed during analysis; run again".into(),
                    ));
                }
            }
            if let Some(scope) = system.rule_system() {
                let valid: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM earth_scope WHERE target=? AND product_id=? AND system=?)")
                    .bind(&book.target).bind(&book.product_id).bind(scope).fetch_one(&mut *tx).await?;
                if !valid {
                    return Err(Error::Config(
                        "Rule System eligibility changed during analysis; run again".into(),
                    ));
                }
            }
            let current: Option<String> = sqlx::query_scalar(
                "SELECT evidence FROM classification_source WHERE target=? AND product_id=?",
            )
            .bind(&book.target)
            .bind(&book.product_id)
            .fetch_optional(&mut *tx)
            .await?;
            if current.as_deref() != book.evidence.as_deref() {
                return Err(Error::Config("Book metadata changed during classification; run again to use the new evidence".into()));
            }
            sqlx::query("INSERT INTO book_classifications(target,product_id,system,category,algorithm,model,confidence,probabilities,input_hash,calculated_at,run_id) VALUES(?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(target,product_id,system) DO UPDATE SET category=excluded.category,algorithm=excluded.algorithm,model=excluded.model,confidence=excluded.confidence,probabilities=excluded.probabilities,input_hash=excluded.input_hash,calculated_at=excluded.calculated_at,run_id=excluded.run_id")
                .bind(&book.target).bind(&book.product_id).bind(system.key()).bind(category).bind(system.algorithm()).bind(crate::decisions::MODEL).bind(confidence).bind(serde_json::to_string(probabilities)?).bind(hash(book.evidence.as_deref().unwrap_or_default())).bind(&now).bind(status.id).execute(&mut *tx).await?;
            // OSR levels depend on the membership set. Identical reclassification
            // must retain them if a subsequent level rerun is cancelled/refused.
            let keep_members = if matches!(system, System::Osr) {
                let current: Vec<String> = sqlx::query_scalar("SELECT group_key FROM book_group_memberships WHERE target=? AND product_id=? AND system=? ORDER BY group_key")
                    .bind(&book.target).bind(&book.product_id).bind(system.key()).fetch_all(&mut *tx).await?;
                current
                    .iter()
                    .map(String::as_str)
                    .collect::<std::collections::BTreeSet<_>>()
                    == selection.members.iter().copied().collect()
            } else {
                false
            };
            if !keep_members {
                sqlx::query(
                "DELETE FROM book_group_memberships WHERE target=? AND product_id=? AND system=?",
            )
            .bind(&book.target)
            .bind(&book.product_id)
            .bind(system.key())
            .execute(&mut *tx)
            .await?;
                for member in &selection.members {
                    sqlx::query("INSERT INTO book_group_memberships(target,product_id,system,group_key) VALUES(?,?,?,?)").bind(&book.target).bind(&book.product_id).bind(system.key()).bind(member).execute(&mut *tx).await?;
                }
            }
            sqlx::query("UPDATE analysis_revision SET revision=revision+1 WHERE singleton=1")
                .execute(&mut *tx)
                .await?;
            system.count(category, &mut progress)?;
        }
        let good = chosen.is_some();
        let detail = failure.unwrap_or(if good {
            ""
        } else {
            "Model refused this book; no classification applied"
        });
        sqlx::query("INSERT INTO classification_attempts(run_id,document,target,product_id,input_hash,state,detail) VALUES(?,?,?,?,?,?,?) ON CONFLICT(run_id,document) DO UPDATE SET state=excluded.state,detail=excluded.detail")
            .bind(status.id).bind(book.document).bind(&book.target).bind(&book.product_id).bind(book.evidence.as_deref().map(hash)).bind(if good{"classified"}else{"unsuccessful"}).bind(detail).execute(&mut *tx).await?;
        let changed=sqlx::query("UPDATE analysis_runs SET last_id=max(last_id,?),processed=processed+1,scored=scored+?,skipped=skipped+?,classification=?,detail=? WHERE id=? AND state='running' AND algorithm=?")
            .bind(book.document).bind(i64::from(good)).bind(i64::from(!good)).bind(serde_json::to_string(&progress)?).bind(if good{String::new()}else{format!("Book #{}: {detail}",book.product_id)}).bind(status.id).bind(system.algorithm()).execute(&mut *tx).await?.rows_affected();
        if changed != 1 {
            return Err(Error::Config("Classification is no longer running".into()));
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
        status.classification = Some(progress);
        status.detail = if good {
            String::new()
        } else {
            format!("Book #{}: {detail}", book.product_id)
        };
        Ok(())
    }
}
