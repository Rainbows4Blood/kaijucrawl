use crate::{
    config::RuntimeSettings,
    domain::*,
    paths::LocalPaths,
    targets::{drivethru as d, sitemap::Entry},
};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::{
    Row, Sqlite, SqlitePool, Transaction,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};
use std::{
    fs::File,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

struct WriterLock(File);
impl Drop for WriterLock {
    fn drop(&mut self) {
        // Unlock explicitly: a forked child can briefly hold an inherited handle
        // before exec closes it. Closing only this descriptor could retain the lock.
        let _ = self.0.unlock();
    }
}

#[derive(Clone)]
pub struct Catalogue {
    pub pool: SqlitePool,
    pub catalogue: Uuid,
    pub owner: Uuid,
    pub generation: i64,
    lock: Arc<Mutex<Option<WriterLock>>>,
    paths: LocalPaths,
}
impl std::fmt::Debug for Catalogue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Catalogue")
            .field("catalogue", &self.catalogue)
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}
impl Catalogue {
    pub async fn connect(paths: &LocalPaths) -> Result<Self> {
        Self::connect_cancel(paths, &CancellationToken::new()).await
    }
    pub async fn connect_cancel(paths: &LocalPaths, cancel: &CancellationToken) -> Result<Self> {
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let paths = paths.clone();
        let lock_path = paths.clone();
        let lock = tokio::task::spawn_blocking(move || -> Result<WriterLock> {
            lock_path.ensure()?;
            let file = crate::paths::private_file(&lock_path.lock())?;
            file.try_lock().map_err(|e| match e {
                std::fs::TryLockError::WouldBlock => Error::Ownership,
                std::fs::TryLockError::Error(e) => Error::Io(e),
            })?;
            let lock = WriterLock(file);
            let database = lock_path.database();
            if !database.exists() {
                crate::paths::private_file(&database)?;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                // Do not open/close an existing SQLite file outside SQLite:
                // doing so could disturb its process-scoped POSIX locks.
                std::fs::set_permissions(database, std::fs::Permissions::from_mode(0o600))?;
            }
            Ok(lock)
        })
        .await
        .map_err(|_| Error::Cancelled)??;
        let options = SqliteConnectOptions::new()
            .collation("exact_decimal", super::legacy::decimal_order)
            .filename(paths.database())
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Full)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options)
            .await?;
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
        sqlx::raw_sql("CREATE TABLE IF NOT EXISTS catalogue(singleton INTEGER PRIMARY KEY CHECK(singleton=1),id BLOB NOT NULL,version INTEGER NOT NULL); CREATE TABLE IF NOT EXISTS ownership(singleton INTEGER PRIMARY KEY CHECK(singleton=1),owner BLOB,generation INTEGER NOT NULL DEFAULT 0);").execute(&mut *tx).await?;
        sqlx::query("INSERT INTO catalogue VALUES(1,$1,0) ON CONFLICT DO NOTHING")
            .bind(Uuid::new_v4())
            .execute(&mut *tx)
            .await?;
        let meta = sqlx::query("SELECT id,version FROM catalogue WHERE singleton=1")
            .fetch_one(&mut *tx)
            .await?;
        let version: i32 = meta.get("version");
        if version > CATALOGUE_VERSION {
            return Err(Error::Config(
                "Catalogue schema is newer than this executable; upgrade kaijucrawl".into(),
            ));
        }
        if !matches!(
            version,
            0 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 | 13 | 14 | 15 | CATALOGUE_VERSION
        ) {
            return Err(Error::Config(
                "Unsupported local catalogue schema; restore a compatible catalogue".into(),
            ));
        }
        if version == 0 {
            sqlx::raw_sql(include_str!("../../migrations/002_local.sql"))
                .execute(&mut *tx)
                .await?;
        }
        if version < 3 {
            sqlx::raw_sql(include_str!("../../migrations/003_queue_throughput.sql"))
                .execute(&mut *tx)
                .await?;
        }
        if version < 4 {
            sqlx::raw_sql(include_str!("../../migrations/004_search.sql"))
                .execute(&mut *tx)
                .await?;
        }
        if version < 5 {
            sqlx::raw_sql(include_str!("../../migrations/005_analysis.sql"))
                .execute(&mut *tx)
                .await?;
        }
        if version < 6 {
            sqlx::raw_sql(include_str!("../../migrations/006_classification.sql"))
                .execute(&mut *tx)
                .await?;
        }
        if version < 7 {
            sqlx::raw_sql(include_str!("../../migrations/007_adventure_players.sql"))
                .execute(&mut *tx)
                .await?;
        }
        if version < 8 {
            sqlx::raw_sql(include_str!("../../migrations/008_solo_play.sql"))
                .execute(&mut *tx)
                .await?;
        }
        if version < 9 {
            sqlx::raw_sql(include_str!("../../migrations/009_dnd.sql"))
                .execute(&mut *tx)
                .await?;
        }
        if version < 10 {
            sqlx::raw_sql(include_str!("../../migrations/010_remove_search.sql"))
                .execute(&mut *tx)
                .await?;
        }
        if version < 11 {
            sqlx::raw_sql(include_str!("../../migrations/011_dnd_context.sql"))
                .execute(&mut *tx)
                .await?;
        }
        if version < 12 {
            sqlx::raw_sql(include_str!("../../migrations/012_earth_context.sql"))
                .execute(&mut *tx)
                .await?;
        }

        if version < 13 {
            sqlx::raw_sql(include_str!("../../migrations/013_earth_editions.sql"))
                .execute(&mut *tx)
                .await?;
        }
        if version < 14 {
            sqlx::raw_sql(include_str!("../../migrations/014_osr.sql"))
                .execute(&mut *tx)
                .await?;
        }
        if version < 15 {
            sqlx::raw_sql(include_str!("../../migrations/015_osr_levels.sql"))
                .execute(&mut *tx)
                .await?;
        }
        if version < 16 {
            sqlx::raw_sql(include_str!("../../migrations/016_warhammer.sql"))
                .execute(&mut *tx)
                .await?;
        }
        if version < CATALOGUE_VERSION {
            sqlx::query("UPDATE catalogue SET version=$1 WHERE singleton=1")
                .bind(CATALOGUE_VERSION)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("INSERT INTO ownership(singleton) VALUES(1) ON CONFLICT DO NOTHING")
            .execute(&mut *tx)
            .await?;
        let owner = Uuid::new_v4();
        let generation = sqlx::query_scalar("UPDATE ownership SET owner=$1,generation=generation+1 WHERE singleton=1 RETURNING generation").bind(owner).fetch_one(&mut *tx).await?;
        let db = Self {
            pool: pool.clone(),
            catalogue: meta.get("id"),
            owner,
            generation,
            lock: Arc::new(Mutex::new(Some(lock))),
            paths,
        };
        db.commit(tx).await?;
        // Acquiring the OS lock proves no previous analysis worker is still alive.
        let mut tx = db.transaction().await?;
        sqlx::query("UPDATE analysis_runs SET state='interrupted',finished_at=?,detail='Application stopped before completion. Completed batches are saved; run analysis again to finish.' WHERE state='running'")
            .bind(Utc::now().to_rfc3339()).execute(&mut *tx).await?;
        sqlx::query("UPDATE classification_attempts SET state='interrupted',detail='Request interrupted before a result was committed; it may have been billed.' WHERE state IN ('in_flight','retry_wait')")
            .execute(&mut *tx).await?;
        db.commit(tx).await?;
        if cancel.is_cancelled() {
            let _ = db.release().await;
            return Err(Error::Cancelled);
        }
        super::archive::Archives::new(&db.paths)
            .cleanup_staging()
            .await?;
        Ok(db)
    }
    fn held(&self) -> Result<()> {
        if self.lock.lock().map_err(|_| Error::Ownership)?.is_some() {
            Ok(())
        } else {
            Err(Error::Ownership)
        }
    }
    pub(super) async fn transaction(&self) -> Result<Transaction<'_, Sqlite>> {
        self.held()?;
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        self.held()?;
        let ok: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM ownership WHERE singleton=1 AND owner=$1 AND generation=$2)").bind(self.owner).bind(self.generation).fetch_one(&mut *tx).await?;
        if !ok {
            return Err(Error::Ownership);
        }
        Ok(tx)
    }
    pub(super) async fn commit(&self, mut tx: Transaction<'_, Sqlite>) -> Result<()> {
        self.held()?;
        let ok: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM ownership WHERE singleton=1 AND owner=$1 AND generation=$2)").bind(self.owner).bind(self.generation).fetch_one(&mut *tx).await?;
        if !ok {
            return Err(Error::Ownership);
        }
        tx.commit().await?;
        Ok(())
    }
    pub async fn verify(&self) -> Result<()> {
        // A read-only ownership check must not reserve SQLite's single writer.
        // Every mutation still checks the fence inside BEGIN IMMEDIATE and at commit.
        self.held()?;
        let ok: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM ownership WHERE singleton=1 AND owner=$1 AND generation=$2)")
            .bind(self.owner).bind(self.generation).fetch_one(&self.pool).await?;
        self.held()?;
        if !ok {
            return Err(Error::Ownership);
        }
        Ok(())
    }
    pub async fn release(&self) -> Result<()> {
        let result = async {
            let mut tx = self.transaction().await?;
            sqlx::query(
                "UPDATE ownership SET owner=NULL WHERE singleton=1 AND owner=$1 AND generation=$2",
            )
            .bind(self.owner)
            .bind(self.generation)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            Ok(())
        }
        .await;
        self.lock.lock().map_err(|_| Error::Ownership)?.take();
        result
    }
    pub async fn new_run(&self, settings: &RuntimeSettings, kind: &str) -> Result<Uuid> {
        let mut tx = self.transaction().await?;
        let run = Uuid::new_v4();
        sqlx::query("UPDATE runs AS r SET state='interrupted',ended_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'),summary=(SELECT json_object('saved_run',count(*) FILTER(WHERE kind='product' AND state='successful'),'failed',count(*) FILTER(WHERE kind='product' AND state='failed'),'unfinished',count(*) FILTER(WHERE state IN ('pending','active','retry','persistence')),'sitemaps_done',count(*) FILTER(WHERE kind='sitemap' AND state='done'),'sitemaps_failed',count(*) FILTER(WHERE kind='sitemap' AND state='failed')) FROM jobs WHERE run=r.id) WHERE r.state IN ('running','authentication paused','storage paused','stopping')").execute(&mut *tx).await?;
        sqlx::query("INSERT INTO runs(id,kind,settings) VALUES($1,$2,$3)")
            .bind(run)
            .bind(kind)
            .bind(json!({"targets":[d::KEY],"runtime":settings}))
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO targets(target) VALUES($1) ON CONFLICT DO NOTHING")
            .bind(d::KEY)
            .execute(&mut *tx)
            .await?;
        if kind == "crawl" {
            sqlx::query("UPDATE jobs SET run=$1,state='pending',attempts=0,owner=NULL,generation=NULL,started=false,outcome=NULL WHERE kind='product' AND state<>'successful'").bind(run).execute(&mut *tx).await?;
            sqlx::query(
                "INSERT INTO jobs(kind,target,identity,url,run) VALUES('sitemap',$1,$2,$2,$3)",
            )
            .bind(d::KEY)
            .bind(d::ROOT)
            .bind(run)
            .execute(&mut *tx)
            .await?;
        }
        self.commit(tx).await?;
        Ok(run)
    }
    pub async fn run_state(&self, run: Uuid, state: &str, ended: bool) -> Result<()> {
        let summary = if ended {
            serde_json::to_value(self.metrics(run).await?)?
        } else {
            Value::Null
        };
        let mut tx = self.transaction().await?;
        sqlx::query("UPDATE runs SET state=$2,ended_at=CASE WHEN $3 THEN strftime('%Y-%m-%dT%H:%M:%fZ','now') ELSE NULL END,summary=CASE WHEN $3 THEN $4 ELSE summary END WHERE id=$1").bind(run).bind(state).bind(ended).bind(summary).execute(&mut *tx).await?;
        self.commit(tx).await
    }
    pub async fn discover(&self, run: Uuid, source: &str, entries: Vec<Entry>) -> Result<()> {
        if entries.len() > 1000 {
            return Err(Error::Config(
                "Discovery batch exceeds 1,000 entries".into(),
            ));
        }
        let mut maps = std::collections::BTreeSet::new();
        let mut products = std::collections::BTreeMap::new();
        let mut provenance = std::collections::BTreeMap::new();
        for entry in entries {
            if entry.sitemap {
                if let Some(url) = d::sitemap_url(&entry.location) {
                    maps.insert(url);
                }
                continue;
            }
            if let Some(id) = d::product_url(&entry.location) {
                products.insert(id.as_str().to_owned(), d::canonical(id.as_str()));
                provenance.insert((id.as_str().to_owned(), entry.location), entry.lastmod);
            }
        }
        let mut tx = self.transaction().await?;
        for url in maps {
            sqlx::query("INSERT INTO jobs(kind,target,identity,url,run,parent) VALUES('sitemap',$1,$2,$2,$3,$4) ON CONFLICT DO NOTHING")
                .bind(d::KEY).bind(url).bind(run).bind(source).execute(&mut *tx).await?;
        }
        for (id, url) in products {
            sqlx::query("INSERT INTO jobs(kind,target,identity,url,run,discovered_run) VALUES('product',$1,$2,$3,$4,$4) ON CONFLICT(target,identity) WHERE kind='product' DO UPDATE SET last_seen=strftime('%Y-%m-%dT%H:%M:%fZ','now')")
                .bind(d::KEY).bind(id).bind(url).bind(run).execute(&mut *tx).await?;
        }
        for ((id, alias), hint) in provenance {
            sqlx::query("INSERT INTO discoveries(job,source,alias,lastmod,run) SELECT id,$1,$2,$3,$4 FROM jobs WHERE target=$5 AND identity=$6 AND kind='product' ON CONFLICT(job,source,alias) DO UPDATE SET last_seen=strftime('%Y-%m-%dT%H:%M:%fZ','now'),lastmod=excluded.lastmod,run=excluded.run")
                .bind(source).bind(alias).bind(hint).bind(run).bind(d::KEY).bind(id).execute(&mut *tx).await?;
        }
        self.commit(tx).await
    }
    pub async fn claim(&self, run: Uuid, limit: i64, max_attempts: i32) -> Result<Vec<Claim>> {
        let mut tx = self.transaction().await?;
        sqlx::query("UPDATE jobs SET state='failed',outcome=COALESCE(outcome,'{\"kind\":\"interrupted\",\"reason\":\"Attempt budget exhausted\"}') WHERE run=$1 AND state IN ('pending','retry') AND attempts>=$2").bind(run).bind(max_attempts).execute(&mut *tx).await?;
        // Avoid scanning a cooled-down target's backlog, and reserve at most one
        // discovery slot so website pacing cannot occupy all product workers.
        let ready: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM targets WHERE cooldown_until<=strftime('%Y-%m-%dT%H:%M:%fZ','now'))")
            .fetch_one(&mut *tx).await?;
        if !ready {
            self.commit(tx).await?;
            return Ok(Vec::new());
        }
        let rows=sqlx::query(r#"
WITH sitemap AS MATERIALIZED (
 SELECT j.id FROM jobs j INDEXED BY jobs_ready
 WHERE j.run=$1 AND j.kind='sitemap' AND j.state IN ('pending','retry')
 AND j.due_at<=strftime('%Y-%m-%dT%H:%M:%fZ','now') AND j.attempts<$5
 AND NOT EXISTS(SELECT 1 FROM jobs WHERE run=$1 AND kind='sitemap' AND state='active')
 AND EXISTS(SELECT 1 FROM targets t WHERE t.target=j.target AND t.cooldown_until<=strftime('%Y-%m-%dT%H:%M:%fZ','now'))
 ORDER BY j.due_at,j.id LIMIT 1
), products AS MATERIALIZED (
 SELECT j.id FROM jobs j INDEXED BY jobs_ready
 WHERE j.run=$1 AND j.kind='product' AND j.state IN ('pending','retry')
 AND j.due_at<=strftime('%Y-%m-%dT%H:%M:%fZ','now') AND j.attempts<$5
 AND EXISTS(SELECT 1 FROM targets t WHERE t.target=j.target AND t.cooldown_until<=strftime('%Y-%m-%dT%H:%M:%fZ','now'))
 AND NOT EXISTS(SELECT 1 FROM intents i WHERE i.target=j.target AND i.product_id=j.identity AND NOT i.finalized)
 ORDER BY j.due_at,j.id LIMIT $2-(SELECT count(*) FROM sitemap)
), picked AS (SELECT id FROM sitemap UNION ALL SELECT id FROM products)
UPDATE jobs AS j SET state='active',owner=$3,generation=$4,started=false
FROM picked WHERE j.id=picked.id RETURNING *
"#)
            .bind(run).bind(limit.clamp(1,1000)).bind(self.owner).bind(self.generation).bind(max_attempts).fetch_all(&mut *tx).await?;
        self.commit(tx).await?;
        Ok(rows
            .into_iter()
            .map(|r| Claim {
                id: r.get("id"),
                target: r.get("target"),
                identity: r.get("identity"),
                url: r.get("url"),
                kind: r.get("kind"),
                attempts: r.get("attempts"),
                run,
            })
            .collect())
    }
    pub async fn begin_attempt(&self, c: &Claim) -> Result<()> {
        let mut tx = self.transaction().await?;
        let changed=sqlx::query("UPDATE jobs SET attempts=attempts+1,lifetime_attempts=lifetime_attempts+1,started=true,checked_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=$1 AND state='active' AND owner=$2 AND generation=$3 AND NOT started").bind(c.id).bind(self.owner).bind(self.generation).execute(&mut *tx).await?.rows_affected();
        if changed != 1 {
            return Err(Error::Ownership);
        }
        sqlx::query("INSERT INTO attempts(job,run,stage,outcome,parser_version) VALUES($1,$2,'request',$3,$4)").bind(c.id).bind(c.run).bind(json!({"status":"started","attempt":c.attempts+1})).bind(PARSER_VERSION).execute(&mut *tx).await?;
        self.commit(tx).await
    }
    pub async fn cooldown(&self, target: &str, seconds: u64) -> Result<()> {
        let mut tx = self.transaction().await?;
        sqlx::query("UPDATE targets SET cooldown_until=max(cooldown_until,strftime('%Y-%m-%dT%H:%M:%fZ','now',printf('+%d seconds',$2))) WHERE target=$1").bind(target).bind(seconds.min(31_536_000) as i64).execute(&mut *tx).await?;
        self.commit(tx).await
    }
    pub async fn remaining_cooldown(&self, target: &str) -> Result<u64> {
        let seconds:Option<f64>=sqlx::query_scalar("SELECT max(0.0,(julianday(cooldown_until)-julianday('now'))*86400.0) FROM targets WHERE target=$1").bind(target).fetch_optional(&self.pool).await?;
        Ok(seconds.unwrap_or(0.0).ceil() as u64)
    }
    pub async fn outcome(&self, c: &Claim, f: Option<&Failure>, max: i32) -> Result<()> {
        let mut tx = self.transaction().await?;
        let retry = f.is_some_and(|f| f.kind == FailureKind::Transient) && c.attempts + 1 < max;
        let state = if f.is_none() {
            "done"
        } else if f.is_some_and(Failure::pauses) {
            "pending"
        } else if retry {
            "retry"
        } else {
            "failed"
        };
        let seconds = f
            .map(|f| {
                f.retry_after_secs.max(if retry {
                    10u64
                        .saturating_mul(2u64.saturating_pow(c.attempts as u32))
                        .min(3600)
                        + (c.id as u64 % 3)
                } else {
                    0
                })
            })
            .unwrap_or(0)
            .min(i64::MAX as u64) as i64;
        let outcome = serde_json::to_value(f)?;
        sqlx::query("UPDATE jobs SET state=$2,outcome=$3,due_at=max(due_at,strftime('%Y-%m-%dT%H:%M:%fZ','now',printf('+%d seconds',$4))),owner=NULL,generation=NULL WHERE id=$1 AND state<>'successful'").bind(c.id).bind(state).bind(&outcome).bind(seconds).execute(&mut *tx).await?;
        if seconds > 0 && f.is_some_and(|f| f.retry_after_secs > 0) {
            sqlx::query("UPDATE targets SET cooldown_until=max(cooldown_until,strftime('%Y-%m-%dT%H:%M:%fZ','now',printf('+%d seconds',$2))) WHERE target=$1").bind(&c.target).bind(seconds).execute(&mut *tx).await?;
        }
        sqlx::query("INSERT INTO attempts(job,run,stage,outcome,parser_version) VALUES($1,$2,'outcome',$3,$4)").bind(c.id).bind(c.run).bind(&outcome).bind(PARSER_VERSION).execute(&mut *tx).await?;
        self.commit(tx).await
    }
    pub async fn release_claims(&self, run: Uuid) -> Result<()> {
        let mut tx = self.transaction().await?;
        sqlx::query("UPDATE jobs SET state='pending',owner=NULL,generation=NULL WHERE run=$1 AND state='active'").bind(run).execute(&mut *tx).await?;
        self.commit(tx).await
    }
    pub async fn intent(&self, i: &Intent) -> Result<()> {
        let mut tx = self.transaction().await?;
        sqlx::query("INSERT INTO intents(id,target,product_id,payload,generation) VALUES($1,$2,$3,$4,$5) ON CONFLICT(id) DO NOTHING").bind(i.id).bind(&i.target).bind(&i.product_id).bind(serde_json::to_value(i)?).bind(self.generation).execute(&mut *tx).await?;
        sqlx::query("UPDATE jobs SET state='persistence' WHERE kind='product' AND target=$1 AND identity=$2 AND state<>'successful'").bind(&i.target).bind(&i.product_id).execute(&mut *tx).await?;
        self.commit(tx).await
    }
    pub async fn pending_intents(&self, after: Uuid) -> Result<Vec<Intent>> {
        let rows = sqlx::query_scalar::<_, Value>(
            "SELECT payload FROM intents WHERE NOT finalized AND id>$1 ORDER BY id LIMIT 100",
        )
        .bind(after)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|v| serde_json::from_value(v).map_err(Error::from))
            .collect()
    }
    pub async fn retire_completed_intent(&self, i: &Intent) -> Result<bool> {
        let mut tx = self.transaction().await?;
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM products WHERE target=$1 AND product_id=$2)",
        )
        .bind(&i.target)
        .bind(&i.product_id)
        .fetch_one(&mut *tx)
        .await?;
        if exists {
            sqlx::query("UPDATE intents SET finalized=true WHERE id=$1")
                .bind(i.id)
                .execute(&mut *tx)
                .await?;
        }
        self.commit(tx).await?;
        Ok(exists)
    }
    pub async fn outdated_count(&self) -> Result<i64> {
        Ok(
            sqlx::query_scalar("SELECT count(*) FROM products WHERE parser_version<>$1")
                .bind(PARSER_VERSION)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn absent_intent(&self, i: &Intent) -> Result<()> {
        let mut tx = self.transaction().await?;
        sqlx::query("DELETE FROM intents WHERE id=$1 AND NOT finalized")
            .bind(i.id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE jobs SET state='pending',owner=NULL,generation=NULL WHERE kind='product' AND target=$1 AND identity=$2 AND state<>'successful' AND NOT EXISTS(SELECT 1 FROM products WHERE target=$1 AND product_id=$2)").bind(&i.target).bind(&i.product_id).execute(&mut *tx).await?;
        self.commit(tx).await
    }
    pub async fn commit_captures(&self, captures: Vec<(Intent, Product)>) -> Result<usize> {
        self.held()?;
        // Filesystem verification must finish before opening the SQL transaction.
        let archives = super::archive::Archives::new(&self.paths);
        for (intent, _) in &captures {
            if archives.read(&intent.archive).await?.is_none() {
                return Err(Error::Integrity);
            }
        }
        let mut tx = self.transaction().await?;
        let mut saved = 0;
        for (i, mut p) in captures {
            p.validate()?;
            if p.target_key != i.target || p.product_id.as_str() != i.product_id {
                return Err(Error::Integrity);
            }
            p.fields.insert("fetched_at".into(), json!(i.fetched_at));
            p.fields
                .insert("last_checked_at".into(), json!(i.fetched_at));
            p.fields.insert("reparsed_at".into(), Value::Null);
            p.fields.insert("http".into(), i.http.clone());
            p.fields.insert("archive".into(), json!(i.archive));
            p.fields.insert(
                "raw_path".into(),
                json!(format!("archives/{}", i.archive.key)),
            );
            let record = serde_json::to_value(&p)?;
            saved+=sqlx::query("INSERT INTO products(target,product_id,title,description_text,publisher,price,currency,page_count,language,record,parser_version,record_schema_version,archive,fetched_at,last_checked_at,intent) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$14,$15) ON CONFLICT(target,product_id) DO NOTHING")
                .bind(&i.target).bind(&i.product_id).bind(&p.title).bind(&p.description_text).bind(record["publisher"].as_str()).bind(record["price"].as_str()).bind(record["currency"].as_str()).bind(record["page_count"].as_i64()).bind(record["language"].as_str()).bind(&record).bind(PARSER_VERSION).bind(SCHEMA_VERSION).bind(json!(i.archive)).bind(i.fetched_at).bind(i.id).execute(&mut *tx).await?.rows_affected() as usize;
            sqlx::query("UPDATE intents SET finalized=true WHERE id=$1")
                .bind(i.id)
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE jobs SET state='successful',outcome=NULL,owner=NULL,generation=NULL WHERE kind='product' AND target=$1 AND identity=$2").bind(&i.target).bind(&i.product_id).execute(&mut *tx).await?;
        }
        self.commit(tx).await?;
        Ok(saved)
    }
    pub async fn metrics(&self, run: Uuid) -> Result<Metrics> {
        // Separate indexed counts avoid evaluating every filter on every wide
        // job row. Earliest due times seek into each target/kind queue index.
        let r=sqlx::query(r#"SELECT
 (SELECT count(*) FROM jobs WHERE kind='product' AND state='successful') successful,
 (SELECT count(*) FROM jobs WHERE run=$1 AND kind='product' AND state='successful') saved_run,
 (SELECT count(*) FROM jobs WHERE run=$1 AND kind='product' AND state='pending') pending,
 (SELECT count(*) FROM jobs WHERE run=$1 AND kind='product' AND state='active') active,
 (SELECT count(*) FROM jobs WHERE run=$1 AND kind='product' AND state='retry') retry,
 (SELECT count(*) FROM jobs WHERE run=$1 AND kind='product' AND state='failed') failed,
 (SELECT count(*) FROM jobs WHERE kind='product' AND state='persistence') persistence,
 (SELECT count(*) FROM jobs WHERE run=$1 AND kind='sitemap' AND state IN ('pending','retry','active')) sp,
 (SELECT count(*) FROM jobs WHERE run=$1 AND kind='sitemap' AND state='retry') sr,
 (SELECT count(*) FROM jobs WHERE run=$1 AND kind='sitemap' AND state='active') sa,
 (SELECT count(*) FROM jobs WHERE run=$1 AND kind='sitemap' AND state='done') sd,
 (SELECT count(*) FROM jobs WHERE run=$1 AND kind='sitemap' AND state='failed') sf,
 (SELECT count(*) FROM jobs WHERE kind='product' AND discovered_run=$1) discovered,
 (SELECT min(max(t.cooldown_until, (
    SELECT j.due_at FROM jobs j INDEXED BY jobs_ready
    WHERE j.run=$1 AND j.kind=k.kind AND j.target=t.target AND j.state IN ('pending','retry')
    ORDER BY j.due_at,j.id LIMIT 1
  ))) FROM targets t CROSS JOIN (SELECT 'product' kind UNION ALL SELECT 'sitemap') k) due
"#).bind(run).fetch_one(&self.pool).await?;
        let last_saved =
            sqlx::query_scalar::<_, Option<DateTime<Utc>>>("SELECT max(saved_at) FROM products")
                .fetch_one(&self.pool)
                .await?;
        let seen = sqlx::query_scalar::<_, i64>(
            "SELECT count(DISTINCT job) FROM discoveries WHERE run=$1",
        )
        .bind(run)
        .fetch_one(&self.pool)
        .await?;
        Ok(Metrics {
            successful: r.get("successful"),
            last_saved,
            saved_run: r.get("saved_run"),
            pending: r.get("pending"),
            active: r.get("active"),
            retry: r.get("retry"),
            failed: r.get("failed"),
            persistence: r.get("persistence"),
            sitemaps_pending: r.get("sp"),
            sitemaps_retry: r.get("sr"),
            sitemaps_active: r.get("sa"),
            sitemaps_done: r.get("sd"),
            sitemaps_failed: r.get("sf"),
            seen,
            discovered: r.get("discovered"),
            next_due: r.get("due"),
        })
    }
    pub async fn failures(&self, run: Uuid, after: i64) -> Result<Vec<String>> {
        Ok(self.failure_page(run, after).await?.0)
    }
    pub async fn failure_page(&self, run: Uuid, after: i64) -> Result<(Vec<String>, Option<i64>)> {
        let kind: Option<String> = sqlx::query_scalar("SELECT kind FROM runs WHERE id=$1")
            .bind(run)
            .fetch_optional(&self.pool)
            .await?;
        let rows = if kind.as_deref() == Some("reprocess") {
            sqlx::query("SELECT id,product_id AS identity,outcome,at AS checked_at FROM reprocess_outcomes WHERE run=$1 AND outcome<>'null' AND id>$2 ORDER BY id LIMIT 100").bind(run).bind(after).fetch_all(&self.pool).await?
        } else {
            sqlx::query("SELECT id,identity,outcome,checked_at FROM jobs WHERE run=$1 AND state='failed' AND id>$2 ORDER BY id LIMIT 100").bind(run).bind(after).fetch_all(&self.pool).await?
        };
        let next = if rows.len() == 100 {
            rows.last().map(|r| r.get::<i64, _>("id"))
        } else {
            None
        };
        Ok((
            rows.iter()
                .map(|r| {
                    format!(
                        "#{} {} · {} · {}",
                        r.get::<i64, _>("id"),
                        crate::telemetry::identity(&r.get::<String, _>("identity")),
                        r.get::<Value, _>("outcome"),
                        r.get::<Option<DateTime<Utc>>, _>("checked_at")
                            .map(|d| d.to_rfc3339())
                            .unwrap_or_default()
                    )
                })
                .collect(),
            next,
        ))
    }
    pub async fn known_product(&self) -> Result<Option<String>> {
        Ok(
            sqlx::query_scalar(
                "SELECT identity FROM jobs WHERE kind='product' ORDER BY id LIMIT 1",
            )
            .fetch_optional(&self.pool)
            .await?,
        )
    }
    pub async fn reprocess_page(
        &self,
        after: &str,
    ) -> Result<Vec<(String, String, Value, Archive)>> {
        let rows=sqlx::query("SELECT target,product_id,record,archive FROM products WHERE target || '/' || product_id>$1 AND parser_version<>$2 ORDER BY target,product_id LIMIT 100").bind(after).bind(PARSER_VERSION).fetch_all(&self.pool).await?;
        rows.into_iter()
            .map(|r| {
                Ok((
                    r.get("target"),
                    r.get("product_id"),
                    r.get("record"),
                    serde_json::from_value(r.get("archive"))?,
                ))
            })
            .collect()
    }
    pub async fn reprocess_commit(
        &self,
        run: Uuid,
        target: &str,
        id: &str,
        old: &Value,
        result: std::result::Result<Product, Failure>,
    ) -> Result<()> {
        let mut tx = self.transaction().await?;
        let mut failure = Value::Null;
        match result {
            Ok(mut p) => {
                p.validate()?;
                if p.target_key != target || p.product_id.as_str() != id {
                    return Err(Error::Integrity);
                }
                for k in [
                    "fetched_at",
                    "last_checked_at",
                    "archive",
                    "raw_path",
                    "discovery",
                    "crawl_status",
                    "http",
                ] {
                    if let Some(v) = old.get(k) {
                        p.fields.insert(k.into(), v.clone());
                    }
                }
                p.fields.insert("reparsed_at".into(), json!(Utc::now()));
                let record = serde_json::to_value(&p)?;
                sqlx::query("UPDATE products SET title=$3,description_text=$4,record=$5,parser_version=$6,record_schema_version=$7,reparsed_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'),publisher=$8,price=$9,currency=$10,page_count=$11,language=$12 WHERE target=$1 AND product_id=$2")
                .bind(target).bind(id).bind(&p.title).bind(&p.description_text).bind(&record).bind(PARSER_VERSION).bind(SCHEMA_VERSION).bind(record["publisher"].as_str()).bind(record["price"].as_str()).bind(record["currency"].as_str()).bind(record["page_count"].as_i64()).bind(record["language"].as_str()).execute(&mut *tx).await?;
            }
            Err(f) => {
                failure = json!(f);
            }
        }
        sqlx::query("INSERT INTO reprocess_outcomes(run,target,product_id,outcome) VALUES($1,$2,$3,$4) ON CONFLICT(run,target,product_id) DO UPDATE SET outcome=EXCLUDED.outcome,at=strftime('%Y-%m-%dT%H:%M:%fZ','now')").bind(run).bind(target).bind(id).bind(&failure).execute(&mut *tx).await?;
        sqlx::query("UPDATE runs SET checkpoint=$2,processed=(SELECT count(*) FROM reprocess_outcomes WHERE run=$1),failures=(SELECT count(*) FROM reprocess_outcomes WHERE run=$1 AND outcome<>'null') WHERE id=$1").bind(run).bind(format!("{target}/{id}")).execute(&mut *tx).await?;
        self.commit(tx).await
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn releasing_owner_unlocks_even_with_a_temporarily_inherited_descriptor() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("catalogue.lock");
        let file = crate::paths::private_file(&path).unwrap();
        file.try_lock().unwrap();
        let inherited = file.try_clone().unwrap();
        let owner = WriterLock(file);
        let competitor = crate::paths::private_file(&path).unwrap();
        assert!(matches!(
            competitor.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
        drop(owner);
        competitor.try_lock().unwrap();
        drop(inherited);
    }
}
