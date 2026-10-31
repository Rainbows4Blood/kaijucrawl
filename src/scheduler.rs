use crate::{
    app::{Status, event},
    auth::Cookies,
    config::Config,
    domain::*,
    http::{Guard, Network},
    parser::runtime::Runtime,
    storage::{archive::Archives, sqlite::Catalogue},
    targets::{drivethru as d, sitemap},
};
use chrono::Utc;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{RwLock, mpsc, watch},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
#[cfg(test)]
mod tests;

trait RuntimeFactory: Sync {
    fn network(
        &self,
        settings: &crate::config::RuntimeSettings,
        cookies: Cookies,
        guard: Guard,
    ) -> Result<Network>;
    fn paths(&self) -> Result<crate::paths::LocalPaths> {
        crate::paths::LocalPaths::current()
    }
    fn parser(&self, settings: &crate::config::RuntimeSettings) -> Result<Runtime>;
}
struct Production;
struct LocalProduction(crate::paths::LocalPaths);
impl RuntimeFactory for LocalProduction {
    fn paths(&self) -> Result<crate::paths::LocalPaths> {
        Ok(self.0.clone())
    }
    fn network(&self, s: &crate::config::RuntimeSettings, c: Cookies, g: Guard) -> Result<Network> {
        Production.network(s, c, g)
    }
    fn parser(&self, s: &crate::config::RuntimeSettings) -> Result<Runtime> {
        Production.parser(s)
    }
}
impl RuntimeFactory for Production {
    fn network(&self, s: &crate::config::RuntimeSettings, c: Cookies, g: Guard) -> Result<Network> {
        Network::new(s, c, g)
    }
    fn parser(&self, s: &crate::config::RuntimeSettings) -> Result<Runtime> {
        Runtime::new(s.parsers.min(s.workers), s.parse_deadline())
    }
}

