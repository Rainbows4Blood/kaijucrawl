//! Desktop operations independent of the webview. Secrets never enter snapshots.
use crate::{
    app::{self, Status},
    auth::{self, BrowserProfile, Cookies},
    config::{Config, Secrets},
    domain::*,
    paths::LocalPaths,
    scheduler, telemetry,
};
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use tokio::sync::{Mutex, watch};
use tokio_util::sync::CancellationToken;

type UiResult<T> = std::result::Result<T, String>;
mod analysis;
mod classification;
mod discovery;
mod library;
fn message(e: impl std::fmt::Display) -> String {
    telemetry::display(&e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn shutdown_waits_for_drain_and_releases_ownership_before_returning() {
        let temp = tempfile::tempdir().unwrap();
        let paths = LocalPaths::at(temp.path());
        let controller = Controller::new(paths.clone(), false);
        controller.initialize().await.unwrap();
        let db = crate::storage::sqlite::Catalogue::connect(&paths)
            .await
            .unwrap();
        let stop = CancellationToken::new();
        let stopped = stop.clone();
        let (allow_finish, finish) = tokio::sync::oneshot::channel();
        let (resume, _) = tokio::sync::mpsc::channel(1);
        controller.inner.lock().await.running = Some(app::Running {
            stop,
            force: CancellationToken::new(),
            resume,
            task: tokio::spawn(async move {
                stopped.cancelled().await;
                let _ = finish.await;
                db.release().await.unwrap();
                db.pool.close().await;
            }),
        });
        controller.tx.send_modify(|s| s.active = true);
        let closing = controller.clone();
        let task = tokio::spawn(async move {
            closing.shutdown().await;
        });
        controller.closing.cancelled().await;
        assert!(controller.snapshot().await.closing);
        assert!(controller.start(false).await.is_err());
        assert!(!task.is_finished());
        assert!(
            crate::storage::sqlite::Catalogue::connect(&paths)
                .await
                .is_err()
        );
        allow_finish.send(()).unwrap();
        task.await.unwrap();
        let next = crate::storage::sqlite::Catalogue::connect(&paths)
            .await
            .unwrap();
        next.release().await.unwrap();
        next.pool.close().await;
    }
    #[tokio::test]
    async fn shutdown_waits_for_pending_configuration_and_blocks_late_commands() {
        let temp = tempfile::tempdir().unwrap();
        let controller = Controller::new(LocalPaths::at(temp.path()), false);
        let operation = controller.operation.lock().await;
        let closing = controller.clone();
        let task = tokio::spawn(async move {
            closing.shutdown().await;
        });
        controller.closing.cancelled().await;
        assert!(controller.snapshot().await.busy);
        assert!(controller.select_target(false).await.is_err());
        assert!(!task.is_finished());
        drop(operation);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn closing_during_storage_initialization_cancels_and_releases_ownership() {
        use crate::storage::sqlite::Catalogue;
        let temp = tempfile::tempdir().unwrap();
        let paths = LocalPaths::at(temp.path());
        let db = Catalogue::connect(&paths).await.unwrap();
        db.release().await.unwrap();
        let mut connection = db.pool.acquire().await.unwrap();
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut *connection)
            .await
            .unwrap();
        let controller = Controller::new(paths.clone(), false);
        let mut updates = controller.tx.subscribe();
        let boot = controller.clone();
        let opening = tokio::spawn(async move { boot.initialize().await });
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !updates.borrow().storage_checking {
                updates.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        let closing = controller.clone();
        let close = tokio::spawn(async move { closing.shutdown().await });
        controller.closing.cancelled().await;
        sqlx::query("ROLLBACK")
            .execute(&mut *connection)
            .await
            .unwrap();
        drop(connection);
        tokio::time::timeout(std::time::Duration::from_secs(5), close)
            .await
            .unwrap()
            .unwrap();
        assert!(opening.await.unwrap().is_err());
        let state = controller.snapshot().await;
        assert!(state.closing && !state.storage_checking && !state.storage_ok);
        let successor = Controller::new(paths, false);
        successor.initialize().await.unwrap();
        assert!(successor.snapshot().await.storage_ok);
        assert!(successor.snapshot().await.catalogue_loaded);
        successor.shutdown().await;
        db.pool.close().await;
    }
}

#[derive(Clone, Serialize)]
pub struct Snapshot {
    pub analysis: crate::analysis::Status,
    pub state: String,
    pub detail: String,
    pub metrics: Metrics,
    pub events: Vec<String>,
    pub active: bool,
    pub busy: bool,
    pub closing: bool,
    pub storage_ok: bool,
    pub storage_checking: bool,
    pub catalogue_loaded: bool,
    pub selected: bool,
    pub elapsed_secs: u64,
    pub downloads: usize,
    pub parsers: usize,
    pub workers: usize,
    pub website_delay_ms: Option<u64>,
    pub api_delay_ms: Option<u64>,
    pub cooldown_secs: u64,
    pub reprocessed: u64,
    pub reprocess_failed: u64,
    pub outdated: i64,
    pub can_resume: bool,
    pub auth_method: String,
    pub browser_name: Option<String>,
    pub has_manual_cookies: bool,
    pub remembered: bool,
    pub browser_container: u32,
    pub root: String,
    pub version: String,
    pub catalogue_version: i32,
    pub record_version: i32,
    pub parser_version: String,
}
#[derive(Clone, Serialize)]
pub struct ProfileChoice {
    pub id: usize,
    pub browser: String,
    pub name: String,
    pub containers: Vec<(u32, String)>,
}
#[derive(Clone, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AuthMethod {
    Anonymous,
    Manual,
    Browser,
}
#[derive(Clone, Deserialize)]
pub struct AuthDraft {
    pub method: AuthMethod,
    pub profile_id: Option<usize>,
    #[serde(default)]
    pub container: u32,
    #[serde(default)]
    pub website_cookie: String,
    #[serde(default)]
    pub api_cookie: String,
    #[serde(default)]
    pub keep_saved: bool,
    #[serde(default)]
    pub remember: bool,
    pub product_id: Option<String>,
}
impl Drop for AuthDraft {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.website_cookie.zeroize();
        self.api_cookie.zeroize();
    }
}
#[derive(Serialize)]
pub struct AccessResult {
    pub passed: bool,
    pub text: String,
}
#[derive(Serialize)]
pub struct FailurePage {
    pub rows: Vec<String>,
    pub next: Option<i64>,
}

