//! Cancellable explicit analysis orchestration; shares the application's mutation gate.
use super::*;
use crate::{analysis::Status as AnalysisStatus, storage::sqlite::Catalogue};

pub(super) struct Running {
    pub(super) cancel: CancellationToken,
    pub task: tokio::task::JoinHandle<()>,
}

impl Controller {
    /// Cache compact aggregates by catalogue identity/revision, separate from snapshots.
    pub async fn analysis_overview(&self) -> UiResult<crate::analysis::Overview> {
        let (_permit, db) = self.catalogue_reader()?;
        let mut cache = self.coverage.lock().await;
        let mut tx = db.pool.begin().await.map_err(message)?;
        let revision: i64 =
            sqlx::query_scalar("SELECT revision FROM analysis_revision WHERE singleton=1")
                .fetch_one(&mut *tx)
                .await
                .map_err(message)?;
        if let Some((identity, old, result)) = cache.as_ref()
            && *identity == db.catalogue
            && *old == revision
        {
            return Ok(result.clone());
        }
        let result = crate::storage::overview::read(&mut tx)
            .await
            .map_err(message)?;
        tx.commit().await.map_err(message)?;
        *cache = Some((db.catalogue, revision, result.clone()));
        Ok(result)
    }
    #[cfg(test)]
    pub(crate) fn install_test_decisions(&self, client: crate::decisions::Client) {
        assert!(self.decisions.set(client).is_ok());
    }
    /// Only this fixed synthetic check is exposed to the webview. Classification
    /// of catalogue data will be a separate explicit analysis, not arbitrary IPC.
    pub async fn test_decisions(&self) -> UiResult<crate::decisions::Decision> {
        let _operation = self.available()?;
        if self.tx.borrow().active || self.analysis.borrow().active() {
            return Err("Stop the current operation before testing Decisions".into());
        }
        let client = self
            .decisions
            .get_or_try_init(|| async { crate::decisions::Client::new() })
            .await
            .map_err(message)?;
        client
            .decide(&crate::decisions::diagnostic(), &self.closing)
            .await
            .map_err(message)
    }