struct Context {
    db: Arc<RwLock<Catalogue>>,
    archives: Archives,
    parser: Runtime,
    network: Network,
    hard: CancellationToken,
    config: Config,
    paths: crate::paths::LocalPaths,
}
pub async fn validate(c: &Config) -> Result<Catalogue> {
    validate_at(c, &crate::paths::LocalPaths::current()?).await
}
#[doc(hidden)]
pub async fn validate_at(c: &Config, paths: &crate::paths::LocalPaths) -> Result<Catalogue> {
    validate_cancel(c, paths, &CancellationToken::new()).await
}
pub async fn validate_cancel(
    c: &Config,
    paths: &crate::paths::LocalPaths,
    cancel: &CancellationToken,
) -> Result<Catalogue> {
    c.runtime.validate()?;
    let db = Catalogue::connect_cancel(paths, cancel).await?;
    let result = Archives::new(paths).test_cancel(cancel).await;
    let release = db.release().await;
    result.and(release)?;
    Ok(db)
}
async fn validate_archives(
    db: &Catalogue,
    archives: &Archives,
    cancel: &CancellationToken,
) -> Result<()> {
    let cancel = cancel.child_token();
    let test = archives.test_cancel(&cancel);
    tokio::pin!(test);
    let mut renewal = tokio::time::interval(Duration::from_secs(10));
    let mut lost = None;
    loop {
        tokio::select! {
            r=&mut test=>return match lost{Some(e)=>Err(e),None=>r},
            _=renewal.tick(),if lost.is_none()=>if let Err(e)=db.verify().await{lost=Some(e);cancel.cancel();}
        }
    }
}
async fn recover(ctx: &Context) -> Result<()> {
    let mut after = Uuid::nil();
    loop {
        let db = ctx.db.read().await.clone();
        let pending = db.pending_intents(after).await?;
        if pending.is_empty() {
            break;
        }
        for intent in pending {
            after = intent.id;
            if db.retire_completed_intent(&intent).await? {
                continue;
            }
            if ctx.hard.is_cancelled() {
                return Err(Error::Cancelled);
            }
            if let Some(body) = ctx.archives.read(&intent.archive).await? {
                let p = ctx
                    .parser
                    .parse(
                        body,
                        intent.product_id.clone(),
                        intent.api_url.clone(),
                        &ctx.hard,
                    )
                    .await?;
                db.commit_captures(vec![(intent, p)]).await?;
            } else {
                db.absent_intent(&intent).await?;
            }
        }
    }
    Ok(())
}
async fn job(ctx: Arc<Context>, claim: Claim) -> Result<Option<(Intent, Product)>> {
    let db = ctx.db.read().await.clone();
    db.verify().await?;
    let on_start: Guard = {
        let db = db.clone();
        let claim = claim.clone();
        Arc::new(move || {
            let db = db.clone();
            let claim = claim.clone();
            Box::pin(async move { db.begin_attempt(&claim).await })
        })
    };
    let url = if claim.kind == "product" {
        d::detail(&claim.identity)
    } else {
        claim.url.clone()
    };
    let response = ctx
        .network
        .get_job(
            &url,
            if claim.kind == "product" {
                Some(&claim.identity)
            } else {
                None
            },
            if claim.kind == "product" {
                PRODUCT_LIMIT
            } else {
                SITEMAP_LIMIT
            },
            &ctx.hard,
            on_start,
        )
        .await?;
    if claim.kind == "sitemap" {
        let (tx, mut rx) = mpsc::channel(2);
        let cancellation = ctx.hard.clone();
        let parse = tokio::task::spawn_blocking(move || {
            sitemap::parse(response.body.as_slice(), |entries| {
                if cancellation.is_cancelled() {
                    return Err(Error::Cancelled);
                }
                tx.blocking_send(entries).map_err(|_| Error::Cancelled)
            })
        });
        while let Some(entries) = rx.recv().await {
            db.discover(claim.run, &claim.url, entries).await?;
        }
        parse.await.map_err(|_| Error::Cancelled)??;
        db.outcome(&claim, None, ctx.config.runtime.attempts)
            .await?;
        Ok(None)
    } else {
        let fetched_at = Utc::now();
        let p = ctx
            .parser
            .parse(
                response.body.clone(),
                claim.identity.clone(),
                response.url.clone(),
                &ctx.hard,
            )
            .await?;
        let archives = ctx.archives.clone();
        let catalogue = db.catalogue;
        let target = claim.target.clone();
        let id = claim.identity.clone();
        let (archive, compressed) = tokio::task::spawn_blocking(move || {
            archives.prepare(catalogue, &target, &id, &response.body)
        })
        .await
        .map_err(|_| Error::Cancelled)??;
        let intent = Intent {
            id: Uuid::new_v4(),
            target: claim.target,
            product_id: claim.identity,
            fetched_at,
            api_url: response.url,
            http: response.http,
            archive,
        };
        tokio::select! {_=ctx.hard.cancelled()=>return Err(Error::Cancelled),r=db.intent(&intent)=>r?}
        tokio::select! {_=ctx.hard.cancelled()=>return Err(Error::Cancelled),r=ctx.archives.write_verified(&intent.archive,compressed)=>r?}
        Ok(Some((intent, p)))
    }
}
#[allow(clippy::too_many_arguments)]
pub async fn run(
    config: Config,
    cookies: Cookies,
    reprocess: bool,
    tx: watch::Sender<Status>,
    stop: CancellationToken,
    hard: CancellationToken,
    resume: mpsc::Receiver<Cookies>,
) -> Result<()> {
    run_with(
        config, cookies, reprocess, tx, stop, hard, resume, Production,
    )
    .await
}
#[allow(clippy::too_many_arguments)]
pub async fn run_at(
    config: Config,
    cookies: Cookies,
    reprocess: bool,
    tx: watch::Sender<Status>,
    stop: CancellationToken,
    hard: CancellationToken,
    resume: mpsc::Receiver<Cookies>,
    paths: crate::paths::LocalPaths,
) -> Result<()> {
    run_with(
        config,
        cookies,
        reprocess,
        tx,
        stop,
        hard,
        resume,
        LocalProduction(paths),
    )
    .await
}
#[allow(clippy::too_many_arguments)]
async fn run_with(
    config: Config,
    cookies: Cookies,
    reprocess: bool,
    tx: watch::Sender<Status>,
    stop: CancellationToken,
    hard: CancellationToken,
    resume: mpsc::Receiver<Cookies>,
    factory: impl RuntimeFactory,
) -> Result<()> {
    config.runtime.validate()?;
    let paths = factory.paths()?;
    let archives = Archives::new(&paths);
    let db = Catalogue::connect_cancel(&paths, &stop).await?;
    if let Err(e) = validate_archives(&db, &archives, &hard).await {
        let _ = db.release().await;
        return Err(e);
    }
    let run = db
        .new_run(
            &config.runtime,
            if reprocess { "reprocess" } else { "crawl" },
        )
        .await?;
    let outdated = db.outdated_count().await?;
    tx.send_modify(|s| {
        s.catalogue = Some(db.clone());
        s.catalogue_loaded = true;
        s.storage_checking = false;
        s.run = Some(run);
        s.outdated = outdated;
        s.reprocessed = 0;
        s.reprocess_failed = 0;
    });
    let db = Arc::new(RwLock::new(db));
    let allowed = Arc::new(AtomicBool::new(true));
    let guard: Guard = {
        let db = db.clone();
        let stop = stop.clone();
        let allowed = allowed.clone();
        Arc::new(move || {
            let db = db.clone();
            let stop = stop.clone();
            let allowed = allowed.clone();
            Box::pin(async move {
                if stop.is_cancelled() || !allowed.load(Ordering::Acquire) {
                    return Err(Error::Cancelled);
                }
                db.read().await.clone().verify().await
            })
        })
    };
    let cooldown = {
        let db = db.clone();
        Arc::new(move |seconds| {
            let db = db.clone();
            Box::pin(async move { db.read().await.clone().cooldown(d::KEY, seconds).await })
                as futures_util::future::BoxFuture<'static, Result<()>>
        })
    };
    let network = factory
        .network(&config.runtime, cookies, guard)?
        .with_cooldown(cooldown);
    let ctx = Arc::new(Context {
        db: db.clone(),
        archives,
        parser: factory.parser(&config.runtime)?,
        network,
        hard: hard.clone(),
        config: config.clone(),
        paths,
    });
    let heartbeat_stop = CancellationToken::new();
    let hb_stop = heartbeat_stop.clone();
    let hb_db = db.clone();
    let hb_allowed = allowed.clone();
    let (hb_tx, hb_rx) = mpsc::channel(1);
    let heartbeat = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(10));
        loop {
            tokio::select! {_ = hb_stop.cancelled()=>break,_ = interval.tick()=>{if let Err(e)=hb_db.read().await.clone().verify().await{hb_allowed.store(false,Ordering::Release);let _=hb_tx.try_send(e);}}}
        }
    });
    let result = drive(Engine {
        ctx: ctx.clone(),
        config,
        tx,
        stop,
        hard: hard.clone(),
        resume,
        hb_rx,
        allowed,
        run,
        reprocess,
    })
    .await;
    hard.cancel();
    heartbeat_stop.cancel();
    let _ = heartbeat.await;
    let release = ctx.db.read().await.clone().release().await;
    result.and(release)
}