struct Inner {
    analysis_task: Option<analysis::Running>,
    config: Config,
    secrets: Secrets,
    cookies: Cookies,
    profiles: Vec<BrowserProfile>,
    running: Option<app::Running>,
    initialized: bool,
    access_tested: bool,
    remembered: bool,
}
pub struct Controller {
    coverage: Mutex<Option<(uuid::Uuid, i64, crate::analysis::Overview)>>,
    decisions: tokio::sync::OnceCell<crate::decisions::Client>,
    analysis: watch::Sender<crate::analysis::Status>,
    paths: LocalPaths,
    production_load: bool,
    inner: Mutex<Inner>,
    operation: Mutex<()>,
    tx: watch::Sender<Status>,
    closing: CancellationToken,
    generation: AtomicU64,
    read_slots: tokio::sync::Semaphore,
}
impl Controller {
    pub fn new(paths: LocalPaths, production_load: bool) -> Arc<Self> {
        let (tx, _) = watch::channel(Status::default());
        let (analysis, _) = watch::channel(crate::analysis::Status::default());
        Arc::new(Self {
            coverage: Mutex::new(None),
            decisions: tokio::sync::OnceCell::new(),
            paths,
            analysis,
            production_load,
            tx,
            operation: Mutex::new(()),
            closing: CancellationToken::new(),
            generation: AtomicU64::new(0),
            read_slots: tokio::sync::Semaphore::new(2),
            inner: Mutex::new(Inner {
                analysis_task: None,
                config: Config::default(),
                secrets: Secrets::default(),
                cookies: Cookies::default(),
                profiles: vec![],
                running: None,
                initialized: false,
                access_tested: false,
                remembered: false,
            }),
        })
    }
    fn available(&self) -> UiResult<tokio::sync::MutexGuard<'_, ()>> {
        if self.closing.is_cancelled() {
            return Err("Application is closing".into());
        }
        self.operation
            .try_lock()
            .map_err(|_| "Another operation is in progress".into())
    }
    async fn configured(&self) -> UiResult<()> {
        if self.inner.lock().await.initialized {
            Ok(())
        } else {
            Err("Load or repair local settings before changing configuration".into())
        }
    }
    pub async fn snapshot(&self) -> Snapshot {
        let inner = self.inner.lock().await;
        let s = self.tx.borrow().clone();
        let pacing = s.pacing;
        Snapshot {
            analysis: self.analysis.borrow().clone(),
            state: message(&s.state),
            detail: message(&s.detail),
            metrics: s.metrics,
            events: s.events.iter().map(message).collect(),
            active: s.active,
            busy: self.operation.try_lock().is_err(),
            closing: self.closing.is_cancelled(),
            storage_ok: s.storage_ok,
            storage_checking: s.storage_checking,
            catalogue_loaded: s.catalogue_loaded,
            selected: inner.config.selected,
            elapsed_secs: if s.run.is_some() {
                s.started.elapsed().as_secs()
            } else {
                0
            },
            downloads: s.downloads,
            parsers: s.parsers,
            workers: pacing.map_or(inner.config.runtime.workers, |p| p.workers),
            website_delay_ms: pacing.and_then(|p| p.website).map(|v| v.as_millis() as u64),
            api_delay_ms: pacing.and_then(|p| p.api).map(|v| v.as_millis() as u64),
            cooldown_secs: pacing.map_or(0, |p| p.cooldown.as_secs()),
            reprocessed: s.reprocessed,
            reprocess_failed: s.reprocess_failed,
            outdated: s.outdated,
            can_resume: s.active
                && s.state.contains("paused")
                && (s.state != "authentication paused" || inner.access_tested),
            auth_method: if inner.config.browser.is_some() {
                "browser"
            } else if !inner.secrets.website_cookie.is_empty()
                || !inner.secrets.api_cookie.is_empty()
            {
                "manual"
            } else {
                "anonymous"
            }
            .into(),
            browser_name: inner.config.browser.as_ref().map(|p| {
                message(format!(
                    "{} · {} · container {}",
                    p.browser, p.name, p.container
                ))
            }),
            has_manual_cookies: !inner.secrets.website_cookie.is_empty()
                || !inner.secrets.api_cookie.is_empty(),
            remembered: inner.remembered,
            browser_container: inner.config.browser.as_ref().map_or(0, |p| p.container),
            root: message(self.paths.root().display()),
            version: env!("CARGO_PKG_VERSION").into(),
            catalogue_version: CATALOGUE_VERSION,
            record_version: SCHEMA_VERSION,
            parser_version: PARSER_VERSION.into(),
        }
    }
    pub async fn initialize(&self) -> UiResult<()> {
        let _operation = self.available()?;
        if self.tx.borrow().active || self.analysis.borrow().active() {
            return Err("Stop the current operation before checking storage".into());
        }
        self.tx.send_modify(|s| {
            s.storage_ok = false;
            s.storage_checking = true;
            s.state = "opening storage".into();
            s.detail = "Opening catalogue and checking archives…".into();
        });
        let result: UiResult<()> = async {
            if !self.inner.lock().await.initialized {
                let paths = self.paths.clone();
                let production = self.production_load;
                let (config, secrets) = tokio::task::spawn_blocking(move || -> Result<_> {
                    if production {
                        Ok((Config::load()?, Secrets::load()?))
                    } else {
                        Ok((Config::load_at(&paths, None)?, Secrets::load_at(&paths)?))
                    }
                })
                .await
                .map_err(message)?
                .map_err(message)?;
                let mut i = self.inner.lock().await;
                i.remembered = !secrets.website_cookie.is_empty() || !secrets.api_cookie.is_empty();
                i.config = config;
                i.secrets = secrets;
                i.initialized = true;
            }
            let config = self.inner.lock().await.config.clone();
            let db = scheduler::validate_cancel(&config, &self.paths, &self.closing)
                .await
                .map_err(message)?;
            if self.closing.is_cancelled() {
                return Err(message(Error::Cancelled));
            }
            let metrics = db.metrics(uuid::Uuid::nil()).await.map_err(message)?;
            let outdated = db.outdated_count().await.map_err(message)?;
            self.analysis.send_replace(
                crate::storage::analysis::latest(&db.pool)
                    .await
                    .map_err(message)?,
            );
            self.tx.send_modify(|s| {
                s.storage_ok = true;
                s.catalogue_loaded = true;
                s.state = "idle".into();
                s.metrics = metrics;
                s.outdated = outdated;
                s.catalogue = Some(db);
                s.detail = "Ready to continue your catalogue".into();
            });
            Ok(())
        }
        .await;
        self.tx.send_modify(|s| {
            s.storage_checking = false;
            if let Err(ref error) = result {
                s.detail = error.clone();
                s.state = if self.closing.is_cancelled() {
                    "closing"
                } else {
                    "storage error"
                }
                .into();
            }
        });
        result
    }
    pub async fn select_target(&self, selected: bool) -> UiResult<()> {
        let _operation = self.available()?;
        self.configured().await?;
        let mut config = self.inner.lock().await.config.clone();
        config.selected = selected;
        let paths = self.paths.clone();
        let saved = config.clone();
        tokio::task::spawn_blocking(move || saved.save_at(&paths))
            .await
            .map_err(message)?
            .map_err(message)?;
        self.inner.lock().await.config = config;
        Ok(())
    }
    pub async fn profiles(&self) -> UiResult<Vec<ProfileChoice>> {
        let _operation = self.available()?;
        self.configured().await?;
        let current = self.inner.lock().await.config.browser.clone();
        let (profiles, choices) = tokio::task::spawn_blocking(move || {
            let mut profiles = auth::discover_profiles();
            if let Some(current) = current {
                profiles.retain(|p| p.path != current.path || p.browser != current.browser);
                profiles.insert(0, current);
            }
            let choices = profiles
                .iter()
                .enumerate()
                .map(|(id, p)| ProfileChoice {
                    id,
                    browser: message(&p.browser),
                    name: message(&p.name),
                    containers: auth::containers(p)
                        .into_iter()
                        .map(|(id, name)| (id, message(name)))
                        .collect(),
                })
                .collect();
            (profiles, choices)
        })
        .await
        .map_err(message)?;
        self.inner.lock().await.profiles = profiles;
        Ok(choices)
    }
    async fn draft(&self, draft: &AuthDraft) -> UiResult<(Config, Secrets)> {
        self.configured().await?;
        let i = self.inner.lock().await;
        let mut config = i.config.clone();
        config.browser = None;
        let mut secrets = Secrets::default();
        match draft.method {
            AuthMethod::Anonymous => (),
            AuthMethod::Manual => {
                secrets = if draft.keep_saved {
                    i.secrets.clone()
                } else {
                    Secrets {
                        website_cookie: draft.website_cookie.clone(),
                        api_cookie: draft.api_cookie.clone(),
                    }
                };
                Cookies::manual(&secrets).map_err(message)?;
            }
            AuthMethod::Browser => {
                let mut profile = match draft.profile_id {
                    Some(id) => i.profiles.get(id).cloned(),
                    None => i.config.browser.clone(),
                }
                .ok_or("Choose a browser profile")?;
                profile.container = draft.container;
                let checked = profile.clone();
                drop(i);
                let containers = tokio::task::spawn_blocking(move || auth::containers(&checked))
                    .await
                    .map_err(message)?;
                if !containers.iter().any(|(id, _)| *id == profile.container) {
                    return Err("Choose a listed Firefox container".into());
                }
                config.browser = Some(profile);
            }
        }
        Ok((config, secrets))
    }
    async fn cookies(config: Config, secrets: Secrets) -> UiResult<Cookies> {
        tokio::task::spawn_blocking(move || match config.browser {
            Some(profile) => auth::import(&profile),
            None => Cookies::manual(&secrets),
        })
        .await
        .map_err(message)?
        .map_err(message)
    }
    pub async fn save_auth(&self, draft: AuthDraft) -> UiResult<()> {
        let _operation = self.available()?;
        let (config, secrets) = self.draft(&draft).await?;
        let paths = self.paths.clone();
        let c = config.clone();
        let persisted = if draft.remember && draft.method == AuthMethod::Manual {
            secrets.clone()
        } else {
            Secrets::default()
        };
        tokio::task::spawn_blocking(move || -> Result<()> {
            c.save_at(&paths)?;
            persisted.persist_at(&paths)
        })
        .await
        .map_err(message)?
        .map_err(message)?;
        let mut i = self.inner.lock().await;
        i.config = config;
        i.secrets = secrets;
        i.cookies = Cookies::default();
        i.access_tested = false;
        i.remembered = draft.remember && draft.method == AuthMethod::Manual;
        Ok(())
    }
    pub async fn clear_auth(&self) -> UiResult<()> {
        self.save_auth(AuthDraft {
            method: AuthMethod::Anonymous,
            profile_id: None,
            container: 0,
            website_cookie: String::new(),
            api_cookie: String::new(),
            keep_saved: false,
            remember: false,
            product_id: None,
        })
        .await
    }
    /// Tests the saved credentials; an unsaved wizard draft is never applied implicitly.
    pub async fn test_access(&self, product_id: Option<String>) -> UiResult<AccessResult> {
        let _operation = self.available()?;
        self.configured().await?;
        let state = self.tx.borrow().clone();
        if state.active && !state.state.contains("paused") {
            return Err("Stop crawling before testing access".into());
        }
        let (config, secrets) = {
            let mut i = self.inner.lock().await;
            i.access_tested = false;
            (i.config.clone(), i.secrets.clone())
        };
        let cookies = Self::cookies(config.clone(), secrets).await?;
        let id = match (product_id.filter(|s| !s.trim().is_empty()), state.catalogue) {
            (Some(id), _) => Some(id),
            (None, Some(db)) => db.known_product().await.map_err(message)?,
            _ => None,
        };
        let report = scheduler::test_access_cancel(&config, cookies.clone(), id, &self.closing)
            .await
            .map_err(message)?;
        let mut i = self.inner.lock().await;
        i.cookies = cookies;
        i.access_tested = report.passed;
        Ok(AccessResult {
            passed: report.passed,
            text: message(report.text),
        })
    }
    pub async fn start(&self, reprocess: bool) -> UiResult<()> {
        let _operation = self.available()?;
        let sequence = self.generation.load(Ordering::SeqCst);
        let s = self.tx.borrow().clone();
        if s.active || self.analysis.borrow().active() {
            return Err("An operation is already running".into());
        }
        if !s.storage_ok {
            return Err("Check local storage before starting".into());
        }
        let (config, secrets) = {
            let i = self.inner.lock().await;
            (i.config.clone(), i.secrets.clone())
        };
        if !reprocess && !config.selected {
            return Err("Select DriveThruRPG before starting".into());
        }
        let cookies = if reprocess {
            Cookies::default()
        } else {
            Self::cookies(config.clone(), secrets).await?
        };
        let mut i = self.inner.lock().await;
        if self.closing.is_cancelled() || self.generation.load(Ordering::SeqCst) != sequence {
            return Err("Start cancelled".into());
        }
        i.cookies = cookies.clone();
        i.access_tested = false;
        i.running = Some(app::start_with_paths(
            config,
            cookies,
            reprocess,
            self.tx.clone(),
            Some(self.paths.clone()),
        ));
        Ok(())
    }
    pub async fn stop(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        if let Some(run) = &self.inner.lock().await.running {
            run.stop.cancel();
        }
        self.tx.send_modify(|s| {
            if s.active {
                s.state = "stopping".into();
                s.detail = "Finishing valid work and releasing catalogue ownership…".into();
            }
        });
    }
    pub async fn resume(&self) -> UiResult<()> {
        let _operation = self.available()?;
        let i = self.inner.lock().await;
        let s = self.tx.borrow().clone();
        if !s.active || !s.state.contains("paused") {
            return Err("No paused run to resume".into());
        }
        if s.state == "authentication paused" && !i.access_tested {
            return Err("Save credentials and pass Test Access before resuming".into());
        }
        i.running
            .as_ref()
            .ok_or("No active run")?
            .resume
            .try_send(i.cookies.clone())
            .map_err(|_| "Resume already pending".to_owned())
    }
    pub async fn failures(&self, after: i64) -> UiResult<FailurePage> {
        let s = self.tx.borrow().clone();
        if let (Some(db), Some(run)) = (s.catalogue, s.run) {
            let (rows, next) = db.failure_page(run, after.max(0)).await.map_err(message)?;
            Ok(FailurePage {
                rows: rows.into_iter().map(message).collect(),
                next,
            })
        } else {
            Ok(FailurePage {
                rows: vec![],
                next: None,
            })
        }
    }
    fn catalogue_reader(
        &self,
    ) -> UiResult<(
        tokio::sync::SemaphorePermit<'_>,
        crate::storage::sqlite::Catalogue,
    )> {
        if self.closing.is_cancelled() {
            return Err("Application is closing".into());
        }
        let permit = self
            .read_slots
            .try_acquire()
            .map_err(|_| "Catalogue reads are busy; try again shortly".to_owned())?;
        let db = self
            .tx
            .borrow()
            .catalogue
            .clone()
            .ok_or("Local catalogue is still opening; retry when storage is ready")?;
        Ok((permit, db))
    }
    pub async fn shutdown(&self) {
        self.closing.cancel();
        self.cancel_analysis().await;
        self.stop().await;
        // Wait for pending initialization/import/save and prevent a late start.
        let _operation = self.operation.lock().await;
        let running = self.inner.lock().await.running.take();
        let analysis = self.inner.lock().await.analysis_task.take();
        if let Some(run) = analysis {
            let _ = run.task.await;
        }
        if let Some(run) = running {
            let _ = run.task.await;
        }
    }
}
