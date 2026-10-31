use crate::{auth::Cookies, config::Config, domain::*, scheduler};
use std::{collections::VecDeque, time::Instant};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
pub struct Status {
    pub state: String,
    pub detail: String,
    pub metrics: Metrics,
    pub events: VecDeque<String>,
    pub failures: Vec<String>,
    pub started: Instant,
    pub active: bool,
    pub storage_ok: bool,
    pub storage_checking: bool,
    pub catalogue_loaded: bool,
    pub reprocessed: u64,
    pub reprocess_failed: u64,
    pub catalogue: Option<crate::storage::sqlite::Catalogue>,
    pub run: Option<uuid::Uuid>,
    pub outdated: i64,
    pub downloads: usize,
    pub parsers: usize,
    pub pacing: Option<Pacing>,
}
impl Default for Status {
    fn default() -> Self {
        Self {
            state: "idle".into(),
            detail: "Opening local storage…".into(),
            metrics: Metrics::default(),
            events: VecDeque::new(),
            failures: vec![],
            started: Instant::now(),
            active: false,
            storage_ok: false,
            storage_checking: true,
            catalogue_loaded: false,
            reprocessed: 0,
            reprocess_failed: 0,
            catalogue: None,
            run: None,
            outdated: 0,
            downloads: 0,
            parsers: 0,
            pacing: None,
        }
    }
}
pub fn event(tx: &watch::Sender<Status>, message: impl Into<String>) {
    let text = crate::telemetry::display(&message.into());
    tx.send_modify(|s| {
        if s.events.len() == 100 {
            s.events.pop_front();
        }
        s.events.push_back(format!(
            "{} {text}",
            chrono::Local::now().format("%H:%M:%S")
        ));
    });
}
pub struct Running {
    pub stop: CancellationToken,
    pub force: CancellationToken,
    pub resume: mpsc::Sender<Cookies>,
    pub task: tokio::task::JoinHandle<()>,
}
pub fn start(c: Config, cookies: Cookies, reprocess: bool, tx: watch::Sender<Status>) -> Running {
    start_with_paths(c, cookies, reprocess, tx, None)
}
pub fn start_with_paths(
    c: Config,
    cookies: Cookies,
    reprocess: bool,
    tx: watch::Sender<Status>,
    paths: Option<crate::paths::LocalPaths>,
) -> Running {
    let stop = CancellationToken::new();
    let force = CancellationToken::new();
    let (resume, rx) = mpsc::channel(1);
    let st = stop.clone();
    let f = force.clone();
    tx.send_modify(|s| {
        s.active = true;
        s.started = Instant::now();
        s.state = "validating storage".into();
        s.detail.clear();
        s.pacing = None;
    });
    let task = tokio::spawn(async move {
        let timer_stop = st.clone();
        let timer_force = f.clone();
        let drain = c.runtime.drain_secs;
        let timer = tokio::spawn(async move {
            timer_stop.cancelled().await;
            tokio::time::sleep(std::time::Duration::from_secs(drain)).await;
            timer_force.cancel();
        });
        let result = match paths {
            Some(paths) => {
                scheduler::run_at(c, cookies, reprocess, tx.clone(), st, f, rx, paths).await
            }
            None => scheduler::run(c, cookies, reprocess, tx.clone(), st, f, rx).await,
        };
        timer.abort();
        if let Err(e) = result {
            event(&tx, e.to_string());
            tx.send_modify(|s| {
                s.state = "interrupted".into();
                s.detail = e.to_string();
            });
        }
        tx.send_modify(|s| s.active = false);
    });
    Running {
        stop,
        force,
        resume,
        task,
    }
}
