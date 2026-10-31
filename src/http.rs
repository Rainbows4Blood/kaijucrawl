use crate::{auth::Cookies, config::RuntimeSettings, domain::*, targets::drivethru as d};
use futures_util::{StreamExt, future::BoxFuture};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::Read,
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};
use tokio::sync::{Mutex, RwLock, Semaphore};
use tokio_util::sync::CancellationToken;
use url::Url;
#[cfg(test)]
mod tests;

pub type Guard = Arc<dyn Fn() -> BoxFuture<'static, Result<()>> + Send + Sync>;
pub type Cooldown = Arc<dyn Fn(u64) -> BoxFuture<'static, Result<()>> + Send + Sync>;
#[derive(Clone)]
pub struct Limiter {
    cooldown: Arc<Mutex<Instant>>,
    slots: Arc<Semaphore>,
}
impl Limiter {
    pub fn new(workers: usize) -> Self {
        Self {
            cooldown: Arc::new(Mutex::new(Instant::now())),
            slots: Arc::new(Semaphore::new(workers)),
        }
    }
    pub async fn pace(&self, cancel: &CancellationToken) -> Result<()> {
        loop {
            let due = *self.cooldown.lock().await;
            if due <= Instant::now() {
                break;
            }
            tokio::select! {_ = cancel.cancelled()=>return Err(Error::Cancelled),_ = tokio::time::sleep_until(due.into())=>{}}
        }
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        Ok(())
    }
    pub async fn cooldown(&self, seconds: u64) {
        let mut until = self.cooldown.lock().await;
        *until = (*until).max(Instant::now() + Duration::from_secs(seconds.min(31_536_000)));
    }
}
#[derive(Default)]
struct OriginPacer {
    interval: RwLock<Duration>,
    last_start: Mutex<Option<Instant>>,
}

