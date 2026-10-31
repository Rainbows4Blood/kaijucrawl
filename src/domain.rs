use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub const PARSER_VERSION: &str = "drivethru-rust-1";
pub const SCHEMA_VERSION: i32 = 2;
pub const CATALOGUE_VERSION: i32 = 16;
pub const PRODUCT_LIMIT: usize = 16 * 1024 * 1024;
pub const SITEMAP_LIMIT: usize = 128 * 1024 * 1024;
pub const ROBOTS_LIMIT: usize = 1024 * 1024;
pub const WORKER_OUTPUT_LIMIT: usize = 128 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Invalid configuration: {0}")]
    Config(String),
    #[error("Local catalogue operation failed; check disk space and file permissions")]
    Database(#[from] sqlx::Error),
    #[error("Catalogue ownership unavailable or lost")]
    Ownership,
    #[error("Archive integrity check failed")]
    Integrity,
    #[error("{0}")]
    Job(#[from] Failure),
    #[error("Local I/O failed")]
    Io(#[from] std::io::Error),
    #[error("Invalid local configuration or worker protocol")]
    Json(#[from] serde_json::Error),
    #[error("Operation cancelled")]
    Cancelled,
    #[error("Operation deadline exceeded")]
    Deadline,
}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProductId(String);
impl ProductId {
    pub fn new(s: &str) -> Result<Self> {
        if s.is_empty()
            || !s.bytes().all(|c| c.is_ascii_digit())
            || s.trim_start_matches('0').is_empty()
        {
            return Err(Error::Config(
                "Product ID must be a positive decimal integer".into(),
            ));
        }
        Ok(Self(s.trim_start_matches('0').into()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    Transient,
    Missing,
    Incomplete,
    Schema,
    Restricted,
    Authentication,
    Policy,
    Robots,
    ParseTimeout,
    Interrupted,
}
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
#[error("{kind:?}: {reason}")]
pub struct Failure {
    pub kind: FailureKind,
    pub reason: String,
    pub status: Option<u16>,
    pub missing_fields: Vec<String>,
    pub retry_after_secs: u64,
}
impl Failure {
    pub fn new(kind: FailureKind, reason: &str) -> Self {
        Self {
            kind,
            reason: reason.into(),
            status: None,
            missing_fields: vec![],
            retry_after_secs: 0,
        }
    }
    pub fn pauses(&self) -> bool {
        matches!(self.kind, FailureKind::Authentication | FailureKind::Policy)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Product {
    pub target_key: String,
    pub product_id: ProductId,
    pub title: String,
    pub description_text: String,
    #[serde(flatten)]
    pub fields: serde_json::Map<String, Value>,
}
impl Product {
    pub fn validate(&self) -> Result<()> {
        let mut f = Failure::new(FailureKind::Incomplete, "Missing required metadata");
        if self.title.trim().is_empty() {
            f.missing_fields.push("title".into());
        }
        if self.description_text.trim().is_empty() {
            f.missing_fields.push("description_text".into());
        }
        if f.missing_fields.is_empty() {
            Ok(())
        } else {
            Err(f.into())
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Archive {
    pub key: String,
    pub sha256: String,
    pub decoded_bytes: i64,
    pub compressed_bytes: i64,
    pub compression: String,
    pub archived_at: DateTime<Utc>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Intent {
    pub id: Uuid,
    pub target: String,
    pub product_id: String,
    pub fetched_at: DateTime<Utc>,
    pub api_url: String,
    pub http: Value,
    pub archive: Archive,
}
#[derive(Clone, Debug)]
pub struct Claim {
    pub id: i64,
    pub target: String,
    pub identity: String,
    pub url: String,
    pub kind: String,
    pub attempts: i32,
    pub run: Uuid,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Metrics {
    pub successful: i64,
    pub last_saved: Option<DateTime<Utc>>,
    pub saved_run: i64,
    pub pending: i64,
    pub active: i64,
    pub retry: i64,
    pub failed: i64,
    pub persistence: i64,
    pub sitemaps_pending: i64,
    pub sitemaps_retry: i64,
    pub sitemaps_active: i64,
    pub sitemaps_done: i64,
    pub sitemaps_failed: i64,
    pub seen: i64,
    pub discovered: i64,
    pub next_due: Option<DateTime<Utc>>,
}

/// Current request-start intervals, not request durations or a queue ETA.
#[derive(Clone, Copy, Debug)]
pub struct Pacing {
    pub workers: usize,
    pub website: Option<std::time::Duration>,
    pub api: Option<std::time::Duration>,
    pub cooldown: std::time::Duration,
}