struct Pause {
    reason: String,
    authentication: bool,
}
impl Pause {
    fn from_error(e: &Error) -> Self {
        Self {
            reason: e.to_string(),
            authentication: matches!(e,Error::Job(f) if f.pauses()),
        }
    }
    fn from_failure(f: &Failure) -> Self {
        Self {
            reason: f.to_string(),
            authentication: f.pauses(),
        }
    }
    fn storage(reason: &str) -> Self {
        Self {
            reason: reason.into(),
            authentication: false,
        }
    }
}
struct Engine {
    ctx: Arc<Context>,
    config: Config,
    tx: watch::Sender<Status>,
    stop: CancellationToken,
    hard: CancellationToken,
    resume: mpsc::Receiver<Cookies>,
    hb_rx: mpsc::Receiver<Error>,
    allowed: Arc<AtomicBool>,
    run: Uuid,
    reprocess: bool,
}
async fn drive(engine: Engine) -> Result<()> {
    let Engine {
        ctx,
        config,
        tx,
        stop,
        hard,
        mut resume,
        mut hb_rx,
        allowed,
        run,
        reprocess,
    } = engine;

    let mut setup = true;
    let mut paused: Option<Pause> = None;
    let mut jobs = JoinSet::new();
    let mut saves = Vec::new();
    let mut last_flush = tokio::time::Instant::now();
    let mut stopping = None;
    let mut checkpoint = String::new();
    let mut scan = std::collections::VecDeque::new();

    let mut pulse = tokio::time::interval(Duration::from_millis(100));
    let mut metrics_due = tokio::time::Instant::now();

    loop {
        let pacing = if reprocess {
            None
        } else {
            Some(ctx.network.pacing().await)
        };
        tx.send_modify(|s| {
            s.pacing = pacing;
            s.downloads = ctx.network.active();
            s.parsers = ctx
                .parser
                .active(config.runtime.parsers.min(config.runtime.workers));
        });

        if stop.is_cancelled() && stopping.is_none() {
            allowed.store(false, Ordering::Release);
            stopping =
                Some(tokio::time::Instant::now() + Duration::from_secs(config.runtime.drain_secs));
            tx.send_modify(|s| s.state = "stopping".into());
            event(&tx, "Stopping new requests; draining validated work");
        }

        if stopping.is_some_and(|deadline| tokio::time::Instant::now() >= deadline)
            || hard.is_cancelled()
        {
            hard.cancel();
        }

        if let Ok(e) = hb_rx.try_recv() {
            paused = Some(Pause::from_error(&e));
            allowed.store(false, Ordering::Release);
            event(&tx, e.to_string());
        }

        if setup && paused.is_none() && stopping.is_none() {
            let prepared = async {
                recover(&ctx).await?;
                ctx.db.read().await.clone().release_claims(run).await?;

                if !reprocess {
                    let remaining = ctx
                        .db
                        .read()
                        .await
                        .clone()
                        .remaining_cooldown(d::KEY)
                        .await?;
                    ctx.network.limiter.cooldown(remaining).await;
                    let roots = ctx.network.refresh_robots(&hard).await?;
                    for roots in roots.chunks(1000) {
                        let entries = roots
                            .iter()
                            .map(|location| sitemap::Entry {
                                location: location.clone(),
                                lastmod: None,
                                sitemap: true,
                            })
                            .collect();
                        ctx.db
                            .read()
                            .await
                            .clone()
                            .discover(run, "robots.txt", entries)
                            .await?;
                    }
                }
                Ok::<(), Error>(())
            }
            .await;

            setup = false;
            if let Err(e) = prepared {
                paused = Some(Pause::from_error(&e));
                allowed.store(false, Ordering::Release);
                event(&tx, e.to_string());
            }
        }

        if !saves.is_empty()
            && (saves.len() >= config.runtime.save_batch
                || last_flush.elapsed() >= Duration::from_millis(config.runtime.flush_ms)
                || stopping.is_some()
                || jobs.is_empty())
        {
            let batch = std::mem::take(&mut saves);
            match ctx.db.read().await.clone().commit_captures(batch).await {
                Ok(n) => event(&tx, format!("Saved {n} products with verified archives")),
                Err(e) => {
                    paused = Some(Pause::from_error(&e));
                    allowed.store(false, Ordering::Release);
                    event(&tx, e.to_string());
                }
            }
            last_flush = tokio::time::Instant::now();
        }

        if stopping.is_some() && jobs.is_empty() {
            break;
        }

        if paused.is_some() {
            ctx.network.suspend().await;
        }

        if paused.is_some() && jobs.is_empty() {
            let pause = paused.as_ref().ok_or(Error::Cancelled)?;
            let reason = &pause.reason;
            let auth = pause.authentication;
            let state = if auth {
                "authentication paused"
            } else {
                "storage paused"
            };

            tx.send_modify(|s| {
                s.state = state.into();
                s.detail = if auth {
                    format!("{reason}. Use Authentication → Test Access, then Resume.")
                } else {
                    format!("{reason}. Restore storage, then Resume, or Stop.")
                };
                s.storage_ok = auth;
            });
            let _ = ctx
                .db
                .read()
                .await
                .clone()
                .run_state(run, state, false)
                .await;

            let replacement = tokio::select! {
                _=stop.cancelled()=>continue,
                _=hard.cancelled()=>{stop.cancel();continue;},
                replacement=resume.recv()=>replacement,
            };
            let Some(cookies) = replacement else {
                stop.cancel();
                continue;
            };
            if ctx.db.read().await.clone().verify().await.is_err() {
                let _ = ctx.db.read().await.clone().release().await;
                match Catalogue::connect(&ctx.paths).await {
                    Ok(new) => {
                        tx.send_modify(|s| s.catalogue = Some(new.clone()));
                        *ctx.db.write().await = new;
                    }
                    Err(e) => {
                        event(&tx, e.to_string());
                        continue;
                    }
                }
            }
            if let Err(e) =
                validate_archives(&ctx.db.read().await.clone(), &ctx.archives, &hard).await
            {
                event(&tx, e.to_string());
                continue;
            }
            ctx.network.replace(cookies).await;
            allowed.store(true, Ordering::Release);
            paused = None;
            setup = true;
            let _ = ctx
                .db
                .read()
                .await
                .clone()
                .run_state(run, "running", false)
                .await;
            event(
                &tx,
                "Resuming the same run without resetting attempt budgets",
            );
            continue;
        }

        if paused.is_none() && stopping.is_none() {
            let state = if reprocess {
                "reprocessing"
            } else {
                "discovering / crawling"
            };
            tx.send_modify(|s| {
                s.state=state.into();s.storage_ok=true;
                s.detail="Successful products are first captures; discovery does not measure storefront completeness".into();
            });
            if reprocess {
                if scan.is_empty() {
                    match ctx
                        .db
                        .read()
                        .await
                        .clone()
                        .reprocess_page(&checkpoint)
                        .await
                    {
                        Ok(page) => scan.extend(page),
                        Err(e) => {
                            paused = Some(Pause::from_error(&e));
                            allowed.store(false, Ordering::Release);
                            event(&tx, e.to_string());
                            continue;
                        }
                    }
                    if scan.is_empty() {
                        break;
                    }
                }

                if let Some((target, id, old, archive)) = scan.pop_front() {
                    checkpoint = format!("{target}/{id}");

                    let result = async {
                        let body = ctx.archives.read(&archive).await?.ok_or(Error::Integrity)?;
                        let parsed = ctx
                            .parser
                            .parse(
                                body,
                                id.clone(),
                                old["api_url"].as_str().unwrap_or("").into(),
                                &hard,
                            )
                            .await;
                        let result = match parsed {
                            Ok(p) => Ok(p),
                            Err(Error::Job(f)) => Err(f),
                            Err(e) => return Err(e),
                        };
                        let failed = result.is_err();
                        ctx.db
                            .read()
                            .await
                            .clone()
                            .reprocess_commit(run, &target, &id, &old, result)
                            .await?;
                        tx.send_modify(|s| {
                            s.reprocessed += 1;
                            if failed {
                                s.reprocess_failed += 1;
                            }
                        });
                        Ok::<(), Error>(())
                    }
                    .await;

                    if let Err(e) = result {
                        paused = Some(Pause::from_error(&e));
                        allowed.store(false, Ordering::Release);
                        event(&tx, e.to_string());
                        scan.clear();
                        checkpoint.clear();
                    }
                }
            } else if jobs.len() < config.runtime.workers {
                match ctx
                    .db
                    .read()
                    .await
                    .clone()
                    .claim(
                        run,
                        (config.runtime.workers - jobs.len())
                            .min(config.runtime.claim_batch as usize)
                            as i64,
                        config.runtime.attempts,
                    )
                    .await
                {
                    Ok(claims) => {
                        for claim in claims {
                            let ctx = ctx.clone();
                            jobs.spawn(async move {
                                let result = job(ctx, claim.clone()).await;
                                (claim, result)
                            });
                        }
                    }
                    Err(e) => {
                        paused = Some(Pause::from_error(&e));
                        allowed.store(false, Ordering::Release);
                        event(&tx, e.to_string());
                    }
                }
            }
        }

        if metrics_due.elapsed() >= Duration::from_secs(1) {
            match ctx.db.read().await.clone().metrics(run).await {
                Ok(metrics) => {
                    let finished = !reprocess
                        && paused.is_none()
                        && jobs.is_empty()
                        && saves.is_empty()
                        && metrics.pending
                            + metrics.active
                            + metrics.retry
                            + metrics.persistence
                            + metrics.sitemaps_pending
                            == 0;

                    tx.send_modify(|s| {
                        s.metrics = metrics.clone();
                        if !reprocess
                            && jobs.is_empty()
                            && metrics.next_due.is_some_and(|t| t > Utc::now())
                        {
                            s.state = "waiting for retry".into();
                        }
                    });

                    if finished {
                        break;
                    }
                }
                Err(e) => {
                    paused = Some(Pause::from_error(&e));
                    allowed.store(false, Ordering::Release);
                    event(&tx, e.to_string());
                }
            }

            metrics_due = tokio::time::Instant::now();
        }

        let mut joined = tokio::select! {
            _=pulse.tick()=>None,
            joined=jobs.join_next(),if !jobs.is_empty()=>joined,
        };
        loop {
            match joined {
                Some(Ok((_, Ok(Some(capture))))) => saves.push(capture),
                Some(Ok((_, Ok(None)))) | Some(Ok((_, Err(Error::Cancelled)))) | None => {}
                Some(Ok((claim, Err(Error::Job(f))))) => {
                    event(
                        &tx,
                        format!("{}: {}", crate::telemetry::identity(&claim.identity), f),
                    );
                    if f.pauses() {
                        paused = Some(Pause::from_failure(&f));
                        allowed.store(false, Ordering::Release);
                    }
                    if let Err(e) = ctx
                        .db
                        .read()
                        .await
                        .clone()
                        .outcome(&claim, Some(&f), config.runtime.attempts)
                        .await
                    {
                        paused = Some(Pause::from_error(&e));
                        allowed.store(false, Ordering::Release);
                    }
                }
                Some(Ok((_, Err(e)))) => {
                    paused = Some(Pause::from_error(&e));
                    allowed.store(false, Ordering::Release);
                    event(&tx, e.to_string());
                }
                Some(Err(_)) => {
                    paused = Some(Pause::storage(
                        "Worker failed; saved claims remain recoverable",
                    ));
                    allowed.store(false, Ordering::Release);
                }
            }
            // Harvest already completed work before another queue/metrics round trip.
            // Keep the batch bounded and handle failures before claiming more work.
            if saves.len() >= config.runtime.save_batch {
                break;
            }
            joined = jobs.try_join_next();
            if joined.is_none() {
                break;
            }
        }
    }

    let db = ctx.db.read().await.clone();
    db.release_claims(run).await?;
    db.run_state(
        run,
        if stop.is_cancelled() {
            "stopped"
        } else {
            "finished"
        },
        true,
    )
    .await?;

    let metrics = db.metrics(run).await?;
    let failures = db.failures(run, 0).await?;
    tx.send_modify(|s| {
        s.state = if stop.is_cancelled() {
            "stopped"
        } else {
            "finished"
        }
        .into();
        s.metrics = metrics;
        s.failures = failures;
    });
    Ok::<(), Error>(())
}