    pub async fn start_analysis(&self) -> UiResult<()> {
        let _operation = self.available()?;
        let state = self.tx.borrow().clone();
        if state.active || self.analysis.borrow().active() {
            return Err("Stop the current operation before starting analysis".into());
        }
        if !state.storage_ok {
            return Err("Wait for local storage to be ready before starting analysis".into());
        }
        let cancel = self.closing.child_token();
        let stopped = cancel.clone();
        let paths = self.paths.clone();
        let progress = self.analysis.clone();
        let app = self.tx.clone();
        let preparing = AnalysisStatus {
            state: "preparing".into(),
            scope: "all".into(),
            query: None,
            detail: "Preparing selected books…".into(),
            ..Default::default()
        };
        progress.send_replace(preparing.clone());
        let task = tokio::spawn(async move {
            let mut status = preparing;
            let result: Result<()> = async {
                let db = Catalogue::connect_cancel(&paths, &stopped).await?;
                app.send_modify(|s| s.catalogue = Some(db.clone()));
                let result: Result<()> = async {
                    if stopped.is_cancelled() {
                        return Err(Error::Cancelled);
                    }
                    status = db.begin_analysis().await?;
                    progress.send_replace(status.clone());
                    loop {
                        if stopped.is_cancelled() {
                            return Err(Error::Cancelled);
                        }
                        let done = db.analysis_batch(&mut status).await?;
                        // Cancellation remains visible until the current transaction commits.
                        if stopped.is_cancelled() && !done {
                            progress.send_modify(|s| {
                                s.processed = status.processed;
                                s.scored = status.scored;
                                s.skipped = status.skipped;
                                s.overview = status.overview.clone();
                            });
                        } else {
                            let mut visible = status.clone();
                            // Keep the mutation gate closed until ownership is released.
                            if done {
                                visible.state = "running".into();
                                visible.detail = "Releasing catalogue ownership…".into();
                            }
                            progress.send_replace(visible);
                        }
                        if done {
                            break;
                        }
                        tokio::task::yield_now().await;
                    }
                    Ok(())
                }
                .await;
                if let Err(ref error) = result {
                    let phase = if matches!(error, Error::Cancelled) {
                        "cancelled"
                    } else {
                        "failed"
                    };
                    let detail = if matches!(error, Error::Cancelled) {
                        "Stopped. Completed batches are saved; run analysis again to finish."
                            .to_owned()
                    } else {
                        message(error)
                    };
                    if status.id.is_some() {
                        let _ = db.finish_analysis(&mut status, phase, &detail).await;
                    }
                }
                // Await all writes before releasing the generation fence and OS lock.
                let release = db.release().await;
                result.and(release)
            }
            .await;
            if let Err(error) = result {
                status.state = if matches!(error, Error::Cancelled) {
                    "cancelled"
                } else {
                    "failed"
                }
                .into();
                status.detail = if matches!(error, Error::Cancelled) {
                    "Stopped. Completed batches are saved; run analysis again to finish.".into()
                } else {
                    message(error)
                };
                status.finished_at = Some(chrono::Utc::now().to_rfc3339());
            }
            progress.send_replace(status);
        });
        self.inner.lock().await.analysis_task = Some(Running { cancel, task });
        Ok(())
    }
    pub async fn cancel_analysis(&self) {
        if let Some(run) = &self.inner.lock().await.analysis_task {
            run.cancel.cancel();
        }
        self.analysis.send_modify(|s| {
            if s.active() {
                s.state = "stopping".into();
                s.detail = if s.classification.is_some() {
                    "Stopping requests and saving completed analysis results…"
                } else {
                    "Finishing the current batch…"
                }
                .into();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::analysis;
    async fn populated() -> (tempfile::TempDir, Arc<Controller>) {
        let temp = tempfile::tempdir().unwrap();
        let paths = LocalPaths::at(temp.path());
        let db = Catalogue::connect(&paths).await.unwrap();
        let intent = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO intents(id,target,product_id,payload,generation,finalized) VALUES(?,'drivethru','1','{}',?,1)").bind(intent).bind(db.generation).execute(&db.pool).await.unwrap();
        sqlx::query("WITH RECURSIVE n(i) AS(VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<2501) INSERT INTO products(target,product_id,title,description_text,record,parser_version,record_schema_version,archive,fetched_at,last_checked_at,intent) SELECT 'drivethru',CAST(i AS TEXT),'Synthetic book','Complete description','{\"rating\":4.7,\"rating_count\":1000}','drivethru-rust-1',2,'{}','2026-01-01','2026-01-01',? FROM n").bind(intent).execute(&db.pool).await.unwrap();
        db.release().await.unwrap();
        db.pool.close().await;
        let controller = Controller::new(paths, false);
        controller.initialize().await.unwrap();
        (temp, controller)
    }
    async fn batch_started(controller: &Controller, updates: &mut watch::Receiver<AnalysisStatus>) {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                updates.changed().await.unwrap();
                if updates.borrow().processed >= 500 {
                    break;
                }
            }
        })
        .await
        .unwrap();
        assert!(controller.snapshot().await.analysis.active());
    }
    #[tokio::test]
    async fn analysis_is_explicit_blocks_mutators_and_cancels_with_totals_available() {
        let (_temp, c) = populated().await;
        assert_eq!(c.snapshot().await.analysis.state, "idle");
        let mut updates = c.analysis.subscribe();
        c.start_analysis().await.unwrap();
        assert!(c.start_analysis().await.is_err());
        assert!(c.start(true).await.is_err());
        assert!(c.initialize().await.is_err());
        batch_started(&c, &mut updates).await;
        assert_eq!(c.analysis_overview().await.unwrap().catalogue_total, 2501);
        c.cancel_analysis().await;
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while c.snapshot().await.analysis.active() {
                updates.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        let saved = c.snapshot().await;
        assert_eq!(saved.analysis.state, "cancelled");
        assert!(saved.analysis.processed >= 500 && saved.analysis.processed < 2501);
        assert_eq!(
            saved.analysis.overview.as_ref().unwrap().analyses[crate::analysis::ALGORITHM]
                .completed,
            saved.analysis.processed
        );
        assert!(!saved.active);
        assert_eq!(saved.state, "idle");
        let next = Catalogue::connect(&c.paths).await.unwrap();
        assert_eq!(
            analysis::latest(&next.pool).await.unwrap().processed,
            saved.analysis.processed
        );
        next.release().await.unwrap();
        next.pool.close().await;
        c.shutdown().await;
    }
    #[tokio::test]
    async fn overview_reads_saved_totals_and_invalidates_its_cache_after_results_or_captures_change()
     {
        let (_temp, c) = populated().await;
        let initial = c.analysis_overview().await.unwrap();
        assert_eq!(initial.catalogue_total, 2501);
        assert_eq!(initial.analyses[crate::analysis::ALGORITHM].completed, 0);
        assert!(c.decisions.get().is_none());
        let version = c.coverage.lock().await.as_ref().unwrap().1;
        c.analysis_overview().await.unwrap();
        assert_eq!(c.coverage.lock().await.as_ref().unwrap().1, version);
        let db = Catalogue::connect(&c.paths).await.unwrap();
        let mut run = db.begin_analysis().await.unwrap();
        while !db.analysis_batch(&mut run).await.unwrap() {}
        let totals = c.analysis_overview().await.unwrap();
        assert_eq!(totals.analyses[crate::analysis::ALGORITHM].completed, 2501);
        assert!(c.coverage.lock().await.as_ref().unwrap().1 > version);
        sqlx::query("DELETE FROM products WHERE product_id='1'")
            .execute(&db.pool)
            .await
            .unwrap();
        let totals = c.analysis_overview().await.unwrap();
        assert_eq!(
            (
                totals.catalogue_total,
                totals.analyses[crate::analysis::ALGORITHM].completed
            ),
            (2500, 2500)
        );
        assert_eq!(c.snapshot().await.analysis.state, "idle");
        assert!(c.decisions.get().is_none());
        db.release().await.unwrap();
        c.shutdown().await;
        assert!(c.analysis_overview().await.is_err());
    }
    #[tokio::test]
    async fn closing_cancels_analysis_awaits_batch_and_releases_ownership() {
        let (_temp, c) = populated().await;
        let mut updates = c.analysis.subscribe();
        c.start_analysis().await.unwrap();
        batch_started(&c, &mut updates).await;
        tokio::time::timeout(std::time::Duration::from_secs(5), c.shutdown())
            .await
            .unwrap();
        assert!(!c.snapshot().await.analysis.active());
        assert!(c.start_analysis().await.is_err());
        let next = Catalogue::connect(&c.paths).await.unwrap();
        let saved = analysis::latest(&next.pool).await.unwrap();
        assert_eq!(saved.state, "cancelled");
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM relevance_scores")
            .fetch_one(&next.pool)
            .await
            .unwrap();
        assert_eq!(count, saved.processed);
        next.release().await.unwrap();
        next.pool.close().await;
    }
}
