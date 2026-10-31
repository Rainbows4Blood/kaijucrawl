//! One explicit sequence, one ownership lifetime; existing algorithms do the work.
use super::*;
use crate::{
    analysis::{AnalysisWorkspace, DiscoveryProgress, Status as AnalysisStatus},
    decisions,
    storage::sqlite::Catalogue,
};
use classification::Kind;

impl AnalysisWorkspace {
    fn stages(self) -> &'static [Option<Kind>] {
        use crate::{
            dnd_context::Kind as Dnd, earth_context::Kind as Earth,
            earth_editions::Kind as Editions,
        };
        match self {
            Self::Discovery => &[
                None,
                Some(Kind::BookRole),
                Some(Kind::SoloPlay),
                Some(Kind::Players),
            ],
            Self::Dnd => &[
                Some(Kind::Dnd),
                Some(Kind::DndEditions),
                Some(Kind::DndSettings),
                Some(Kind::DndContext(Dnd::Levels)),
                Some(Kind::DndContext(Dnd::Locations)),
                Some(Kind::DndContext(Dnd::Date)),
            ],
            Self::WarhammerFantasy => &[
                Some(Kind::Warhammer(crate::warhammer::Family::Fantasy)),
                Some(Kind::WarhammerContext(
                    crate::warhammer_context::Kind::FantasyLocations,
                )),
                Some(Kind::WarhammerContext(
                    crate::warhammer_context::Kind::FantasyDate,
                )),
            ],
            Self::Warhammer40k => &[
                Some(Kind::Warhammer(crate::warhammer::Family::FortyK)),
                Some(Kind::WarhammerContext(
                    crate::warhammer_context::Kind::FortyKLocations,
                )),
                Some(Kind::WarhammerContext(
                    crate::warhammer_context::Kind::FortyKDate,
                )),
            ],
            Self::Osr => &[Some(Kind::Osr), Some(Kind::OsrLevels)],
            Self::Shadowrun => &[
                Some(Kind::EarthEditions(Editions::Shadowrun)),
                Some(Kind::EarthContext(Earth::ShadowrunLocation)),
                Some(Kind::EarthContext(Earth::ShadowrunDate)),
            ],
            Self::Coc => &[
                Some(Kind::EarthEditions(Editions::Coc)),
                Some(Kind::EarthContext(Earth::CocLocation)),
                Some(Kind::EarthContext(Earth::CocDate)),
            ],
        }
    }
}

impl Controller {
    pub async fn start_discovery_analysis(&self, rerun_all: bool) -> UiResult<()> {
        self.start_sequence(AnalysisWorkspace::Discovery, rerun_all)
            .await
    }
    pub async fn start_system_analysis(
        &self,
        workspace: AnalysisWorkspace,
        rerun_all: bool,
    ) -> UiResult<()> {
        if workspace == AnalysisWorkspace::Discovery {
            return Err("Use the Discovery action for system-independent analyses".into());
        }
        self.start_sequence(workspace, rerun_all).await
    }
    async fn start_sequence(&self, workspace: AnalysisWorkspace, rerun_all: bool) -> UiResult<()> {
        let _operation = self.available()?;
        if self.tx.borrow().active || self.analysis.borrow().active() {
            return Err("Stop the current operation before starting analysis".into());
        }
        if !self.tx.borrow().storage_ok {
            return Err("Wait for local storage before starting analysis".into());
        }
        if workspace != AnalysisWorkspace::Discovery {
            self.analysis_overview()
                .await?
                .require_discovery_complete()
                .map_err(message)?;
        }
        let client = self
            .decisions
            .get_or_try_init(|| async { decisions::Client::new() })
            .await
            .map_err(message)?
            .clone();
        client.check_credentials().map_err(message)?;
        let cancel = self.closing.child_token();
        let stopped = cancel.clone();
        let paths = self.paths.clone();
        let progress = self.analysis.clone();
        let app = self.tx.clone();
        let preparing = AnalysisStatus {
            discovery: Some(DiscoveryProgress {
                workspace,
                rerun_all,
                step: 0,
                completed_steps: 0,
                unsuccessful: 0,
            }),
            state: "preparing".into(),
            detail: format!("Preparing {} analysis…", workspace.name()),
            ..Default::default()
        };
        progress.send_replace(preparing.clone());
        let task = tokio::spawn(async move {
            let mut status = preparing;
            let result: Result<()> = async {
                let db = Catalogue::connect_cancel(&paths, &stopped).await?;
                app.send_modify(|s| s.catalogue = Some(db.clone()));
                let result =
                    run_sequence(&db, &client, workspace, &mut status, &progress, &stopped).await;
                if let Err(ref error) = result {
                    let (state, detail) = failure(error, workspace);
                    if status.id.is_some() {
                        let _ = db.finish_analysis(&mut status, state, &detail).await;
                    }
                }
                // Never allow another mutator or publish completion before drain/release.
                let release = db.release().await;
                result.and(release)
            }
            .await;
            match result {
                Ok(()) => {
                    status.state = "completed".into();
                    status.detail = format!(
                        "{} analysis finished. Completed results are saved in the catalogue.",
                        workspace.name()
                    );
                }
                Err(error) => {
                    let (state, detail) = failure(&error, workspace);
                    status.state = state.into();
                    status.detail = detail;
                }
            }
            status.finished_at = Some(chrono::Utc::now().to_rfc3339());
            progress.send_replace(status);
        });
        self.inner.lock().await.analysis_task = Some(analysis::Running { cancel, task });
        Ok(())
    }
}