pub struct AccessReport {
    pub text: String,
    pub passed: bool,
}
/// No catalogue handle: this diagnostic cannot mutate queue/product/archive state.
pub async fn test_access(
    config: &Config,
    cookies: Cookies,
    id: Option<String>,
) -> Result<AccessReport> {
    test_access_with(config, cookies, id, &Production).await
}
async fn test_access_with(
    config: &Config,
    cookies: Cookies,
    id: Option<String>,
    factory: &dyn RuntimeFactory,
) -> Result<AccessReport> {
    test_access_with_cancel(config, cookies, id, factory, &CancellationToken::new()).await
}
pub async fn test_access_cancel(
    config: &Config,
    cookies: Cookies,
    id: Option<String>,
    cancel: &CancellationToken,
) -> Result<AccessReport> {
    test_access_with_cancel(config, cookies, id, &Production, cancel).await
}
async fn test_access_with_cancel(
    config: &Config,
    cookies: Cookies,
    id: Option<String>,
    factory: &dyn RuntimeFactory,
    cancel: &CancellationToken,
) -> Result<AccessReport> {
    let id = id.map(|s| d::test_id(&s)).transpose()?;
    let loaded = cookies.loaded();
    let mut report = AccessReport {
        text: format!(
            "Cookies loaded: {}\nAuthenticated access: unverified (cookie loading and public HTTP 200 are not account signals)",
            if loaded { "yes" } else { "no" }
        ),
        passed: true,
    };
    if let Some(profile) = &config.browser {
        let source = if profile.browser == "Firefox" {
            match cookies.session_count {
                Some(count) => format!("database + {count} session snapshot cookies"),
                None => "database only (session snapshot unavailable)".into(),
            }
        } else {
            "persisted cookies only".into()
        };
        report.text.push_str(&format!(
            "\nCookie source: {} profile, container {}; {source}",
            profile.browser, profile.container,
        ));
    }
    for warning in &cookies.warnings {
        report
            .text
            .push_str(&format!("\nImport warning: {warning}"));
    }
    let mut scopes = vec![];
    for (label, origin) in [("Website", d::WEBSITE), ("API", d::API)] {
        let url = url::Url::parse(&format!("{origin}robots.txt"))
            .map_err(|_| Error::Config("Invalid adapter robots URL".into()))?;
        scopes.push((label, origin, url.to_string(), cookies.diagnostics(&url)));
    }
    let network = factory.network(
        &config.runtime,
        cookies,
        Arc::new(|| Box::pin(async { Ok(()) })),
    )?;
    let mut authentication_failed = false;
    for (label, origin, url, diagnostics) in scopes {
        report
            .text
            .push_str(&format!("\n\n{label} robots: {url}\n{diagnostics}"));
        match network.load_robots(origin, cancel).await {
            Ok((status, _)) => report.text.push_str(&format!(
                "\nRobots policy: HTTP {status} — {}",
                if status == 404 || status == 410 {
                    "absent; no restrictions declared"
                } else {
                    "loaded"
                },
            )),
            Err(e) => {
                report.passed = false;
                authentication_failed |=
                    matches!(&e, Error::Job(f) if f.kind == FailureKind::Authentication);
                report
                    .text
                    .push_str(&format!("\nRobots policy: {}", access_error(&e)));
            }
        }
    }
    if !report.passed {
        report.text.push_str("\n\nWebsite/API endpoints: not tested; required robots policies could not be established\nProduct response: not tested");
        if authentication_failed {
            report.text.push_str("\n\nNext: check the selected browser profile/container and cf_clearance eligibility above. Reimport after refreshing the session, or paste the complete Cookie header for each exact host.\nEligible cookies are attached to robots requests, but browser login/clearance does not guarantee this HTTP client is accepted. A challenge is not proof of missing cookies.");
        }
        return Ok(report);
    }
    report.text.push('\n');
    match network.get(d::WEBSITE, None, PRODUCT_LIMIT, cancel).await {
        Ok(r) => report
            .text
            .push_str(&format!("\nWebsite reachable: HTTP {}", r.status)),
        Err(e) => {
            report.passed = false;
            report
                .text
                .push_str(&format!("\nWebsite access: {}", access_error(&e)));
        }
    }
    if let Some(id) = id {
        match network
            .get(
                &d::detail(id.as_str()),
                Some(id.as_str()),
                PRODUCT_LIMIT,
                cancel,
            )
            .await
        {
            Ok(r) => {
                report
                    .text
                    .push_str(&format!("\nAPI reachable: HTTP {}", r.status));
                match factory
                    .parser(&config.runtime)?
                    .parse(r.body, id.as_str().into(), r.url, cancel)
                    .await
                {
                    Ok(_) => report.text.push_str(
                        "\nProduct response: parseable, identity and completeness verified",
                    ),
                    Err(e) => {
                        report.passed = false;
                        report.text.push_str(&format!("\nProduct response: {e}"));
                    }
                }
            }
            Err(e) => {
                report.passed = false;
                report.text.push_str(&format!(
                    "\nAPI access: {}\nProduct response: not parseable or unavailable",
                    access_error(&e),
                ));
            }
        }
    } else {
        report.text.push_str("\nAPI product endpoint / product response: not tested; enter a known product ID in this diagnostic");
    }
    Ok(report)
}
fn access_error(error: &Error) -> String {
    match error {
        Error::Job(Failure {
            status: Some(status),
            ..
        }) => format!("HTTP {status} — {error}"),
        _ => error.to_string(),
    }
}
