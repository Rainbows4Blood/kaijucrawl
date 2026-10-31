//! Bounded parallel classification; a single coordinator owns durable progress.
use super::*;
use crate::{
    analysis::Status as AnalysisStatus, classification, decisions, storage::sqlite::Catalogue,
};
use std::{collections::VecDeque, time::Duration};
use tokio::task::JoinSet;
mod kind;
mod retry;
pub(super) use kind::Kind;

struct Work {
    book: crate::storage::classification::Candidate,
    request: kind::Request,
    failures: u32,
}
impl Controller {
    pub async fn start_classification(&self, reclassify: bool) -> UiResult<()> {
        self.start_online_analysis(Kind::BookRole, reclassify).await
    }
    pub async fn start_solo_classification(&self, reclassify: bool) -> UiResult<()> {
        self.start_online_analysis(Kind::SoloPlay, reclassify).await
    }
    pub async fn start_dnd_classification(&self, reclassify: bool) -> UiResult<()> {
        self.start_online_analysis(Kind::Dnd, reclassify).await
    }
    pub async fn start_dnd_editions(&self, reclassify: bool) -> UiResult<()> {
        self.start_online_analysis(Kind::DndEditions, reclassify)
            .await
    }
    pub async fn start_earth_editions(
        &self,
        kind: crate::earth_editions::Kind,
        reclassify: bool,
    ) -> UiResult<()> {
        self.start_online_analysis(Kind::EarthEditions(kind), reclassify)
            .await
    }
    pub async fn start_dnd_settings(&self, reclassify: bool) -> UiResult<()> {
        self.start_online_analysis(Kind::DndSettings, reclassify)
            .await
    }
    pub async fn start_dnd_extraction(
        &self,
        kind: crate::dnd_context::Kind,
        reanalyze: bool,
    ) -> UiResult<()> {
        self.start_online_analysis(Kind::DndContext(kind), reanalyze)
            .await
    }
    pub async fn start_player_analysis(&self, reanalyze: bool) -> UiResult<()> {
        self.start_online_analysis(Kind::Players, reanalyze).await
    }
    pub async fn start_earth_extraction(
        &self,
        kind: crate::earth_context::Kind,
        reanalyze: bool,
    ) -> UiResult<()> {
        self.start_online_analysis(Kind::EarthContext(kind), reanalyze)
            .await
    }
    async fn start_online_analysis(&self, kind: Kind, reclassify: bool) -> UiResult<()> {
        let _operation = self.available()?;
        if self.tx.borrow().active || self.analysis.borrow().active() {
            return Err("Stop the current operation before starting classification".into());
        }
        if !self.tx.borrow().storage_ok {
            return Err("Wait for local storage before starting classification".into());
        }
        if kind.requires_discovery() {
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
            algorithm: kind.algorithm().into(),
            state: "preparing".into(),
            detail: "Checking saved analysis results…".into(),
            classification: Some(classification::Progress {
                workers: client.worker_limit(),
                reclassify,
                ..Default::default()
            }),
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
                    if kind.requires_discovery() {
                        db.require_discovery_complete().await?;
                    }
                    status = kind.begin(&db, reclassify).await?;
                    progress.send_replace(status.clone());
                    run_workers(kind, &db, &client, &mut status, &progress, &stopped).await?;
                    db.finish_analysis(
                        &mut status, "completed",
                        "Analysis finished. Results are saved in the catalogue.",
                    ).await?;
                    Ok(())
                }.await;
                if let Err(ref error)=result {
                    let (state,detail)=if matches!(error,Error::Cancelled){("cancelled","Stopped. Saved results are kept; run again to analyze remaining books.".into())}else{("failed",message(error))};
                    if status.id.is_some(){let _=db.finish_analysis(&mut status,state,&detail).await;}
                }
                let release=db.release().await;result.and(release)
            }.await;
            if let Err(error) = result {
                status.state = if matches!(error, Error::Cancelled) {
                    "cancelled"
                } else {
                    "failed"
                }
                .into();
                status.detail = if matches!(error, Error::Cancelled) {
                    "Stopped. Saved results are kept; run again to analyze remaining books.".into()
                } else {
                    message(error)
                };
                status.finished_at = Some(chrono::Utc::now().to_rfc3339());
            }
            progress.send_replace(status);
        });
        self.inner.lock().await.analysis_task = Some(analysis::Running { cancel, task });
        Ok(())
    }
}

/// Only this coordinator touches status/SQLite; workers own HTTP and bounded evidence.
pub(super) async fn run_workers(
    kind: Kind,
    db: &Catalogue,
    client: &decisions::Client,
    status: &mut AnalysisStatus,
    progress: &watch::Sender<AnalysisStatus>,
    stopped: &CancellationToken,
) -> Result<()> {
    let workers = client.worker_limit();
    if let Some(p) = &mut status.classification {
        p.workers = workers;
    }
    let requests_stop = stopped.child_token();
    let mut tasks = JoinSet::new();
    let mut retry_queue: VecDeque<Work> = VecDeque::new();
    let mut cooldown = retry::Cooldown::default();
    let mut after = 0;
    let mut exhausted = false;
    let mut failure = None;
    let mut storage_failed = false;
    loop {
        if stopped.is_cancelled() && failure.is_none() {
            failure = Some(Error::Cancelled);
        }
        if failure.is_some() {
            requests_stop.cancel();
        }
        // Consume ready responses before admitting more paid work. Each completed task
        // still occupies its slot until its result has been durably saved.
        let ready = tasks.try_join_next();
        if ready.is_none()
            && failure.is_none()
            && cooldown.remaining().is_zero()
            && tasks.len() < workers
            && (!retry_queue.is_empty()
                || (!exhausted && tasks.len() + retry_queue.len() < workers))
        {
            let admission: Result<()> = async {
                if stopped.is_cancelled() {
                    return Err(Error::Cancelled);
                }
                let work = if let Some(work) = retry_queue.pop_front() {
                    db.classification_redispatch(status, &work.book).await?;
                    work
                } else {
                    let Some(book) = kind.next(db, status, after).await? else {
                        exhausted = true;
                        return Ok(());
                    };
                    after = book.document;
                    let request = book
                        .evidence
                        .as_deref()
                        .map(serde_json::from_str)
                        .transpose()?;
                    let Some(request) = request else {
                        kind.commit(
                            db,
                            status,
                            &book,
                            None,
                            Some("Metadata exceeds the evidence size limit; no API request sent"),
                        )
                        .await?;
                        return Ok(());
                    };
                    let request = match kind.request(request) {
                        Ok(request) => request,
                        Err(error) => {
                            kind.commit(db, status, &book, None, Some(&error.to_string()))
                                .await?;
                            return Ok(());
                        }
                    };
                    if stopped.is_cancelled() {
                        return Err(Error::Cancelled);
                    }
                    db.classification_dispatch(status, &book).await?;
                    Work {
                        book,
                        request,
                        failures: 0,
                    }
                };
                db.verify().await?;
                let client = client.clone();
                let cancel = requests_stop.clone();
                tasks.spawn(async move {
                    let decision = work.request.send(&client, &cancel).await;
                    (work, decision)
                });
                Ok(())
            }
            .await;
            if let Err(error) = admission {
                failure = Some(error);
            }
            publish_retry(status, retry_queue.len(), &cooldown);
            publish(status, progress, tasks.len(), failure.is_some());
            continue;
        }
        let completion = match ready {
            Some(result) => result,
            None if tasks.is_empty()
                && (failure.is_some() || (exhausted && retry_queue.is_empty())) =>
            {
                break;
            }
            None => {
                let wait = cooldown.remaining();
                tokio::select! {
                    biased;
                    result = tasks.join_next(), if !tasks.is_empty() => {
                        let Some(result) = result else { break };
                        result
                    }
                    _ = stopped.cancelled(), if failure.is_none() => {
                        failure = Some(Error::Cancelled);
                        continue;
                    }
                    _ = tokio::time::sleep(wait.min(Duration::from_secs(1))),
                        if failure.is_none() && (!wait.is_zero() || !retry_queue.is_empty()) => {
                        publish_retry(status, retry_queue.len(), &cooldown);
                        publish(status, progress, tasks.len(), false);
                        continue;
                    }
                }
            }
        };
        match completion {
            Ok((work, Ok(decision))) if !storage_failed => {
                // Preserve already-received successes even while cancellation/errors drain.
                if let Err(error) = kind
                    .commit(db, status, &work.book, Some(&decision), None)
                    .await
                {
                    storage_failed = true;
                    failure = Some(error);
                }
            }
            Ok((mut work, Err(rejected))) if !storage_failed => {
                let error = rejected.error;
                let invalid = matches!(
                    error,
                    decisions::Error::InvalidResponse | decisions::Error::ResponseTooLarge
                );
                let current = match work.request.model() {
                    Ok(model) => model,
                    Err(error) => {
                        failure = Some(Error::Config(error.to_string()));
                        None
                    }
                };
                let escalating = if invalid && failure.is_none() {
                    current
                } else {
                    None
                };
                if (rejected.usage.is_some() || escalating.is_some())
                    && let Err(error) = db
                        .classification_response_error(
                            status,
                            &work.book,
                            rejected.usage.as_ref(),
                            escalating,
                        )
                        .await
                {
                    storage_failed = true;
                    failure = Some(error);
                }
                if failure.is_none() {
                    if escalating == Some(crate::responses::Model::Astra) {
                        if let Err(error)=kind.commit(db,status,&work.book,None,Some("Invalid structured output after Luna, Terra, Sol and Astra; run again to retry this book")).await {
                            storage_failed=true;failure=Some(error);
                        }
                    } else {
                        if escalating.is_some() {
                            if let Err(error) = work.request.escalate() {
                                failure = Some(Error::Config(error.to_string()));
                            }
                            work.failures = 0;
                        }
                        if failure.is_none() {
                            if let Some(cause) = error.retry_hint() {
                                work.failures = work.failures.saturating_add(1);
                                let jitter = (uuid::Uuid::new_v4().as_u128() % 1001) as u64;
                                cooldown.extend(
                                    retry::delay(work.failures, cause.after, jitter),
                                    cause,
                                );
                                match db
                                    .classification_retry_wait(status, &work.book, cause.reason)
                                    .await
                                {
                                    Ok(()) => retry_queue.push_back(work),
                                    Err(error) => {
                                        storage_failed = true;
                                        failure = Some(error);
                                    }
                                }
                            } else {
                                failure = Some(if matches!(error, decisions::Error::Cancelled) {
                                    Error::Cancelled
                                } else {
                                    Error::Config(error.to_string())
                                });
                            }
                        }
                    }
                }
            }
            Err(_) if failure.is_none() => {
                failure = Some(Error::Config(
                    "Classification worker stopped unexpectedly".into(),
                ));
            }
            _ => {}
        }
        publish_retry(status, retry_queue.len(), &cooldown);
        publish(status, progress, tasks.len(), failure.is_some());
    }
    publish_retry(status, 0, &retry::Cooldown::default());
    publish(status, progress, 0, failure.is_some());
    failure.map_or(Ok(()), Err)
}