fn failure(error: &Error, workspace: AnalysisWorkspace) -> (&'static str, String) {
    if matches!(error, Error::Cancelled) {
        (
            "cancelled",
            format!(
                "{} analysis cancelled. Saved results are kept; run again to continue missing work.",
                workspace.name()
            ),
        )
    } else {
        ("failed", message(error))
    }
}

async fn run_sequence(
    db: &Catalogue,
    client: &decisions::Client,
    workspace: AnalysisWorkspace,
    status: &mut AnalysisStatus,
    progress: &watch::Sender<AnalysisStatus>,
    stopped: &CancellationToken,
) -> Result<()> {
    let rerun_all = status.discovery.as_ref().is_some_and(|p| p.rerun_all);
    if workspace != AnalysisWorkspace::Discovery {
        db.require_discovery_complete().await?;
    }
    let mut unsuccessful = 0;
    for (step, kind) in workspace.stages().iter().copied().enumerate() {
        if stopped.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let overview = status.overview.clone();
        // A begin failure must not finish or mislabel the preceding completed run.
        *status = AnalysisStatus {
            algorithm: kind
                .map_or(crate::analysis::ALGORITHM, Kind::algorithm)
                .into(),
            state: "preparing".into(),
            overview,
            discovery: Some(DiscoveryProgress {
                workspace,
                rerun_all,
                step,
                completed_steps: step,
                unsuccessful,
            }),
            detail: format!("Preparing the next {} analysis…", workspace.name()),
            ..Default::default()
        };
        progress.send_replace(status.clone());
        let mut current = match kind {
            None => db.begin_relevance(rerun_all).await?,
            Some(kind) => kind.begin(db, rerun_all).await?,
        };
        current.discovery = status.discovery.clone();
        *status = current;
        progress.send_replace(status.clone());
        match kind {
            None => loop {
                if stopped.is_cancelled() {
                    return Err(Error::Cancelled);
                }
                let done = db.analysis_batch(status).await?;
                let mut visible = status.clone();
                visible.state = if stopped.is_cancelled() {
                    "stopping"
                } else {
                    "running"
                }
                .into();
                progress.send_replace(visible);
                if done {
                    break;
                }
                tokio::task::yield_now().await;
            },
            Some(kind) => {
                classification::run_workers(kind, db, client, status, progress, stopped).await?;
                db.finish_analysis(
                    status,
                    "completed",
                    "Analysis stage finished. Saved results are retained.",
                )
                .await?;
                unsuccessful += status.skipped;
            }
        }
        status.discovery = Some(DiscoveryProgress {
            workspace,
            rerun_all,
            step,
            completed_steps: step + 1,
            unsuccessful,
        });
        let mut visible = status.clone();
        visible.state = if stopped.is_cancelled() {
            "stopping"
        } else {
            "running"
        }
        .into();
        visible.detail = "Finishing the analysis stage…".into();
        progress.send_replace(visible);
    }
    if stopped.is_cancelled() {
        return Err(Error::Cancelled);
    }
    Ok(())
}