#[derive(Clone)]
pub struct Policy {
    pub body: String,
    pub sitemaps: Vec<String>,
    pub interval: Duration,
}
type RobotsGroup = (Vec<String>, Vec<(String, String)>);
impl Policy {
    pub fn parse(body: String, agent: &str) -> Result<Self> {
        if body.len() > ROBOTS_LIMIT || body.trim_start().starts_with(['<', '{', '[']) {
            return Err(
                Failure::new(FailureKind::Policy, "Robots response is not a policy").into(),
            );
        }
        let mut sitemaps = vec![];
        let mut groups: Vec<RobotsGroup> = vec![];
        let mut agents = vec![];
        let mut rules = vec![];
        for line in body.lines().chain(std::iter::once("User-agent: __end__")) {
            let line = line.split('#').next().unwrap_or("").trim();
            let Some((k, v)) = line.split_once(':') else {
                continue;
            };
            let k = k.trim().to_ascii_lowercase();
            let v = v.trim();
            if k == "sitemap" {
                if let Some(u) = d::sitemap_url(v) {
                    sitemaps.push(u);
                }
                continue;
            }
            if k == "user-agent" {
                if !rules.is_empty() {
                    groups.push((std::mem::take(&mut agents), std::mem::take(&mut rules)));
                }
                agents.push(v.to_lowercase());
            } else if !agents.is_empty() {
                rules.push((k, v.into()));
            }
        }
        let agent = agent.to_ascii_lowercase();
        let score = |as_: &Vec<String>| {
            as_.iter()
                .filter_map(|a| {
                    if a == "*" {
                        Some(0)
                    } else if agent.contains(a) {
                        Some(a.len())
                    } else {
                        None
                    }
                })
                .max()
        };
        let best = groups.iter().filter_map(|(a, _)| score(a)).max();
        let mut delay = Duration::ZERO;
        for (a, rules) in &groups {
            if score(a) != best || best.is_none() {
                continue;
            }
            for (k, v) in rules {
                let secs = if k == "crawl-delay" {
                    v.parse::<f64>().ok()
                } else if k == "request-rate" {
                    v.split_once('/')
                        .and_then(|(n, s)| Some(s.parse::<f64>().ok()? / n.parse::<f64>().ok()?))
                } else {
                    None
                };
                if let Some(s) = secs
                    && s.is_finite()
                    && (0.0..=31_536_000.0).contains(&s)
                {
                    delay = delay.max(Duration::from_secs_f64(s));
                }
            }
        }
        Ok(Self {
            body,
            sitemaps,
            interval: delay,
        })
    }
    pub fn allows(&self, agent: &str, url: &str) -> bool {
        let product = agent.split(['/', ' ']).next().unwrap_or(agent);
        robotstxt::DefaultMatcher::default().one_agent_allowed_by_robots(&self.body, product, url)
    }
}
pub struct Response {
    pub body: Vec<u8>,
    pub url: String,
    pub http: Value,
    pub status: u16,
}
#[derive(Clone)]
pub struct Network {
    client: reqwest::Client,
    pub limiter: Limiter,
    cookies: Arc<RwLock<Arc<Cookies>>>,
    policies: Arc<RwLock<HashMap<String, Policy>>>,
    origin_pacers: Arc<RwLock<HashMap<String, Arc<OriginPacer>>>>,
    settings: RuntimeSettings,
    guard: Guard,
    cooldown_sink: Option<Cooldown>,
    starts: Arc<RwLock<CancellationToken>>,
    #[cfg(test)]
    pub(crate) fixture_endpoint: Option<Url>,
}
impl Network {
    pub fn active(&self) -> usize {
        self.settings
            .workers
            .saturating_sub(self.limiter.slots.available_permits())
    }
    pub fn new(s: &RuntimeSettings, cookies: Cookies, guard: Guard) -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(&s.user_agent)
            .connect_timeout(Duration::from_secs(s.connect_secs))
            .read_timeout(Duration::from_secs(s.read_secs))
            .timeout(Duration::from_secs(s.total_secs))
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .build()
            .map_err(|_| Error::Config("Cannot construct HTTPS client".into()))?;
        Ok(Self {
            client,
            limiter: Limiter::new(s.workers),
            cookies: Arc::new(RwLock::new(Arc::new(cookies))),
            policies: Arc::new(RwLock::new(HashMap::new())),
            origin_pacers: Arc::new(RwLock::new(HashMap::new())),
            settings: s.clone(),
            guard,
            cooldown_sink: None,
            starts: Arc::new(RwLock::new(CancellationToken::new())),
            #[cfg(test)]
            fixture_endpoint: None,
        })
    }
    pub fn with_cooldown(mut self, sink: Cooldown) -> Self {
        self.cooldown_sink = Some(sink);
        self
    }
    async fn origin_pacer(&self, origin: &str) -> Arc<OriginPacer> {
        if let Some(pacer) = self.origin_pacers.read().await.get(origin) {
            return pacer.clone();
        }
        self.origin_pacers
            .write()
            .await
            .entry(origin.into())
            .or_default()
            .clone()
    }
    async fn pace_origin(&self, origin: &str, cancel: &CancellationToken) -> Result<()> {
        let pacer = self.origin_pacer(origin).await;
        let mut last = tokio::select! {
            _ = cancel.cancelled() => return Err(Error::Cancelled),
            last = pacer.last_start.lock() => last,
        };
        if let Some(previous) = *last {
            let due = previous + *pacer.interval.read().await;
            if due > Instant::now() {
                tokio::select! {
                    _ = cancel.cancelled() => return Err(Error::Cancelled),
                    _ = tokio::time::sleep_until(due.into()) => {}
                }
            }
        }
        // Only this origin's robots policy adds normal pacing. A server
        // cooldown remains shared by the target, without any minimum interval.
        self.limiter.pace(cancel).await?;
        *last = Some(Instant::now());
        Ok(())
    }
    pub async fn pacing(&self) -> Pacing {
        let policies = self.policies.read().await;
        let effective = |origin: &str| {
            policies
                .get(origin.trim_end_matches('/'))
                .map(|policy| policy.interval)
        };
        Pacing {
            workers: self.settings.workers,
            website: effective(d::WEBSITE),
            api: effective(d::API),
            cooldown: self
                .limiter
                .cooldown
                .lock()
                .await
                .saturating_duration_since(Instant::now()),
        }
    }
    pub async fn replace(&self, cookies: Cookies) {
        *self.cookies.write().await = Arc::new(cookies);
        self.policies.write().await.clear();
        *self.starts.write().await = CancellationToken::new();
    }
    pub async fn suspend(&self) {
        self.starts.read().await.cancel();
    }
    pub async fn refresh_robots(&self, cancel: &CancellationToken) -> Result<Vec<String>> {
        let mut declarations = vec![];
        self.policies.write().await.clear();
        for origin in [d::WEBSITE, d::API] {
            let (_, sitemaps) = self.load_robots(origin, cancel).await?;
            declarations.extend(sitemaps);
        }
        Ok(declarations)
    }
    /// Load each approved origin independently so diagnostics can report both.
    /// Other requests still require a successfully loaded policy.
    pub(crate) async fn load_robots(
        &self,
        origin: &str,
        cancel: &CancellationToken,
    ) -> Result<(u16, Vec<String>)> {
        self.policies
            .write()
            .await
            .remove(origin.trim_end_matches('/'));
        let url = format!("{origin}robots.txt");
        for attempt in 0..self.settings.attempts {
            let response = self
                .fetch(&url, None, ROBOTS_LIMIT, true, cancel, None)
                .await;
            let (status, body) = match response {
                Ok(r) => (r.status, r.body),
                Err(Error::Job(f)) if f.kind == FailureKind::Missing => {
                    (f.status.unwrap_or(404), Vec::new())
                }
                Err(Error::Job(f))
                    if f.kind == FailureKind::Transient && attempt + 1 < self.settings.attempts =>
                {
                    self.limiter
                        .cooldown(
                            f.retry_after_secs
                                .max(10 * 2u64.saturating_pow(attempt as u32).min(360)),
                        )
                        .await;
                    continue;
                }
                Err(Error::Job(mut f)) => {
                    if f.kind != FailureKind::Authentication {
                        f.kind = FailureKind::Policy;
                    }
                    return Err(f.into());
                }
                Err(e) => return Err(e),
            };
            let policy = String::from_utf8(body)
                .map_err(|_| {
                    Error::from(Failure::new(
                        FailureKind::Policy,
                        "Robots encoding unsupported",
                    ))
                })
                .and_then(|body| Policy::parse(body, &self.settings.robots_agent))
                .map_err(|e| match e {
                    Error::Job(mut f) => {
                        f.status = Some(status);
                        Error::Job(f)
                    }
                    e => e,
                })?;
            // Replace the policy for this origin only, including a reduced or
            // removed delay after refresh. Last start times survive refreshes.
            *self
                .origin_pacer(origin.trim_end_matches('/'))
                .await
                .interval
                .write()
                .await = policy.interval;
            let declarations = policy.sitemaps.clone();
            self.policies
                .write()
                .await
                .insert(origin.trim_end_matches('/').into(), policy);
            return Ok((status, declarations));
        }
        Err(Failure::new(FailureKind::Policy, "No robots attempts configured").into())
    }
    pub async fn get(
        &self,
        url: &str,
        product: Option<&str>,
        limit: usize,
        cancel: &CancellationToken,
    ) -> Result<Response> {
        self.fetch(url, product, limit, false, cancel, None).await
    }
    pub async fn get_job(
        &self,
        url: &str,
        product: Option<&str>,
        limit: usize,
        cancel: &CancellationToken,
        start: Guard,
    ) -> Result<Response> {
        self.fetch(url, product, limit, false, cancel, Some(start))
            .await
    }
    #[allow(clippy::too_many_arguments)]
    async fn fetch(
        &self,
        url: &str,
        product: Option<&str>,
        limit: usize,
        robots: bool,
        cancel: &CancellationToken,
        start: Option<Guard>,
    ) -> Result<Response> {
        let original = Url::parse(url)
            .map_err(|_| Failure::new(FailureKind::Schema, "Invalid request URL"))?;
        if !d::approved(&original, false) && !d::approved(&original, true) {
            return Err(Failure::new(FailureKind::Schema, "Unapproved request origin").into());
        }
        self.fetch_approved(original, product, limit, robots, cancel, start)
            .await
    }
    async fn fetch_approved(
        &self,
        original: Url,
        product: Option<&str>,
        limit: usize,
        robots: bool,
        cancel: &CancellationToken,
        mut start: Option<Guard>,
    ) -> Result<Response> {
        let permit = tokio::select! {_ = cancel.cancelled()=>return Err(Error::Cancelled),p=self.limiter.slots.clone().acquire_owned()=>p.map_err(|_|Error::Cancelled)?};
        let cookies = self.cookies.read().await.clone();
        let mut url = original.clone();
        let mut deadline: Option<Instant> = None;
        let operation = async {
            for _ in 0..=if robots { 6 } else { 8 } {
                if !robots {
                    let ps = self.policies.read().await;
                    let policy = ps.get(&url.origin().ascii_serialization()).ok_or_else(|| {
                        Failure::new(FailureKind::Policy, "Robots policy not loaded")
                    })?;
                    if !policy.allows(&self.settings.robots_agent, url.as_str()) {
                        return Err(
                            Failure::new(FailureKind::Robots, "Excluded by robots policy").into(),
                        );
                    }
                }
                let starts = self.starts.read().await.clone();
                let origin = url.origin().ascii_serialization();
                tokio::select! { _=starts.cancelled()=>return Err(Error::Cancelled), result=self.pace_origin(&origin, cancel)=>result? }
                (self.guard)().await?;
                let deadline = *deadline.get_or_insert_with(|| {
                    Instant::now() + Duration::from_secs(self.settings.total_secs)
                });
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(Failure::new(
                        FailureKind::Transient,
                        "Total response deadline exceeded",
                    )
                    .into());
                }
                #[allow(unused_mut)]
                let mut destination = url.clone();
                #[cfg(test)]
                if let Some(endpoint) = &self.fixture_endpoint {
                    destination = endpoint.clone();
                    destination.set_path(url.path());
                    destination.set_query(url.query());
                }
                let mut req = self
                    .client
                    .get(destination)
                    .timeout(remaining)
                    .header("Accept-Encoding", "gzip")
                    .header(
                        "Accept",
                        if product.is_some() {
                            "application/json"
                        } else {
                            "*/*"
                        },
                    );
                if let Some(c) = cookies.header(&url) {
                    let mut h = reqwest::header::HeaderValue::from_str(&c).map_err(|_| {
                        Failure::new(
                            FailureKind::Authentication,
                            "Invalid configured cookie header",
                        )
                    })?;
                    h.set_sensitive(true);
                    req = req.header(reqwest::header::COOKIE, h);
                }
                if let Some(start) = start.take() {
                    start().await?;
                }
                let response = req.send().await.map_err(|_| {
                    Failure::new(FailureKind::Transient, "Transport or response timeout")
                })?;
                let status = response.status().as_u16();
                let headers = response.headers().clone();
                if response.status().is_redirection() && status != 304 {
                    let next = headers
                        .get("location")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| url.join(v).ok())
                        .ok_or_else(|| Failure::new(FailureKind::Schema, "Invalid redirect"))?;
                    if !d::redirect(&original, &next, product) {
                        return Err(Failure::new(
                            FailureKind::Authentication,
                            "Redirect left approved request scope; inspect authentication",
                        )
                        .into());
                    }
                    url = next;
                    continue;
                }
                if response.content_length().is_some_and(|n| n > limit as u64) {
                    return Err(Failure::new(
                        FailureKind::Schema,
                        "Encoded response exceeds resource limit",
                    )
                    .into());
                }
                let mut stream = response.bytes_stream();
                let mut body = Vec::new();
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk.map_err(|_| {
                        Failure::new(FailureKind::Transient, "Response read failed")
                    })?;
                    if body.len() + chunk.len() > limit {
                        return Err(Failure::new(
                            FailureKind::Schema,
                            "Encoded response exceeds resource limit",
                        )
                        .into());
                    }
                    body.extend_from_slice(&chunk);
                }
                if headers.get("content-encoding").is_some_and(|h| h == "gzip") {
                    body = tokio::task::spawn_blocking(move || {
                        let mut decoded = Vec::new();
                        flate2::read::MultiGzDecoder::new(body.as_slice())
                            .take(limit as u64 + 1)
                            .read_to_end(&mut decoded)
                            .map_err(|_| {
                                Failure::new(FailureKind::Schema, "Malformed gzip response")
                            })?;
                        if decoded.len() > limit {
                            return Err(Failure::new(
                                FailureKind::Schema,
                                "Decoded response exceeds resource limit",
                            ));
                        }
                        Ok(decoded)
                    })
                    .await
                    .map_err(|_| Error::Cancelled)??;
                } else if headers
                    .get("content-encoding")
                    .is_some_and(|h| h != "identity")
                {
                    return Err(
                        Failure::new(FailureKind::Schema, "Unsupported content encoding").into(),
                    );
                }
                let failure = if headers
                    .get("cf-mitigated")
                    .is_some_and(|h| h == "challenge")
                {
                    let mut f = Failure::new(
                        FailureKind::Authentication,
                        "Cloudflare challenge (cf-mitigated: challenge); browser login does not establish crawler access",
                    );
                    f.status = Some(status);
                    Some(f)
                } else if !robots && product.is_some() && d::approved(&url, true) {
                    d::classify_product_api(status, &body)
                } else {
                    d::classify(status, &body)
                };
                if let Some(mut failure) = failure {
                    failure.retry_after_secs = failure.retry_after_secs.max(
                        headers
                            .get("retry-after")
                            .and_then(|v| v.to_str().ok())
                            .map(retry_after)
                            .unwrap_or(0),
                    );
                    if failure.retry_after_secs > 0 {
                        self.limiter.cooldown(failure.retry_after_secs).await;
                        if let Some(sink) = &self.cooldown_sink {
                            sink(failure.retry_after_secs).await?;
                        }
                    }
                    return Err(failure.into());
                }
                let mut http = json!({"status":status});
                for h in ["etag", "last-modified"] {
                    if let Some(v) = headers.get(h).and_then(|v| v.to_str().ok())
                        && v.len() <= 1024
                    {
                        http[h] = json!(v);
                    }
                }
                return Ok(Response {
                    body,
                    url: url.to_string(),
                    http,
                    status,
                });
            }
            Err(Failure::new(FailureKind::Schema, "Redirect limit exceeded").into())
        };
        let result = tokio::select! {_ = cancel.cancelled()=>Err(Error::Cancelled),r=operation=>r};
        drop(permit);
        result
    }
}
pub fn retry_after(s: &str) -> u64 {
    s.parse::<u64>()
        .ok()
        .or_else(|| {
            httpdate::parse_http_date(s)
                .ok()
                .and_then(|t| t.duration_since(SystemTime::now()).ok())
                .map(|d| d.as_secs().saturating_add(1))
        })
        .unwrap_or(0)
        .min(31_536_000)
}