fn publish_retry(status: &mut AnalysisStatus, queued: usize, cooldown: &retry::Cooldown) {
    if let Some(p) = &mut status.classification {
        p.retrying = queued;
        p.retry_in_secs = cooldown.seconds();
        let cause = if queued > 0 || p.retry_in_secs > 0 {
            cooldown.cause
        } else {
            None
        };
        p.retry_status = cause.and_then(|c| c.status);
        p.retry_reason = cause.map(|c| c.reason.into());
    }
}

fn publish(
    status: &mut AnalysisStatus,
    progress: &watch::Sender<AnalysisStatus>,
    active: usize,
    stopping: bool,
) {
    if let Some(p) = &mut status.classification {
        p.in_flight = active;
    }
    let mut visible = status.clone();
    if stopping {
        visible.state = "stopping".into();
        visible.detail = "Stopping requests and saving completed analysis results…".into();
    }
    progress.send_replace(visible);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decisions::tests::{http, server};
    use serde_json::json;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
    };

    async fn parallel_fixture(n: i64) -> (tempfile::TempDir, Arc<Controller>, TcpListener) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = decisions::Client::local_with_workers(
            format!("http://{}/v1/decisions", listener.local_addr().unwrap()),
            3,
        );
        let (temp, c) = fixture(client, n).await;
        (temp, c, listener)
    }
    async fn receive(listener: &TcpListener) -> (TcpStream, serde_json::Value) {
        tokio::time::timeout(std::time::Duration::from_secs(20), async {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut chunk = [0; 4096];
                let n = socket.read(&mut chunk).await.unwrap();
                assert!(n > 0);
                request.extend_from_slice(&chunk[..n]);
                if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]).to_lowercase();
                    let length: usize = headers
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if request.len() >= end + 4 + length {
                        return (socket, serde_json::from_slice(&request[end + 4..]).unwrap());
                    }
                }
            }
        })
        .await
        .unwrap()
    }
    fn success() -> String {
        http(200, &json!({"model":decisions::MODEL,"answers":[{"type":"choice","name":"primary_book_role","choice":"adventure","confidence":0.9,"probabilities":[{"value":"core_rulebook","probability":0.0},{"value":"supplement","probability":0.0},{"value":"adventure","probability":1.0},{"value":"none","probability":0.0}]}],"usage":{"input_tokens":450,"input_tokens_details":{"cached_tokens":0,"cache_write_tokens":0},"output_tokens":0,"total_tokens":450}}).to_string(), "")
    }
    async fn no_extra_request(listener: &TcpListener) {
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept())
                .await
                .is_err()
        );
    }
    async fn processed(c: &Controller, count: i64) {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let mut updates = c.analysis.subscribe();
            loop {
                if updates.borrow().processed >= count {
                    break;
                }
                updates.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
    }
    async fn retry_waiting(c: &Controller, code: Option<u16>) -> AnalysisStatus {
        tokio::time::timeout(Duration::from_secs(5), async {
            let mut updates = c.analysis.subscribe();
            loop {
                let s = updates.borrow().clone();
                if s.classification.as_ref().is_some_and(|p| {
                    p.retry_status == code && p.retry_reason.is_some() && p.retrying > 0
                }) {
                    return s;
                }
                updates.changed().await.unwrap();
            }
        })
        .await
        .unwrap()
    }
    #[tokio::test]
    async fn rate_and_capacity_responses_retry_the_same_book_until_success_without_double_counting()
    {
        let (_temp, c, listener) = parallel_fixture(1).await;
        c.start_classification(false).await.unwrap();
        let mut evidence = None;
        let mut last_attempt: Option<(tokio::time::Instant, Duration)> = None;
        for code in [429, 503] {
            let (mut socket, body) = receive(&listener).await;
            if let Some((started, minimum)) = last_attempt {
                assert!(started.elapsed() >= minimum);
            }
            if let Some(ref expected) = evidence {
                assert_eq!(&body, expected);
            } else {
                evidence = Some(body);
            }
            let started = tokio::time::Instant::now();
            socket
                .write_all(http(code, "PRIVATE_PROVIDER_ERROR", "Retry-After: 0\r\n").as_bytes())
                .await
                .unwrap();
            let waiting = retry_waiting(&c, Some(code)).await;
            assert_eq!(
                (waiting.state.as_str(), waiting.processed, waiting.skipped),
                ("running", 0, 0)
            );
            let p = waiting.classification.unwrap();
            let base = if code == 429 { 2 } else { 4 };
            last_attempt = Some((started, Duration::from_secs(base)));
            assert!(p.retry_in_secs >= base && p.retry_in_secs <= base + 1);
            assert_eq!(p.requests, if code == 429 { 1 } else { 2 });
            assert_eq!(p.retries, if code == 429 { 0 } else { 1 });
            assert!(!waiting.detail.contains("PRIVATE_PROVIDER_ERROR"));
            no_extra_request(&listener).await;
        }
        let (mut socket, body) = receive(&listener).await;
        let (started, minimum) = last_attempt.unwrap();
        assert!(started.elapsed() >= minimum);
        assert_eq!(Some(body), evidence);
        socket.write_all(success().as_bytes()).await.unwrap();
        let s = finished(&c).await;
        assert_eq!(
            (s.state.as_str(), s.processed, s.scored, s.skipped),
            ("completed", 1, 1, 0)
        );
        let p = s.classification.unwrap();
        assert_eq!(
            (
                p.requests,
                p.retries,
                p.input_tokens,
                p.retrying,
                p.retry_in_secs
            ),
            (3, 2, 450, 0, 0)
        );
        assert_eq!(p.retry_status, None);
        assert_eq!(p.retry_reason, None);
        c.start_classification(false).await.unwrap();
        assert_eq!(finished(&c).await.classification.unwrap().requests, 0);
        no_extra_request(&listener).await;
        c.shutdown().await;
    }
    #[tokio::test]
    async fn shared_server_cooldowns_block_new_work_and_cancel_or_close_without_waiting() {
        for (code, close) in [(429, false), (503, true), (500, false), (401, true)] {
            let (_temp, c, listener) = parallel_fixture(4).await;
            c.start_classification(false).await.unwrap();
            let (mut first, _) = receive(&listener).await;
            let (mut second, _) = receive(&listener).await;
            let (mut third, _) = receive(&listener).await;
            first
                .write_all(http(code, "PRIVATE_PROVIDER_ERROR", "Retry-After: 7200\r\n").as_bytes())
                .await
                .unwrap();
            let waiting = retry_waiting(&c, Some(code)).await;
            assert!(waiting.classification.unwrap().retry_in_secs >= 7200);
            second.write_all(success().as_bytes()).await.unwrap();
            third.write_all(success().as_bytes()).await.unwrap();
            processed(&c, 2).await;
            no_extra_request(&listener).await;
            let p = c.snapshot().await.analysis.classification.unwrap();
            assert_eq!((p.requests, p.retries, p.retrying), (3, 0, 1));
            assert!(p.retry_in_secs >= 7199);
            if close {
                tokio::time::timeout(Duration::from_secs(5), c.shutdown())
                    .await
                    .unwrap();
            } else {
                c.cancel_analysis().await;
                assert_eq!(finished(&c).await.state, "cancelled");
                c.shutdown().await;
            }
            let s = c.snapshot().await.analysis;
            assert_eq!((s.state.as_str(), s.scored), ("cancelled", 2));
            let p = s.classification.unwrap();
            assert_eq!((p.retrying, p.retry_in_secs, p.in_flight), (0, 0, 0));
            let next = Catalogue::connect(&c.paths).await.unwrap();
            assert_eq!(next.begin_classification(false).await.unwrap().total, 2);
            let waiting:i64=sqlx::query_scalar("SELECT count(*) FROM classification_attempts WHERE state IN ('in_flight','retry_wait')").fetch_one(&next.pool).await.unwrap();
            assert_eq!(waiting, 0);
            next.release().await.unwrap();
        }
    }
    #[tokio::test]
    async fn parallel_requests_refill_on_out_of_order_completion_and_preserve_exact_totals() {
        let (_temp, c, listener) = parallel_fixture(5).await;
        c.start_classification(false).await.unwrap();
        let (mut first, a) = receive(&listener).await;
        let (mut second, b) = receive(&listener).await;
        let (mut third, d) = receive(&listener).await;
        no_extra_request(&listener).await;
        assert_eq!(
            c.snapshot()
                .await
                .analysis
                .classification
                .unwrap()
                .in_flight,
            3
        );
        third.write_all(success().as_bytes()).await.unwrap();
        let (mut fourth, e) = receive(&listener).await;
        assert_eq!(c.snapshot().await.analysis.processed, 1);
        first.write_all(success().as_bytes()).await.unwrap();
        second.write_all(success().as_bytes()).await.unwrap();
        fourth.write_all(success().as_bytes()).await.unwrap();
        let (mut fifth, f) = receive(&listener).await;
        fifth.write_all(success().as_bytes()).await.unwrap();
        let s = finished(&c).await;
        assert_eq!(
            (s.state.as_str(), s.processed, s.scored),
            ("completed", 5, 5)
        );
        let p = s.classification.unwrap();
        assert_eq!(
            (p.workers, p.in_flight, p.requests, p.input_tokens),
            (3, 0, 5, 2250)
        );
        let evidence: std::collections::HashSet<_> = [a, b, d, e, f]
            .into_iter()
            .map(|v| v["input"].to_string())
            .collect();
        assert_eq!(evidence.len(), 5);
        c.start_classification(false).await.unwrap();
        assert_eq!(finished(&c).await.classification.unwrap().requests, 0);
        no_extra_request(&listener).await;
        // Forced reruns still admit each book exactly once despite saved results.
        c.start_classification(true).await.unwrap();
        let mut evidence = std::collections::HashSet::new();
        for _ in 0..5 {
            let (mut socket, body) = receive(&listener).await;
            assert!(evidence.insert(body["input"].to_string()));
            socket.write_all(success().as_bytes()).await.unwrap();
        }
        assert_eq!(finished(&c).await.classification.unwrap().requests, 5);
        c.shutdown().await;
    }
    #[tokio::test]
    async fn parallel_close_keeps_completed_results_and_cancels_every_other_worker() {
        let (_temp, c, listener) = parallel_fixture(5).await;
        c.start_classification(false).await.unwrap();
        let (_first, _) = receive(&listener).await;
        let (_second, _) = receive(&listener).await;
        let (mut third, _) = receive(&listener).await;
        third.write_all(success().as_bytes()).await.unwrap();
        let (_fourth, _) = receive(&listener).await;
        processed(&c, 1).await;
        tokio::time::timeout(std::time::Duration::from_secs(5), c.shutdown())
            .await
            .unwrap();
        let s = c.snapshot().await.analysis;
        assert_eq!((s.state.as_str(), s.processed), ("cancelled", 1));
        assert_eq!(s.classification.unwrap().in_flight, 0);
        no_extra_request(&listener).await;
        let next = Catalogue::connect(&c.paths).await.unwrap();
        let remaining = next.begin_classification(false).await.unwrap();
        assert_eq!(remaining.total, 4);
        let interrupted: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM classification_attempts WHERE state='cancelled'",
        )
        .fetch_one(&next.pool)
        .await
        .unwrap();
        assert_eq!(interrupted, 3);
        next.release().await.unwrap();
    }
    #[tokio::test]
    async fn parallel_storage_error_stops_admission_and_preserves_saved_results() {
        let (_temp, c, listener) = parallel_fixture(10).await;
        c.start_classification(false).await.unwrap();
        let (mut first, _) = receive(&listener).await;
        let (mut second, _) = receive(&listener).await;
        let (_third, _) = receive(&listener).await;
        first.write_all(success().as_bytes()).await.unwrap();
        let (_fourth, _) = receive(&listener).await;
        let db = c.tx.borrow().catalogue.clone().unwrap();
        sqlx::raw_sql("CREATE TRIGGER fail_classification_write BEFORE INSERT ON book_classifications BEGIN SELECT RAISE(ABORT,'synthetic storage failure'); END")
            .execute(&db.pool).await.unwrap();
        second.write_all(success().as_bytes()).await.unwrap();
        let s = finished(&c).await;
        assert_eq!((s.state.as_str(), s.scored), ("failed", 1));
        let p = s.classification.unwrap();
        assert_eq!((p.requests, p.in_flight, p.input_tokens), (4, 0, 450));
        assert_eq!(p.retries, 0);
        no_extra_request(&listener).await;
        sqlx::raw_sql("DROP TRIGGER fail_classification_write")
            .execute(&db.pool)
            .await
            .unwrap();
        c.shutdown().await;
    }
    async fn fixture(client: decisions::Client, n: i64) -> (tempfile::TempDir, Arc<Controller>) {
        let temp = tempfile::tempdir().unwrap();
        let paths = LocalPaths::at(temp.path());
        let db = Catalogue::connect(&paths).await.unwrap();
        let intent = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO intents(id,target,product_id,payload,generation,finalized) VALUES(?,'drivethru','1','{}',?,1)").bind(intent).bind(db.generation).execute(&db.pool).await.unwrap();
        sqlx::query("WITH RECURSIVE n(i) AS(VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<?) INSERT INTO products(target,product_id,title,description_text,record,parser_version,record_schema_version,archive,fetched_at,last_checked_at,intent) SELECT 'drivethru',CAST(i AS TEXT),'Synthetic adventure '||i,'A full ready-to-play scenario for four heroes','{\"raw_metadata\":\"DO_NOT_SEND\"}','drivethru-rust-1',2,'{}','2026-01-01','2026-01-01',? FROM n").bind(n).bind(intent).execute(&db.pool).await.unwrap();
        db.release().await.unwrap();
        db.pool.close().await;
        let c = Controller::new(paths, false);
        c.install_test_decisions(client);
        c.initialize().await.unwrap();
        (temp, c)
    }
    async fn finished(c: &Controller) -> AnalysisStatus {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let mut updates = c.analysis.subscribe();
            loop {
                let s = updates.borrow().clone();
                if !s.active() {
                    return s;
                }
                updates.changed().await.unwrap();
            }
        })
        .await
        .unwrap()
    }
    #[tokio::test]
    async fn classification_is_explicit_uses_saved_evidence_and_reruns_skip_paid_work() {
        let body = json!({"model":decisions::MODEL,"answers":[{"type":"choice","name":"primary_book_role","choice":"adventure","confidence":0.9,"probabilities":[{"value":"core_rulebook","probability":0.0},{"value":"supplement","probability":0.0},{"value":"adventure","probability":1.0},{"value":"none","probability":0.0}]}],"usage":{"input_tokens":450,"input_tokens_details":{"cached_tokens":0,"cache_write_tokens":0},"output_tokens":0,"total_tokens":450}});
        let (client, received, task) = server(http(200, &body.to_string(), "")).await;
        let (_temp, c) = fixture(client, 1).await;
        assert_eq!(c.snapshot().await.analysis.state, "idle");
        c.start_classification(false).await.unwrap();
        assert!(c.start_analysis().await.is_err());
        assert!(c.start(false).await.is_err());
        let request = received.await.unwrap();
        assert!(request.contains("A full ready-to-play scenario"));
        assert!(!request.contains("DO_NOT_SEND"));
        assert!(request.contains("\"primary_book_role\""));
        let s = finished(&c).await;
        assert_eq!(s.state, "completed");
        assert_eq!(s.scored, 1);
        assert_eq!(s.classification.unwrap().input_tokens, 450);
        assert_eq!(
            c.analysis_overview().await.unwrap().analyses[crate::classification::ALGORITHM].counts
                ["adventure"],
            1
        );
        c.start_classification(false).await.unwrap();
        let s = finished(&c).await;
        assert_eq!(s.total, 0);
        assert_eq!(s.classification.unwrap().requests, 0);
        task.await.unwrap();
        c.shutdown().await;
    }
    #[tokio::test]
    async fn api_and_connection_failures_retry_unchanged_evidence_and_recover_without_rerun() {
        use decisions::Error as ApiError;
        let mut failures: Vec<_> = [302, 400, 401, 403, 408, 409, 422, 500, 502, 504]
            .into_iter()
            .map(|status| {
                (
                    Some(http(status, "PRIVATE_PROVIDER_ERROR", "")),
                    ApiError::Http {
                        status,
                        retry_after_secs: None,
                    },
                )
            })
            .collect();
        failures.extend([
            (None, ApiError::Transport), // Wi-Fi drop before response headers.
            (
                Some(
                    "HTTP/1.1 200 OK\r\nContent-Length: 200\r\n\r\n{\"PRIVATE_PROVIDER_ERROR\":"
                        .into(),
                ),
                ApiError::Transport,
            ), // Interrupted body.
            (Some(String::new()), ApiError::Timeout),
            (
                Some(http(200, "PRIVATE_PROVIDER_ERROR", "")),
                ApiError::InvalidResponse,
            ),
            (
                Some("HTTP/1.1 200 OK\r\nContent-Length: 1048577\r\n\r\n".into()),
                ApiError::ResponseTooLarge,
            ),
        ]);
        for (response, expected) in failures {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let mut client = decisions::Client::local(format!(
                "http://{}/v1/decisions",
                listener.local_addr().unwrap()
            ));
            if matches!(expected, ApiError::Timeout) {
                client = client.with_test_deadline(Duration::from_secs(1));
            }
            let (_temp, c) = fixture(client, 1).await;
            c.start_classification(false).await.unwrap();
            let (mut socket, evidence) = receive(&listener).await;
            let started = tokio::time::Instant::now();
            if let Some(ref reply) = response {
                socket.write_all(reply.as_bytes()).await.unwrap();
            }
            // An empty response holds the connection open until the attempt times out.
            let stalled = if response.as_deref() == Some("") {
                Some(socket)
            } else {
                drop(socket);
                None
            };
            let waiting = retry_waiting(&c, expected.retry_hint().unwrap().status).await;
            assert_eq!(
                (waiting.state.as_str(), waiting.processed, waiting.skipped),
                ("running", 0, 0)
            );
            let p = waiting.classification.unwrap();
            assert_eq!((p.requests, p.retries, p.retrying), (1, 0, 1));
            assert!(p.retry_in_secs >= 2 && p.retry_in_secs <= 3);
            assert!(
                !serde_json::to_string(&p)
                    .unwrap()
                    .contains("PRIVATE_PROVIDER_ERROR")
            );
            let (mut socket, retried) = receive(&listener).await;
            assert!(started.elapsed() >= Duration::from_secs(2));
            assert_eq!(retried, evidence);
            socket.write_all(success().as_bytes()).await.unwrap();
            let done = finished(&c).await;
            assert_eq!(
                (
                    done.state.as_str(),
                    done.processed,
                    done.scored,
                    done.skipped
                ),
                ("completed", 1, 1, 0)
            );
            let p = done.classification.unwrap();
            assert_eq!(
                (p.requests, p.retries, p.input_tokens, p.retrying),
                (2, 1, 450, 0)
            );
            assert_eq!(p.retry_reason, None);
            drop(stalled);
            c.shutdown().await;
        }
    }
    #[tokio::test]
    async fn close_cancels_inflight_classification_and_releases_ownership() {
        let (client, received, task) = server(String::new()).await;
        let (_temp, c) = fixture(client, 3).await;
        c.start_classification(false).await.unwrap();
        received.await.unwrap();
        assert!(c.start_classification(false).await.is_err());
        assert_eq!(c.analysis_overview().await.unwrap().catalogue_total, 3);
        tokio::time::timeout(std::time::Duration::from_secs(5), c.shutdown())
            .await
            .unwrap();
        assert_eq!(c.snapshot().await.analysis.state, "cancelled");
        let next = Catalogue::connect(&c.paths).await.unwrap();
        let status = next.begin_classification(false).await.unwrap();
        assert_eq!(status.total, 3);
        next.release().await.unwrap();
        task.abort();
    }
    #[tokio::test]
    async fn player_analysis_selects_adventures_retries_responses_and_keeps_null_results() {
        let (_temp, c, listener) = parallel_fixture(4).await;
        let db = Catalogue::connect(&c.paths).await.unwrap();
        let mut roles = db.begin_classification(false).await.unwrap();
        sqlx::query("INSERT INTO book_classifications(target,product_id,system,category,algorithm,model,confidence,probabilities,input_hash,calculated_at,run_id) SELECT target,product_id,'book_role',CASE WHEN product_id='4' THEN 'supplement' ELSE 'adventure' END,'primary-book-role-1','gpt-6-luna',1,'[]','synthetic','2026-01-01',? FROM products").bind(roles.id).execute(&db.pool).await.unwrap();
        db.finish_analysis(&mut roles, "completed", "")
            .await
            .unwrap();
        db.release().await.unwrap();
        c.start_player_analysis(false).await.unwrap();
        assert!(c.start_classification(false).await.is_err());
        assert!(c.start_analysis().await.is_err());
        let (mut first, evidence) = receive(&listener).await;
        let (mut second, _) = receive(&listener).await;
        let (mut third, _) = receive(&listener).await;
        assert_eq!(evidence["model"], crate::players::MODEL);
        assert_eq!(evidence["input"][0]["role"], "developer");
        assert!(!evidence.to_string().contains("DO_NOT_SEND"));
        first
            .write_all(http(502, "PRIVATE_PROVIDER_ERROR", "").as_bytes())
            .await
            .unwrap();
        retry_waiting(&c, Some(502)).await;
        let unknown =
            crate::players::tests::response(json!({"min_players":null,"max_players":null}));
        second
            .write_all(http(200, &unknown.to_string(), "").as_bytes())
            .await
            .unwrap();
        let known = crate::players::tests::response(json!({"min_players":3,"max_players":5}));
        third
            .write_all(http(200, &known.to_string(), "").as_bytes())
            .await
            .unwrap();
        processed(&c, 2).await;
        no_extra_request(&listener).await;
        let (mut retried, body) = receive(&listener).await;
        assert_eq!(body, evidence);
        retried
            .write_all(http(200, &known.to_string(), "").as_bytes())
            .await
            .unwrap();
        let s = finished(&c).await;
        assert_eq!(
            (s.state.as_str(), s.algorithm.as_str(), s.scored),
            ("completed", crate::players::ALGORITHM, 3)
        );
        let p = s.classification.unwrap();
        assert_eq!(
            (
                p.requests,
                p.retries,
                p.known_players,
                p.unknown_players,
                p.output_tokens
            ),
            (4, 1, 2, 1, 60)
        );
        c.start_player_analysis(false).await.unwrap();
        assert_eq!(finished(&c).await.classification.unwrap().requests, 0);
        no_extra_request(&listener).await;
        c.start_player_analysis(true).await.unwrap();
        let (_a, _) = receive(&listener).await;
        let (_b, _) = receive(&listener).await;
        let (_d, _) = receive(&listener).await;
        tokio::time::timeout(Duration::from_secs(5), c.shutdown())
            .await
            .unwrap();
        assert_eq!(c.snapshot().await.analysis.state, "cancelled");
        let next = Catalogue::connect(&c.paths).await.unwrap();
        assert_eq!(next.begin_players(false).await.unwrap().total, 0);
        next.release().await.unwrap();
    }
    #[tokio::test]
    async fn dnd_setting_and_context_routes_retry_reuse_unknowns_and_cancel() {
        use crate::dnd_context::{Data, Kind as Context};
        let (_temp, c, listener) = parallel_fixture(2).await;
        let db = Catalogue::connect(&c.paths).await.unwrap();
        let mut prerequisite = db.begin_dnd(false, false).await.unwrap();
        sqlx::query("INSERT INTO book_classifications(target,product_id,system,category,algorithm,model,confidence,probabilities,input_hash,calculated_at,run_id) SELECT target,product_id,'dnd',CASE WHEN product_id='2' THEN 'no' ELSE 'dnd_only' END,'dnd-compatibility-1','gpt-6-luna',1,'[]','synthetic','2026-01-01',? FROM products").bind(prerequisite.id).execute(&db.pool).await.unwrap();
        db.finish_analysis(&mut prerequisite, "completed", "")
            .await
            .unwrap();
        complete_discovery(&db).await;
        db.release().await.unwrap();
        c.start_dnd_settings(false).await.unwrap();
        let (mut socket, body) = receive(&listener).await;
        assert_eq!(body["questions"].as_array().unwrap().len(), 15);
        assert!(!body.to_string().contains("prompt_cache_breakpoint"));
        assert!(!body.to_string().contains("DO_NOT_SEND"));
        let answers: Vec<_> = crate::dnd_settings::SETTINGS.iter().map(|s|s.0).chain(["any"]).map(|name| {
            let choice = if name == "al_qadim" { "applies" } else { "uncertain" };
            let probabilities = ["applies", "not_applicable", "uncertain"].map(|value| json!({"value":value,"probability":if value == choice {1} else {0}}));
            json!({"type":"choice","name":name,"choice":choice,"confidence":1,"probabilities":probabilities})
        }).collect();
        let value = json!({"model":decisions::MODEL,"answers":answers,"usage":{"input_tokens":600,"output_tokens":0,"total_tokens":600,"input_tokens_details":{"cached_tokens":0,"cache_write_tokens":0}}});
        socket
            .write_all(http(200, &value.to_string(), "").as_bytes())
            .await
            .unwrap();
        let done = finished(&c).await;
        assert_eq!(done.scored, 1);
        let totals = &done.overview.unwrap().analyses[crate::dnd_settings::ALGORITHM];
        assert_eq!(
            (
                totals.eligible,
                totals.counts["al_qadim"],
                totals.counts["forgotten_realms"]
            ),
            (1, 1, 1)
        );
        c.start_dnd_settings(false).await.unwrap();
        assert_eq!(finished(&c).await.classification.unwrap().requests, 0);
        for kind in Context::ALL {
            c.start_dnd_extraction(kind, false).await.unwrap();
            let (mut socket, body) = receive(&listener).await;
            assert_eq!(body["model"], "gpt-6-luna");
            assert_eq!(body["text"]["format"]["name"], kind.key());
            assert_eq!(body["text"]["format"]["strict"], true);
            assert!(
                body["input"][0]
                    .to_string()
                    .contains("prompt_cache_breakpoint")
            );
            assert!(
                !body["input"][1]
                    .to_string()
                    .contains("prompt_cache_breakpoint")
            );
            assert!(!body.to_string().contains("DO_NOT_SEND"));
            let (data, known) = match kind {
                Context::Levels => (json!({"min_level":null,"max_level":null}), false),
                Context::Locations => (json!({"locations":["Stonebridge"]}), true),
                Context::Date => (json!({"in_universe_date":"circa 1492 DR"}), true),
            };
            if kind == Context::Levels {
                // A syntactically valid provider response with invalid field types retries too.
                let invalid =
                    crate::players::tests::response(json!({"min_level":"bad","max_level":null}));
                socket
                    .write_all(http(200, &invalid.to_string(), "").as_bytes())
                    .await
                    .unwrap();
                retry_waiting(&c, None).await;
                let (retried, again) = receive(&listener).await;
                assert_eq!(again["model"], "gpt-6.1-terra");
                assert_eq!(again["input"], body["input"]);
                socket = retried;
            }
            let mut value = crate::players::tests::response(data);
            if kind == Context::Levels {
                value["model"] = json!("gpt-6.1-terra");
            }
            socket
                .write_all(http(200, &value.to_string(), "").as_bytes())
                .await
                .unwrap();
            let done = finished(&c).await;
            assert_eq!((done.total, done.processed, done.scored), (1, 1, 1));
            assert_eq!(
                done.classification.as_ref().unwrap().context_known,
                i64::from(known)
            );
            c.start_dnd_extraction(kind, false).await.unwrap();
            assert_eq!(finished(&c).await.classification.unwrap().requests, 0);
        }
        no_extra_request(&listener).await;
        c.start_dnd_extraction(Context::Date, true).await.unwrap();
        let (_pending, _) = receive(&listener).await;
        tokio::time::timeout(Duration::from_secs(5), c.shutdown())
            .await
            .unwrap();
        assert_eq!(c.snapshot().await.analysis.state, "cancelled");
        let next = Catalogue::connect(&c.paths).await.unwrap();
        let saved = crate::storage::dnd_context::saved(&next.pool, "drivethru", "1", Context::Date)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            saved.data,
            Data::Date {
                in_universe_date: Some("circa 1492 DR".into())
            }
        );
        assert_eq!(
            next.begin_context(Context::Date, false)
                .await
                .unwrap()
                .total,
            0
        );
        next.release().await.unwrap();
    }
    #[tokio::test]
    async fn earth_edition_routes_use_independent_decisions_retry_reuse_and_drain() {
        use crate::earth_editions::Kind as Earth;
        let (_temp, c, listener) = parallel_fixture(3).await;
        let db = Catalogue::connect(&c.paths).await.unwrap();
        sqlx::query("UPDATE products SET record=json_set(record,'$.rule_systems',CASE product_id WHEN '1' THEN json_array('Shadowrun') WHEN '2' THEN json_array('Call of Cthulhu') ELSE json_array('Cthulhu Eternal') END)").execute(&db.pool).await.unwrap();
        complete_discovery(&db).await;
        db.release().await.unwrap();
        for kind in Earth::ALL {
            c.start_earth_editions(kind, false).await.unwrap();
            let (mut socket, body) = receive(&listener).await;
            assert_eq!(body["model"], decisions::MODEL);
            assert_eq!(body["prompt_cache_options"]["mode"], "explicit");
            assert!(!body.to_string().contains("prompt_cache_breakpoint"));
            assert!(!body.to_string().contains("DO_NOT_SEND"));
            assert_eq!(
                body["questions"].as_array().unwrap().len(),
                kind.editions().len()
            );
            socket
                .write_all(http(503, "PRIVATE_PROVIDER_ERROR", "").as_bytes())
                .await
                .unwrap();
            retry_waiting(&c, Some(503)).await;
            let (mut socket, again) = receive(&listener).await;
            assert_eq!(body, again);
            let answer = json!({"model":decisions::MODEL,"answers":kind.editions().iter().map(|(key,_)|json!({"type":"choice","name":key,"choice":"uncertain","confidence":1.0,"probabilities":[{"value":"supported","probability":0.0},{"value":"not_supported","probability":0.0},{"value":"uncertain","probability":1.0}]})).collect::<Vec<_>>(),"usage":{"input_tokens":600,"output_tokens":0,"total_tokens":600,"input_tokens_details":{"cached_tokens":0,"cache_write_tokens":0}}});
            socket
                .write_all(http(200, &answer.to_string(), "").as_bytes())
                .await
                .unwrap();
            let done = finished(&c).await;
            assert_eq!((done.total, done.scored), (1, 1));
            assert_eq!(done.classification.as_ref().unwrap().retries, 1);
            assert_eq!(
                done.overview.as_ref().unwrap().analyses[kind.algorithm()].counts["unknown"],
                1
            );
            c.start_earth_editions(kind, false).await.unwrap();
            assert_eq!(finished(&c).await.classification.unwrap().requests, 0);
        }
        no_extra_request(&listener).await;
        c.start_earth_editions(Earth::Shadowrun, true)
            .await
            .unwrap();
        let (_pending, _) = receive(&listener).await;
        tokio::time::timeout(Duration::from_secs(5), c.shutdown())
            .await
            .unwrap();
        assert_eq!(c.snapshot().await.analysis.state, "cancelled");
        let next = Catalogue::connect(&c.paths).await.unwrap();
        assert!(
            crate::storage::classification::saved_earth_editions(
                &next.pool,
                "drivethru",
                "1",
                Earth::Shadowrun
            )
            .await
            .unwrap()
            .is_some()
        );
        next.release().await.unwrap();
    }
    #[tokio::test]
    async fn structured_escalation_preserves_model_on_http_retry_usage_and_cancellation() {
        use crate::{earth_context::Kind as Earth, responses::Model};
        let (_temp, c, listener) = parallel_fixture(1).await;
        let db = Catalogue::connect(&c.paths).await.unwrap();
        sqlx::query(
            "UPDATE products SET record=json_set(record,'$.rule_systems',json_array('Shadowrun'))",
        )
        .execute(&db.pool)
        .await
        .unwrap();
        complete_discovery(&db).await;
        db.release().await.unwrap();
        c.start_earth_extraction(Earth::ShadowrunDate, false)
            .await
            .unwrap();
        let mut original = None;
        for model in Model::ALL {
            let (mut socket, body) = receive(&listener).await;
            assert_eq!(body["model"], model.name());
            let source = original.get_or_insert_with(|| body["input"].clone());
            assert_eq!(&body["input"], source);
            if model == Model::Sol {
                socket
                    .write_all(http(503, "PRIVATE_PROVIDER_ERROR", "").as_bytes())
                    .await
                    .unwrap();
                let (next, again) = receive(&listener).await;
                assert_eq!(again, body);
                socket = next;
            }
            let mut value = crate::players::tests::response(
                json!({"date_start":"2055-01-01","date_end":"2059-12-31","precision":if model==Model::Astra{"range"}else{"decade"},"basis":"explicit"}),
            );
            value["model"] = json!(format!("{}-2026-10-01", model.name()));
            socket
                .write_all(http(200, &value.to_string(), "").as_bytes())
                .await
                .unwrap();
        }
        let done = finished(&c).await;
        let p = done.classification.as_ref().unwrap();
        assert_eq!(
            (
                done.scored,
                p.requests,
                p.retries,
                p.model_escalations,
                p.exhausted_responses,
                p.input_tokens,
                p.output_tokens
            ),
            (1, 5, 4, 3, 0, 6400, 80)
        );
        let db = Catalogue::connect(&c.paths).await.unwrap();
        assert_eq!(
            crate::storage::earth_context::saved(&db.pool, "drivethru", "1", Earth::ShadowrunDate)
                .await
                .unwrap()
                .unwrap()
                .model,
            "gpt-6-astra-2026-10-01"
        );
        db.release().await.unwrap();
        c.start_earth_extraction(Earth::ShadowrunDate, true)
            .await
            .unwrap();
        let (mut socket, body) = receive(&listener).await;
        assert_eq!(body["model"], Model::Luna.name());
        socket
            .write_all(http(200, "{malformed", "").as_bytes())
            .await
            .unwrap();
        let (_pending, body) = receive(&listener).await;
        assert_eq!(body["model"], Model::Terra.name());
        tokio::time::timeout(Duration::from_secs(5), c.shutdown())
            .await
            .unwrap();
        let db = Catalogue::connect(&c.paths).await.unwrap();
        assert_eq!(
            db.begin_earth(Earth::ShadowrunDate, false)
                .await
                .unwrap()
                .total,
            0
        );
        db.release().await.unwrap();
    }
    #[tokio::test]
    async fn astra_exhaustion_continues_other_books_preserves_prior_result_and_retries_missing_next_run()
     {
        use crate::{earth_context::Kind as Earth, responses::Model};
        let (_temp, c, listener) = parallel_fixture(3).await;
        let db = Catalogue::connect(&c.paths).await.unwrap();
        sqlx::query(
            "UPDATE products SET record=json_set(record,'$.rule_systems',json_array('Shadowrun'))",
        )
        .execute(&db.pool)
        .await
        .unwrap();
        let mut prior = db.begin_earth(Earth::ShadowrunDate, false).await.unwrap();
        let book = db.earth_next_after(&prior, 0).await.unwrap().unwrap();
        let previous = Earth::ShadowrunDate
            .decode(crate::players::tests::response(
                json!({"date_start":null,"date_end":null,"precision":"unknown","basis":"unknown"}),
            ))
            .unwrap();
        db.earth_commit(&mut prior, &book, Some(&previous), None)
            .await
            .unwrap();
        db.finish_analysis(&mut prior, "completed", "")
            .await
            .unwrap();
        complete_discovery(&db).await;
        db.release().await.unwrap();
        c.start_earth_extraction(Earth::ShadowrunDate, true)
            .await
            .unwrap();
        // Two persistent failures and a successful sibling, all starting at Luna.
        for _ in 0..9 {
            let (mut socket, body) = receive(&listener).await;
            let source: serde_json::Value =
                serde_json::from_str(body["input"][1]["content"][0]["text"].as_str().unwrap())
                    .unwrap();
            let title = source["title"].as_str().unwrap();
            let sibling = title.ends_with('3');
            let mut value = crate::players::tests::response(if sibling {
                json!({"date_start":null,"date_end":null,"precision":"unknown","basis":"unknown"})
            } else {
                json!({"date_start":"2055-01-01","date_end":"2059-12-31","precision":"decade","basis":"explicit"})
            });
            value["model"] = body["model"].clone();
            socket
                .write_all(http(200, &value.to_string(), "").as_bytes())
                .await
                .unwrap();
        }
        let done = finished(&c).await;
        let p = done.classification.unwrap();
        assert_eq!(
            (
                done.state.as_str(),
                done.processed,
                done.scored,
                done.skipped
            ),
            ("completed", 3, 1, 2)
        );
        assert_eq!(
            (
                p.requests,
                p.retries,
                p.model_escalations,
                p.exhausted_responses,
                p.input_tokens
            ),
            (9, 6, 6, 2, 14400)
        );
        no_extra_request(&listener).await;
        let db = Catalogue::connect(&c.paths).await.unwrap();
        assert_eq!(
            crate::storage::earth_context::saved(&db.pool, "drivethru", "1", Earth::ShadowrunDate)
                .await
                .unwrap()
                .unwrap()
                .model,
            Model::Luna.name()
        );
        let mut next = db.begin_earth(Earth::ShadowrunDate, false).await.unwrap();
        assert_eq!(next.total, 1);
        assert_eq!(
            db.earth_next_after(&next, 0)
                .await
                .unwrap()
                .unwrap()
                .product_id,
            "2"
        );
        db.finish_analysis(&mut next, "cancelled", "")
            .await
            .unwrap();
        db.release().await.unwrap();
    }
    #[tokio::test]
    async fn earth_analysis_routes_use_rule_metadata_retry_invalid_dates_and_drain() {
        use crate::earth_context::Kind as Earth;
        let (_temp, c, listener) = parallel_fixture(3).await;
        let db = Catalogue::connect(&c.paths).await.unwrap();
        sqlx::query("UPDATE products SET record=json_set(record,'$.rule_systems',CASE product_id WHEN '1' THEN json('[\"SHADOWRUN\"]') WHEN '2' THEN json('[\"Call of Cthulhu\"]') ELSE json('[\"Cthulhu Eternal\"]') END)").execute(&db.pool).await.unwrap();
        complete_discovery(&db).await;
        db.release().await.unwrap();
        for kind in Earth::ALL {
            c.start_earth_extraction(kind, false).await.unwrap();
            let (mut socket, body) = receive(&listener).await;
            assert_eq!(body["text"]["format"]["name"], kind.field());
            assert_eq!(body["model"], "gpt-6-luna");
            assert!(!body.to_string().contains("DO_NOT_SEND"));
            assert!(
                body["input"][0]
                    .to_string()
                    .contains("prompt_cache_breakpoint")
            );
            assert!(
                !body["input"][1]
                    .to_string()
                    .contains("prompt_cache_breakpoint")
            );
            let evidence: serde_json::Value =
                serde_json::from_str(body["input"][1]["content"][0]["text"].as_str().unwrap())
                    .unwrap();
            assert_eq!(
                evidence["rule_systems"][0],
                if kind.system() == "coc" {
                    "Call of Cthulhu"
                } else {
                    "SHADOWRUN"
                }
            );
            no_extra_request(&listener).await;
            if kind == Earth::ShadowrunDate {
                let invalid = crate::players::tests::response(
                    json!({"date_start":"2084-02-30","date_end":"2084-02-30","precision":"day","basis":"explicit"}),
                );
                socket
                    .write_all(http(200, &invalid.to_string(), "").as_bytes())
                    .await
                    .unwrap();
                retry_waiting(&c, None).await;
                let (next, again) = receive(&listener).await;
                assert_eq!(again["model"], "gpt-6.1-terra");
                assert_eq!(again["input"], body["input"]);
                socket = next;
            }
            let value = if kind.field() == "location" {
                json!({"location":null})
            } else if kind.system() == "coc" {
                json!({"date_start":null,"date_end":null,"precision":"era","basis":"modern"})
            } else {
                json!({"date_start":"2084-01-01","date_end":"2084-12-31","precision":"year","basis":"explicit"})
            };
            let mut value = crate::players::tests::response(value);
            if kind == Earth::ShadowrunDate {
                value["model"] = json!("gpt-6.1-terra");
            }
            socket
                .write_all(http(200, &value.to_string(), "").as_bytes())
                .await
                .unwrap();
            let done = finished(&c).await;
            assert_eq!((done.total, done.scored), (1, 1));
            assert_eq!(
                done.overview.as_ref().unwrap().analyses[kind.algorithm()].completed,
                1
            );
            c.start_earth_extraction(kind, false).await.unwrap();
            assert_eq!(finished(&c).await.classification.unwrap().requests, 0);
        }
        no_extra_request(&listener).await;
        c.start_earth_extraction(Earth::CocDate, true)
            .await
            .unwrap();
        let (_pending, _) = receive(&listener).await;
        tokio::time::timeout(Duration::from_secs(5), c.shutdown())
            .await
            .unwrap();
        assert_eq!(c.snapshot().await.analysis.state, "cancelled");
        let next = Catalogue::connect(&c.paths).await.unwrap();
        let mut run = next.begin_earth(Earth::CocDate, false).await.unwrap();
        assert_eq!(run.total, 0);
        next.finish_analysis(&mut run, "completed", "")
            .await
            .unwrap();
        next.release().await.unwrap();
    }
    #[tokio::test]
    async fn solo_analysis_runs_without_prior_roles_retries_and_reuses_neither() {
        let (_temp, c, listener) = parallel_fixture(3).await;
        c.start_solo_classification(false).await.unwrap();
        assert!(c.start_player_analysis(false).await.is_err());
        let (mut first, evidence) = receive(&listener).await;
        let (mut second, _) = receive(&listener).await;
        let (mut third, _) = receive(&listener).await;
        assert_eq!(evidence["model"], decisions::MODEL);
        assert_eq!(evidence["prompt_cache_options"]["mode"], "explicit");
        assert!(!evidence.to_string().contains("prompt_cache_breakpoint"));
        assert_eq!(evidence["questions"].as_array().unwrap().len(), 1);
        assert_eq!(evidence["questions"][0]["name"], "solo_play");
        assert_eq!(
            evidence["questions"][0]["choices"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert_eq!(
            evidence["questions"][0]["instructions"],
            crate::solo::INSTRUCTIONS
        );
        let input = &evidence["input"][0]["content"][0]["data"];
        assert!(input["description"].is_string());
        for key in [
            "min_players",
            "max_players",
            "book_role",
            "solo_play",
            "relevance_score",
            "raw_metadata",
        ] {
            assert!(input.get(key).is_none());
        }
        assert!(!input.to_string().contains("DO_NOT_SEND"));
        let response = |category: &str| {
            http(200, &json!({"model":decisions::MODEL,"answers":[{"type":"choice","name":"solo_play","choice":category,"confidence":1.0,"probabilities":(["solo_adventure","gamebook","neither"].map(|v|json!({"value":v,"probability":if v==category{1.0}else{0.0}})))}],"usage":{"input_tokens":400,"output_tokens":0,"total_tokens":400,"input_tokens_details":{"cached_tokens":0,"cache_write_tokens":0}}}).to_string(), "")
        };
        first
            .write_all(http(503, "PRIVATE_PROVIDER_ERROR", "").as_bytes())
            .await
            .unwrap();
        retry_waiting(&c, Some(503)).await;
        second
            .write_all(response("gamebook").as_bytes())
            .await
            .unwrap();
        third
            .write_all(response("neither").as_bytes())
            .await
            .unwrap();
        processed(&c, 2).await;
        no_extra_request(&listener).await;
        let (mut retried, body) = receive(&listener).await;
        assert_eq!(body, evidence);
        retried
            .write_all(response("solo_adventure").as_bytes())
            .await
            .unwrap();
        let s = finished(&c).await;
        assert_eq!(
            (s.state.as_str(), s.algorithm.as_str(), s.scored),
            ("completed", crate::solo::ALGORITHM, 3)
        );
        let p = s.classification.unwrap();
        assert_eq!(
            (
                p.requests,
                p.retries,
                p.solo_adventure,
                p.gamebook,
                p.neither,
                p.input_tokens
            ),
            (4, 1, 1, 1, 1, 1200)
        );
        c.start_solo_classification(false).await.unwrap();
        assert_eq!(finished(&c).await.classification.unwrap().requests, 0);
        no_extra_request(&listener).await;
        c.start_solo_classification(true).await.unwrap();
        let (_a, _) = receive(&listener).await;
        let (_b, _) = receive(&listener).await;
        let (_d, _) = receive(&listener).await;
        tokio::time::timeout(Duration::from_secs(5), c.shutdown())
            .await
            .unwrap();
        let next = Catalogue::connect(&c.paths).await.unwrap();
        assert_eq!(
            next.begin_solo_classification(false).await.unwrap().total,
            0
        );
        next.release().await.unwrap();
    }
    #[tokio::test]
    async fn dnd_requests_and_conditional_edition_checks_retry_reuse_and_drain() {
        let (_temp, c, listener) = parallel_fixture(3).await;
        let choice = |name: &str, value: &str, values: &[&str]| json!({"type":"choice","name":name,"choice":value,"confidence":1.0,"probabilities":values.iter().map(|v|json!({"value":v,"probability":if *v==value{1.0}else{0.0}})).collect::<Vec<_>>()});
        let response = |answers: Vec<serde_json::Value>| {
            http(200,&json!({"model":decisions::MODEL,"answers":answers,"usage":{"input_tokens":600,"output_tokens":0,"total_tokens":600,"input_tokens_details":{"cached_tokens":0,"cache_write_tokens":0}}}).to_string(),"")
        };
        let db = Catalogue::connect(&c.paths).await.unwrap();
        complete_discovery(&db).await;
        db.release().await.unwrap();
        c.start_dnd_classification(false).await.unwrap();
        let (mut first, body) = receive(&listener).await;
        let (mut second, _) = receive(&listener).await;
        let (mut third, _) = receive(&listener).await;
        assert_eq!(body["questions"][0]["name"], "dnd");
        assert_eq!(
            body["questions"][0]["instructions"],
            crate::dnd::INSTRUCTIONS
        );
        assert_eq!(body["model"], decisions::MODEL);
        assert_eq!(body["prompt_cache_options"]["mode"], "explicit");
        assert!(!body.to_string().contains("prompt_cache_breakpoint"));
        first
            .write_all(http(502, "PRIVATE_PROVIDER_ERROR", "").as_bytes())
            .await
            .unwrap();
        retry_waiting(&c, Some(502)).await;
        second
            .write_all(
                response(vec![choice(
                    "dnd",
                    "no",
                    &["dnd_only", "dnd_and_others", "no"],
                )])
                .as_bytes(),
            )
            .await
            .unwrap();
        third
            .write_all(
                response(vec![choice(
                    "dnd",
                    "dnd_and_others",
                    &["dnd_only", "dnd_and_others", "no"],
                )])
                .as_bytes(),
            )
            .await
            .unwrap();
        processed(&c, 2).await;
        let (mut retried, again) = receive(&listener).await;
        assert_eq!(body, again);
        retried
            .write_all(
                response(vec![choice(
                    "dnd",
                    "dnd_only",
                    &["dnd_only", "dnd_and_others", "no"],
                )])
                .as_bytes(),
            )
            .await
            .unwrap();
        let done = finished(&c).await;
        let p = done.classification.unwrap();
        assert_eq!(
            (
                done.scored,
                p.dnd_only,
                p.dnd_and_others,
                p.dnd_no,
                p.retries
            ),
            (3, 1, 1, 1, 1)
        );
        let overview = c.analysis_overview().await.unwrap();
        assert_eq!(
            (
                overview.analyses[crate::dnd::ALGORITHM].completed,
                overview.analyses[crate::dnd::EDITIONS_ALGORITHM].eligible
            ),
            (3, 2)
        );
        c.start_dnd_editions(false).await.unwrap();
        assert!(c.start_solo_classification(false).await.is_err());
        let (mut first, body) = receive(&listener).await;
        let (mut second, _) = receive(&listener).await;
        no_extra_request(&listener).await;
        assert_eq!(body["questions"].as_array().unwrap().len(), 9);
        let input = &body["input"][0]["content"][0]["data"];
        assert!(input["description"].is_string());
        for key in [
            "dnd",
            "book_role",
            "min_players",
            "solo_play",
            "relevance_score",
            "raw_metadata",
        ] {
            assert!(input.get(key).is_none());
        }
        let answers = |known: bool| {
            crate::dnd::EDITIONS
                .iter()
                .map(|(key, _)| {
                    choice(
                        key,
                        if known && matches!(*key, "bx" | "5e_2024") {
                            "supported"
                        } else {
                            "uncertain"
                        },
                        &["supported", "not_supported", "uncertain"],
                    )
                })
                .collect()
        };
        first
            .write_all(http(503, "PRIVATE_PROVIDER_ERROR", "").as_bytes())
            .await
            .unwrap();
        retry_waiting(&c, Some(503)).await;
        second
            .write_all(response(answers(false)).as_bytes())
            .await
            .unwrap();
        processed(&c, 1).await;
        let (mut retried, again) = receive(&listener).await;
        assert_eq!(body, again);
        retried
            .write_all(response(answers(true)).as_bytes())
            .await
            .unwrap();
        let done = finished(&c).await;
        let p = done.classification.unwrap();
        assert_eq!(
            (done.scored, p.editions_known, p.editions_unknown, p.retries),
            (2, 1, 1, 1)
        );
        let live = &done.overview.as_ref().unwrap().analyses[crate::dnd::EDITIONS_ALGORITHM];
        assert_eq!((live.completed, live.eligible), (2, 2));
        assert_eq!(
            (
                live.counts["bx"],
                live.counts["5e_2024"],
                live.counts["unknown"]
            ),
            (1, 1, 1)
        );
        let overview = c.analysis_overview().await.unwrap();
        assert_eq!(
            overview.analyses[crate::dnd::EDITIONS_ALGORITHM].completed,
            2
        );
        c.start_dnd_editions(false).await.unwrap();
        assert_eq!(finished(&c).await.total, 0);
        no_extra_request(&listener).await;
        c.start_dnd_editions(true).await.unwrap();
        let (_a, _) = receive(&listener).await;
        let (_b, _) = receive(&listener).await;
        tokio::time::timeout(Duration::from_secs(5), c.shutdown())
            .await
            .unwrap();
        let next = Catalogue::connect(&c.paths).await.unwrap();
        assert_eq!(next.begin_dnd(true, false).await.unwrap().total, 0);
        next.release().await.unwrap();
    }
    #[tokio::test]
    async fn missing_key_blocks_classification_but_does_not_block_offline_analysis() {
        let (_temp, c) = fixture(decisions::Client::new().unwrap(), 1).await;
        assert!(
            c.start_discovery_analysis(false)
                .await
                .unwrap_err()
                .contains("OPENAI_API_KEY")
        );
        assert!(
            c.start_classification(false)
                .await
                .unwrap_err()
                .contains("OPENAI_API_KEY")
        );
        assert_eq!(c.snapshot().await.analysis.state, "idle");
        c.start_analysis().await.unwrap();
        assert_eq!(finished(&c).await.state, "completed");
        c.shutdown().await;
    }
    fn discovery_solo_response() -> String {
        http(200, &json!({"model":decisions::MODEL,"answers":[{"type":"choice","name":"solo_play","choice":"neither","confidence":1.0,"probabilities":[{"value":"solo_adventure","probability":0.0},{"value":"gamebook","probability":0.0},{"value":"neither","probability":1.0}]}],"usage":{"input_tokens":400,"output_tokens":0,"total_tokens":400,"input_tokens_details":{"cached_tokens":0,"cache_write_tokens":0}}}).to_string(), "")
    }

    // Synthetic current Discovery coverage, using the normal commit paths.
    async fn complete_discovery(db: &Catalogue) {
        let mut weighted = db.begin_relevance(false).await.unwrap();
        while !db.analysis_batch(&mut weighted).await.unwrap() {}
        for request in [
            crate::classification::request(json!({})),
            crate::solo::request(json!({})),
        ] {
            let request = json!({"questions":request.questions});
            let decision =
                serde_json::from_value::<decisions::Decision>(sequence_decision(&request)).unwrap();
            let mut run = if request["questions"][0]["name"] == "primary_book_role" {
                db.begin_classification(false).await.unwrap()
            } else {
                db.begin_solo_classification(false).await.unwrap()
            };
            while let Some(book) = db.classification_next_after(&run, 0).await.unwrap() {
                db.classification_commit(&mut run, &book, Some(&decision), None)
                    .await
                    .unwrap();
            }
            db.finish_analysis(&mut run, "completed", "").await.unwrap();
        }
        db.require_discovery_complete().await.unwrap();
    }

    fn sequence_decision(request: &serde_json::Value) -> serde_json::Value {
        let answers: Vec<_> = request["questions"].as_array().unwrap().iter().map(|q| {
            let choices = q["choices"].as_array().unwrap();
            let chosen = if q["name"] == "dnd" { json!("dnd_only") } else if q["name"] == "family" {json!("supported")} else { choices.last().unwrap()["value"].clone() };
            json!({"type":"choice","name":q["name"],"choice":chosen,"confidence":1.0,"probabilities":choices.iter().map(|c|json!({"value":c["value"],"probability":if c["value"]==chosen {1.0}else{0.0}})).collect::<Vec<_>>()})
        }).collect();
        json!({"model":decisions::MODEL,"answers":answers,"usage":{"input_tokens":10,"output_tokens":0,"total_tokens":10,"input_tokens_details":{"cached_tokens":0,"cache_write_tokens":0}}})
    }

    #[tokio::test]
    async fn system_gate_blocks_all_entrypoints_and_rechecks_after_ownership() {
        use crate::analysis::AnalysisWorkspace as Workspace;
        let (_temp, c, listener) = parallel_fixture(1).await;
        for workspace in [
            Workspace::Dnd,
            Workspace::Osr,
            Workspace::Shadowrun,
            Workspace::Coc,
            Workspace::WarhammerFantasy,
            Workspace::Warhammer40k,
        ] {
            assert!(
                c.start_system_analysis(workspace, false)
                    .await
                    .unwrap_err()
                    .contains("Discovery")
            );
        }
        for kind in [
            Kind::Dnd,
            Kind::DndEditions,
            Kind::DndSettings,
            Kind::Osr,
            Kind::OsrLevels,
        ]
        .into_iter()
        .chain(crate::dnd_context::Kind::ALL.map(Kind::DndContext))
        .chain(crate::earth_context::Kind::ALL.map(Kind::EarthContext))
        .chain(crate::earth_editions::Kind::ALL.map(Kind::EarthEditions))
        .chain(crate::warhammer::Family::ALL.map(Kind::Warhammer))
        .chain(crate::warhammer_context::Kind::ALL.map(Kind::WarhammerContext))
        {
            assert!(
                c.start_online_analysis(kind, false)
                    .await
                    .unwrap_err()
                    .contains("Discovery")
            );
        }
        no_extra_request(&listener).await;
        let db = Catalogue::connect(&c.paths).await.unwrap();
        complete_discovery(&db).await;
        let mut run = db.begin_dnd(false, false).await.unwrap();
        let full = run.overview.clone().unwrap();
        db.finish_analysis(&mut run, "cancelled", "fixture")
            .await
            .unwrap();
        // Simulate a source edit after a UI has read complete coverage.
        sqlx::query("UPDATE products SET title='Changed fixture',record=json_set(record,'$.title','Changed fixture')")
            .execute(&db.pool)
            .await
            .unwrap();
        assert!(db.require_discovery_complete().await.is_err());
        let revision = sqlx::query_scalar::<_, i64>(
            "SELECT revision FROM analysis_revision WHERE singleton=1",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        // Force the same stale preflight result a source change between preflight
        // and ownership acquisition could produce. The owned check is authoritative.
        *c.coverage.lock().await = Some((db.catalogue, revision, full));
        db.release().await.unwrap();
        for sequence in [true, false] {
            if sequence {
                c.start_system_analysis(Workspace::Dnd, false)
                    .await
                    .unwrap();
            } else {
                c.start_dnd_classification(false).await.unwrap();
            }
            let done = finished(&c).await;
            assert_eq!(done.state, "failed");
            assert!(done.detail.contains("Discovery"));
        }
        no_extra_request(&listener).await;
    }

    #[tokio::test]
    async fn system_sequences_order_reuse_unknowns_and_cancel_reruns() {
        use crate::analysis::AnalysisWorkspace as Workspace;
        for workspace in [
            Workspace::Dnd,
            Workspace::Osr,
            Workspace::Shadowrun,
            Workspace::Coc,
            Workspace::WarhammerFantasy,
            Workspace::Warhammer40k,
        ] {
            let (_temp, c, listener) = parallel_fixture(1).await;
            let db = Catalogue::connect(&c.paths).await.unwrap();
            sqlx::query("UPDATE products SET record=json_set(record,'$.rule_systems',json_array('Shadowrun','Call of Cthulhu','Old-School Revival (OSR)','Warhammer'))").execute(&db.pool).await.unwrap();
            complete_discovery(&db).await;
            db.release().await.unwrap();
            let expected: Vec<&str> = match workspace {
                Workspace::Dnd => vec![
                    crate::dnd::ALGORITHM,
                    crate::dnd::EDITIONS_ALGORITHM,
                    crate::dnd_settings::ALGORITHM,
                    "dnd-levels-1",
                    "dnd-locations-1",
                    "dnd-date-1",
                ],
                Workspace::WarhammerFantasy => vec![
                    "warhammer-fantasy-games-1",
                    "warhammer-fantasy-locations-1",
                    "warhammer-fantasy-date-1",
                ],
                Workspace::Warhammer40k => vec![
                    "warhammer-40k-games-1",
                    "warhammer-40k-locations-1",
                    "warhammer-40k-date-1",
                ],
                Workspace::Osr => vec![crate::osr::ALGORITHM],
                Workspace::Shadowrun => vec![
                    "shadowrun-editions-1",
                    "shadowrun-location-1",
                    "shadowrun-date-1",
                ],
                _ => vec!["coc-editions-1", "coc-location-1", "coc-date-1"],
            };
            c.start_system_analysis(workspace, false).await.unwrap();
            for (step, algorithm) in expected.iter().enumerate() {
                let (mut socket, body) = receive(&listener).await;
                let current = c.snapshot().await.analysis;
                assert_eq!(current.algorithm, *algorithm);
                assert_eq!(current.discovery.as_ref().unwrap().workspace, workspace);
                assert_eq!(current.discovery.unwrap().step, step);
                assert!(Catalogue::connect(&c.paths).await.is_err());
                let response = if body["questions"].is_array() {
                    sequence_decision(&body)
                } else {
                    let value = match *algorithm {
                        "dnd-levels-1" => json!({"min_level":null,"max_level":null}),
                        "dnd-locations-1"
                        | "warhammer-fantasy-locations-1"
                        | "warhammer-40k-locations-1" => json!({"locations":[]}),
                        "dnd-date-1" | "warhammer-fantasy-date-1" | "warhammer-40k-date-1" => {
                            json!({"in_universe_date":null})
                        }
                        "shadowrun-location-1" | "coc-location-1" => json!({"location":null}),
                        _ => {
                            json!({"date_start":null,"date_end":null,"precision":"unknown","basis":"unknown"})
                        }
                    };
                    crate::players::tests::response(value)
                };
                socket
                    .write_all(http(200, &response.to_string(), "").as_bytes())
                    .await
                    .unwrap();
            }
            let done = finished(&c).await;
            assert_eq!(done.state, "completed");
            assert_eq!(
                done.discovery.unwrap().completed_steps,
                expected.len() + usize::from(workspace == Workspace::Osr)
            );
            for algorithm in &expected {
                assert_eq!(
                    done.overview.as_ref().unwrap().analyses[*algorithm].completed,
                    1
                );
            }
            c.start_system_analysis(workspace, false).await.unwrap();
            assert_eq!(finished(&c).await.classification.unwrap().requests, 0);
            no_extra_request(&listener).await;
            c.start_system_analysis(workspace, true).await.unwrap();
            let (_pending, _) = receive(&listener).await;
            c.cancel_analysis().await;
            assert_eq!(finished(&c).await.state, "cancelled");
            no_extra_request(&listener).await;
            let db = Catalogue::connect(&c.paths).await.unwrap();
            db.require_discovery_complete().await.unwrap();
            db.release().await.unwrap();
        }
    }

    #[tokio::test]
    async fn osr_level_stage_scopes_adventures_retries_escalates_reuses_and_cancels() {
        use crate::analysis::AnalysisWorkspace;
        let (_temp, c, listener) = parallel_fixture(2).await;
        let db = Catalogue::connect(&c.paths).await.unwrap();
        sqlx::query("UPDATE products SET record=json_set(record,'$.rule_systems',json_array('Old-School Revival (OSR)'))").execute(&db.pool).await.unwrap();
        complete_discovery(&db).await;
        sqlx::query("UPDATE book_classifications SET category='adventure' WHERE product_id='1' AND system='book_role'").execute(&db.pool).await.unwrap();
        let mut players = db.begin_players(false).await.unwrap();
        let b = db.players_next_after(&players, 0).await.unwrap().unwrap();
        let player_response = crate::players::from_response(
            crate::responses::decode(crate::players::tests::response(
                json!({"min_players":null,"max_players":null}),
            ))
            .unwrap(),
        )
        .unwrap();
        db.players_commit(&mut players, &b, Some(&player_response), None)
            .await
            .unwrap();
        db.finish_analysis(&mut players, "completed", "")
            .await
            .unwrap();
        db.require_discovery_complete().await.unwrap();
        db.release().await.unwrap();
        c.start_system_analysis(AnalysisWorkspace::Osr, false)
            .await
            .unwrap();
        for _ in 0..2 {
            let (mut socket, request) = receive(&listener).await;
            let mut answer = sequence_decision(&request);
            for a in answer["answers"].as_array_mut().unwrap() {
                if ["old_school_essentials", "dungeon_crawl_classics"]
                    .contains(&a["name"].as_str().unwrap())
                {
                    a["choice"] = json!("supported");
                    for p in a["probabilities"].as_array_mut().unwrap() {
                        p["probability"] = json!(if p["value"] == "supported" { 1.0 } else { 0.0 });
                    }
                }
            }
            socket
                .write_all(http(200, &answer.to_string(), "").as_bytes())
                .await
                .unwrap();
        }
        let (mut socket, request) = receive(&listener).await;
        assert_eq!(request["text"]["format"]["name"], "osr_levels");
        let evidence: serde_json::Value =
            serde_json::from_str(request["input"][1]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(
            evidence["requested_systems"],
            json!(["dungeon_crawl_classics", "old_school_essentials"])
        );
        assert!(
            evidence["book"]["title"]
                .as_str()
                .unwrap()
                .contains("adventure 1")
        );
        assert!(!evidence.to_string().contains("DO_NOT_SEND"));
        assert_eq!(c.snapshot().await.analysis.discovery.unwrap().step, 1);
        no_extra_request(&listener).await;
        socket
            .write_all(http(503, "SYNTHETIC", "").as_bytes())
            .await
            .unwrap();
        retry_waiting(&c, Some(503)).await;
        let (mut socket, retry) = receive(&listener).await;
        assert_eq!(retry, request);
        socket
            .write_all(
                http(
                    200,
                    &crate::players::tests::response(json!({"ranges":[]})).to_string(),
                    "",
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let (mut socket, larger) = receive(&listener).await;
        assert_eq!(larger["model"], "gpt-6.1-terra");
        assert_eq!(larger["input"], request["input"]);
        let values = json!({"ranges":[{"system":"old_school_essentials","game":null,"status":"known","min_level":3,"max_level":5},{"system":"dungeon_crawl_classics","game":null,"status":"known","min_level":0,"max_level":0}]});
        let mut response = crate::players::tests::response(values.clone());
        response["model"] = json!("gpt-6.1-terra");
        socket
            .write_all(http(200, &response.to_string(), "").as_bytes())
            .await
            .unwrap();
        let done = finished(&c).await;
        assert_eq!(done.state, "completed");
        assert_eq!((done.total, done.scored), (1, 1));
        assert_eq!(done.discovery.unwrap().completed_steps, 2);
        assert_eq!(
            done.overview.unwrap().analyses[crate::osr_levels::ALGORITHM].completed,
            1
        );
        assert_eq!(done.classification.unwrap().retries, 2);
        c.start_system_analysis(AnalysisWorkspace::Osr, false)
            .await
            .unwrap();
        assert_eq!(finished(&c).await.total, 0);
        no_extra_request(&listener).await;
        // Keep identical OSR memberships and cancel only the level replacement.
        c.start_system_analysis(AnalysisWorkspace::Osr, true)
            .await
            .unwrap();
        for _ in 0..2 {
            let (mut socket, request) = receive(&listener).await;
            let mut answer = sequence_decision(&request);
            for a in answer["answers"].as_array_mut().unwrap() {
                if ["old_school_essentials", "dungeon_crawl_classics"]
                    .contains(&a["name"].as_str().unwrap())
                {
                    a["choice"] = json!("supported");
                    for p in a["probabilities"].as_array_mut().unwrap() {
                        p["probability"] = json!(if p["value"] == "supported" { 1.0 } else { 0.0 });
                    }
                }
            }
            socket
                .write_all(http(200, &answer.to_string(), "").as_bytes())
                .await
                .unwrap();
        }
        let (_pending, body) = receive(&listener).await;
        assert_eq!(body["model"], "gpt-6-luna");
        c.cancel_analysis().await;
        assert_eq!(finished(&c).await.state, "cancelled");
        let db = Catalogue::connect(&c.paths).await.unwrap();
        let saved = crate::storage::osr_levels::saved(&db.pool, "drivethru", "1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.model, "gpt-6.1-terra");
        assert_eq!(saved.data.ranges.len(), 2);
        db.release().await.unwrap();
        c.shutdown().await;
    }
    #[tokio::test]
    async fn osr_sequence_filters_before_dispatch_and_retries_without_classifying_other_books() {
        use crate::analysis::AnalysisWorkspace;
        let (_temp, c, listener) = parallel_fixture(3).await;
        let db = Catalogue::connect(&c.paths).await.unwrap();
        sqlx::query("UPDATE products SET record=json_set(record,'$.rule_systems',CASE WHEN product_id='1' THEN json_array('Old-School Revival (OSR)') ELSE json_array('OSRIC') END,'$.genres',json_array('Old-School Revival (OSR)'))").execute(&db.pool).await.unwrap();
        complete_discovery(&db).await;
        db.release().await.unwrap();
        c.start_system_analysis(AnalysisWorkspace::Osr, false)
            .await
            .unwrap();
        let (mut socket, body) = receive(&listener).await;
        assert_eq!(body["questions"].as_array().unwrap().len(), 15);
        assert_eq!(body["model"], decisions::MODEL);
        assert_eq!(body["prompt_cache_options"]["mode"], "explicit");
        assert!(!body.to_string().contains("prompt_cache_breakpoint"));
        assert!(!body.to_string().contains("DO_NOT_SEND"));
        assert!(body.to_string().contains("Synthetic adventure 1"));
        no_extra_request(&listener).await;
        socket
            .write_all(http(503, "SYNTHETIC", "").as_bytes())
            .await
            .unwrap();
        retry_waiting(&c, Some(503)).await;
        let (mut socket, repeated) = receive(&listener).await;
        assert_eq!(body, repeated);
        socket
            .write_all(http(200, &sequence_decision(&body).to_string(), "").as_bytes())
            .await
            .unwrap();
        let done = finished(&c).await;
        assert_eq!((done.total, done.scored), (0, 0));
        assert_eq!(done.discovery.as_ref().unwrap().completed_steps, 2);
        let db = Catalogue::connect(&c.paths).await.unwrap();
        let retries:i64=sqlx::query_scalar("SELECT json_extract(classification,'$.retries') FROM analysis_runs WHERE algorithm='osr-systems-1' ORDER BY id DESC LIMIT 1").fetch_one(&db.pool).await.unwrap();
        assert_eq!(retries, 1);
        db.release().await.unwrap();
        assert_eq!(
            done.overview.as_ref().unwrap().analyses[crate::osr::ALGORITHM].counts["not_osr"],
            1
        );
        c.start_system_analysis(AnalysisWorkspace::Osr, false)
            .await
            .unwrap();
        assert_eq!(finished(&c).await.classification.unwrap().requests, 0);
        no_extra_request(&listener).await;
    }
    #[tokio::test]
    async fn discovery_sequence_orders_stages_reuses_results_and_reruns_explicitly() {
        let (_temp, c, listener) = parallel_fixture(1).await;
        c.start_discovery_analysis(false).await.unwrap();
        assert!(c.start_discovery_analysis(false).await.is_err());
        let (mut role, request) = receive(&listener).await;
        assert_eq!(request["questions"][0]["name"], "primary_book_role");
        assert_eq!(c.snapshot().await.analysis.discovery.unwrap().step, 1);
        assert!(c.start_analysis().await.is_err());
        assert!(c.start(false).await.is_err());
        assert!(c.initialize().await.is_err());
        assert!(Catalogue::connect(&c.paths).await.is_err());
        role.write_all(http(503, "SYNTHETIC", "").as_bytes())
            .await
            .unwrap();
        retry_waiting(&c, Some(503)).await;
        assert_eq!(
            c.snapshot()
                .await
                .analysis
                .discovery
                .unwrap()
                .completed_steps,
            1
        );
        let (mut retry, repeated) = receive(&listener).await;
        assert_eq!(request, repeated);
        retry.write_all(success().as_bytes()).await.unwrap();
        let (mut solo, request) = receive(&listener).await;
        assert_eq!(request["questions"][0]["name"], "solo_play");
        assert_eq!(c.snapshot().await.analysis.discovery.unwrap().step, 2);
        solo.write_all(discovery_solo_response().as_bytes())
            .await
            .unwrap();
        let (mut players, request) = receive(&listener).await;
        assert!(request["input"][0]["content"].is_array());
        assert_eq!(c.snapshot().await.analysis.discovery.unwrap().step, 3);
        let unknown =
            crate::players::tests::response(json!({"min_players":null,"max_players":null}));
        players
            .write_all(http(200, &unknown.to_string(), "").as_bytes())
            .await
            .unwrap();
        let done = finished(&c).await;
        assert_eq!(done.state, "completed");
        assert_eq!(done.discovery.as_ref().unwrap().completed_steps, 4);
        assert_eq!(done.discovery.as_ref().unwrap().unsuccessful, 0);
        for algorithm in [
            crate::analysis::ALGORITHM,
            crate::classification::ALGORITHM,
            crate::solo::ALGORITHM,
            crate::players::ALGORITHM,
        ] {
            assert_eq!(
                done.overview.as_ref().unwrap().analyses[algorithm].completed,
                1
            );
        }
        let pool = c.tx.borrow().catalogue.as_ref().unwrap().pool.clone();
        let runs: Vec<String> =
            sqlx::query_scalar("SELECT algorithm FROM analysis_runs ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(
            runs,
            [
                crate::analysis::ALGORITHM,
                crate::classification::ALGORITHM,
                crate::solo::ALGORITHM,
                crate::players::ALGORITHM
            ]
        );
        let score_run: i64 = sqlx::query_scalar("SELECT run_id FROM relevance_scores")
            .fetch_one(&pool)
            .await
            .unwrap();
        c.start_discovery_analysis(false).await.unwrap();
        assert_eq!(finished(&c).await.discovery.unwrap().completed_steps, 4);
        no_extra_request(&listener).await;
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT run_id FROM relevance_scores")
                .fetch_one(&pool)
                .await
                .unwrap(),
            score_run
        );
        c.start_discovery_analysis(true).await.unwrap();
        let refusal = http(200, &json!({"model":decisions::MODEL,"answers":[{"type":"refusal","name":"primary_book_role"}],"usage":{"input_tokens":400,"output_tokens":0,"total_tokens":400,"input_tokens_details":{"cached_tokens":0,"cache_write_tokens":0}}}).to_string(), "");
        for (name, response) in [
            ("primary_book_role", refusal),
            ("solo_play", discovery_solo_response()),
        ] {
            let (mut socket, request) = receive(&listener).await;
            assert_eq!(request["questions"][0]["name"], name);
            assert!(c.snapshot().await.analysis.discovery.unwrap().rerun_all);
            socket.write_all(response.as_bytes()).await.unwrap();
        }
        let (mut socket, _) = receive(&listener).await;
        socket
            .write_all(http(200, &unknown.to_string(), "").as_bytes())
            .await
            .unwrap();
        let rerun_done = finished(&c).await;
        assert_eq!(rerun_done.state, "completed");
        assert_eq!(rerun_done.discovery.unwrap().unsuccessful, 1);
        assert_ne!(
            sqlx::query_scalar::<_, i64>("SELECT run_id FROM relevance_scores")
                .fetch_one(&pool)
                .await
                .unwrap(),
            score_run
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM book_classifications WHERE system='dnd'"
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            0
        );
        no_extra_request(&listener).await;
        c.shutdown().await;
    }
    #[tokio::test]
    async fn discovery_cancel_and_close_drain_without_starting_later_stages() {
        let (_temp, c, listener) = parallel_fixture(1).await;
        c.start_discovery_analysis(false).await.unwrap();
        let (mut role, _) = receive(&listener).await;
        role.write_all(success().as_bytes()).await.unwrap();
        let (_pending_solo, _) = receive(&listener).await;
        c.cancel_analysis().await;
        let done = finished(&c).await;
        assert_eq!(done.state, "cancelled");
        assert_eq!(done.discovery.unwrap().completed_steps, 2);
        no_extra_request(&listener).await;
        let next = Catalogue::connect(&c.paths).await.unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM analysis_runs WHERE algorithm='adventure-players-1'"
            )
            .fetch_one(&next.pool)
            .await
            .unwrap(),
            0
        );
        next.release().await.unwrap();
        c.start_discovery_analysis(false).await.unwrap();
        let (mut solo, request) = receive(&listener).await;
        assert_eq!(request["questions"][0]["name"], "solo_play");
        solo.write_all(discovery_solo_response().as_bytes())
            .await
            .unwrap();
        let (_pending_players, _) = receive(&listener).await;
        tokio::time::timeout(Duration::from_secs(5), c.shutdown())
            .await
            .unwrap();
        assert_eq!(c.snapshot().await.analysis.state, "cancelled");
        assert!(c.start_discovery_analysis(false).await.is_err());
        no_extra_request(&listener).await;
        let next = Catalogue::connect(&c.paths).await.unwrap();
        assert_eq!(next.begin_players(false).await.unwrap().total, 1);
        next.release().await.unwrap();
    }
    #[tokio::test]
    async fn discovery_local_failure_stops_the_sequence_and_releases_ownership() {
        let (_temp, c, listener) = parallel_fixture(1).await;
        let db = Catalogue::connect(&c.paths).await.unwrap();
        sqlx::raw_sql("CREATE TRIGGER fail_discovery BEFORE INSERT ON book_classifications BEGIN SELECT RAISE(ABORT,'Synthetic storage failure'); END").execute(&db.pool).await.unwrap();
        db.release().await.unwrap();
        c.start_discovery_analysis(false).await.unwrap();
        let (mut role, _) = receive(&listener).await;
        role.write_all(success().as_bytes()).await.unwrap();
        let failed = finished(&c).await;
        assert_eq!(failed.state, "failed");
        assert_eq!(failed.discovery.unwrap().completed_steps, 1);
        no_extra_request(&listener).await;
        let next = Catalogue::connect(&c.paths).await.unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM analysis_runs")
                .fetch_one(&next.pool)
                .await
                .unwrap(),
            2
        );
        next.release().await.unwrap();
        c.shutdown().await;
    }
}
