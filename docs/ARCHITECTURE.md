# Implementation architecture

The [specification](../SPEC.md) is the product contract. This is one Rust crate, pinned to Rust 1.99.0 (also the MSRV) with `Cargo.lock`. Catalogue schema 16, product record schema 2 and parser version `drivethru-rust-1` are independent boundaries.

## Boundaries

| Module | Responsibility |
| --- | --- |
| `desktop` | Native Tauri window, narrow IPC commands, navigation restrictions and graceful close |
| `controller`, `app` | Serialized operations, secret-free snapshots, asynchronous run control and shutdown |
| `ui/` | Bundled TypeScript views, guided authentication, masked reviews and keyboard navigation |
| `paths`, `config` | Fixed home-directory paths, atomic settings/secrets files and migration of applicable old settings/cookies |
| `domain` | Identity, captures, archive references, claims, failures, metrics and resource limits |
| `scheduler` | Bounded work, batching, pause/resume, shutdown, intent recovery and reprocessing |
| `targets::drivethru` | Host/URL rules, site/group IDs, taxonomy/media constants and access classification |
| `targets::sitemap` | Bounded XML/gzip parsing and discovery batches |
| `http`, `auth` | Reused HTTP client, per-target pacing, robots, redirects, scoped cookies and browser import |
| `parser` | Pure extraction and isolated-process protocol |
| `storage::sqlite` | SQL, migrations, ownership, durable queue, records and attempts |
| `storage::archive` | Immutable local gzip files, atomic publication, hash verification and storage checks |
| `analysis`, `controller::analysis`, `storage::analysis`, `ui/analysis.ts` | Pure weighted-score policy, cancellable run orchestration, fenced batch persistence and explicit Analysis workspace |
| `decisions` | Fixed OpenAI endpoint/model, environment-only credential, bounded text/JSON requests, typed and validated answers, cancellation and usage; no catalogue access |

SQLx uses bundled SQLite for the catalogue; rusqlite separately reads browser profiles. Neither exposes network storage. The desktop controller and scheduler use domain storage operations rather than assembling SQL. Storefront constants remain in the adapter.

## OpenAI Decisions

The Decisions adapter is reused lazily by `Controller::test_decisions`. The connection-test IPC action constructs a fixed synthetic request in Rust, holds the controller operation gate and observes its closing token. Shutdown cancels the request and waits for that gate. The browser cannot choose endpoints, supply questions or receive keys/provider error bodies. The separate explicit classification command selects evidence from local storage in Rust. The adapter has no storage dependencies. The common request encoder disables prompt caching with explicit mode and no marked input boundaries, covering classification, retries and diagnostics without changing evidence/questions or saved-result versions. Provider usage still passes through unchanged. Refusals remain separate from None. `classification` defines the versioned single-primary-role policy; `storage::classification` owns evidence projection, checkpoints, result/group writes and provenance. Migration 006 adds these structures without automatic API work.

## Fixed storage and startup

`LocalPaths::current()` resolves the OS home directory and appends `.kaijucrawl`. There is no user-facing override. Tests inject disposable paths through library operations; the application does not read them from settings, environment variables or CLI options.

The folder contains `settings.toml`, `secrets.json`, `catalogue.sqlite3`, SQLite sidecars, `catalogue.lock` and `archives/`. Settings/secrets use flushed same-directory temporary files and atomic replacement. Unix directories/files use owner-only permissions; Windows files inherit the profile ACL. Remembered cookies are plaintext JSON; session-only values stay in memory. The old keyring is read once as a compatibility source when the local secrets file is absent, never as a destination. Old cloud credentials are discarded. Existing remote data is not contacted or imported.

User settings contain target/browser choices only. `RuntimeSettings` derives download and parser capacity from `std::thread::available_parallelism()` with fallback one, plus fixed internal bounds. `Config.runtime` is skipped during settings serialization/deserialization, so old `[advanced]` values and attempted `[runtime]` overrides are ignored and disappear on the next save. New run snapshots store the actual policy under `runtime`; historical snapshots are unchanged.

Startup checks storage in a background task, loads totals and enables Start. Checks acquire ownership, initialize/verify the schema, and perform a temporary local write/read/flush/delete check. They release ownership and retain the pool for read-only dashboard queries. Failure permits a retry without a wizard. The Local storage view displays fixed paths and permits a recheck. Opening the application never starts a crawl.

## Ownership and SQLite

An exclusive OS lock on `catalogue.lock` is held throughout mutating work. Competing handles fail promptly; process death releases ownership without a lease timeout. The lock file is never deleted to force takeover. Each acquisition increments the persisted generation and installs a random owner token.

Mutations use `BEGIN IMMEDIATE`, checking the held lock and SQLite owner/generation before mutation and commit. Release invalidates clones. Ownership is periodically checked while running; it does not expire on a timer. Heartbeats and pre-request verification read the persisted fence and check the OS lock before/after the read, without acquiring a writer reservation. Transactions do not span HTTP, parsing or archive-file operations. Read-only pools may remain after release.

The pool has four connections, a ten-second acquisition limit and a five-second SQLite busy timeout. WAL, FULL synchronous commits and foreign keys are enabled. Versioned initialization preserves compatible data and refuses unknown/newer schemas. The former PostgreSQL schema is not an input to local migration.

Migration 003 adds partial indexes for ready jobs, exhausted attempt budgets and pending product intents, plus covering indexes for run counts, discoveries and last-save time. Version 2 upgrades transactionally under the OS ownership lock without altering existing records. Queue selection seeks due sitemap/product work separately and materializes only a bounded set of IDs before updating claims. One active sitemap slot prevents discovery starvation without filling every worker with website pacing waits. Ready jobs are ordered by due time and ID.

Metrics use indexed counts and earliest-due seeks; they do not scan product metadata or evaluate every state predicate over every job row. The scheduler drains already completed tasks up to its save-batch bound before another refill cycle. Final archive verification and fenced metadata commits remain synchronous with success publication, and staged work remains bounded by worker and batch capacity.

Product uniqueness is `(target, identity)`; sitemap uniqueness includes the run. Bounded batches use parameterized inserts/upserts; recovery/maintenance use keyset pagination. Provenance retains aliases and sightings. `catalogue_records` combines products with discovery/crawl state. Prices use exact decimal TEXT, not floating point or SQLite NUMERIC affinity; numeric calculations need explicit decimal handling. Nested/raw metadata remains complete JSON text.

## Retiring Search

Migration 010 removes the derived Search projection, facets, FTS index/shadow tables, backfill checkpoint and maintenance triggers within the owned bootstrap transaction. Historical migrations remain for schema compatibility, but no Search backfill runs. Canonical products/rowids, archives, crawl state, analysis results, memberships and history are retained. The legacy exact-decimal collation remains private to storage so older schemas can open and be retired; no search-query implementation remains.

The desktop shell exposes Crawlers, Discovery, Dungeons & Dragons, OSR, Shadowrun, Call of Cthulhu, Warhammer Fantasy and Warhammer 40k in a flyout main menu. `ui/app.ts` owns menu dismissal, focus containment/restoration and workspace/tab selection while retaining mounted workspace state. `ui/workspaces.ts` defines stage labels and local SVG icons; `ui/themes.css` applies whole-shell themes from the selected workspace. `ui/discovery.ts` renders the shared dashboard with four Discovery, six D&D, two OSR or three Shadowrun/CoC/Warhammer coverage cards and invokes one backend sequence command; it does not schedule individual stages. The old `createAnalysis` dropdown component remains intact but is not mounted; system implementations are reused by the dashboards. Search components, detail/filter views and IPC commands remain removed. Analyses read canonical `products` using rowid keysets; historical document IDs and checkpoints remain compatible. Historical query JSON is opaque and never executed.

`controller::discovery` acquires catalogue ownership once and reuses `storage::analysis` batches plus the existing online coordinator for relevance → primary roles → solo play → Adventure players. The same sequencer also serves six D&D stages and three stages per Earth system. All stages retain independent durable run/checkpoint/usage records. `AnalysisWorkspace` selects the fixed plan; transient `Status.discovery.workspace` defaults to Discovery for compatibility. Both system-sequence and individual-system entrypoints check current Discovery coverage, then repeat the check through `Catalogue::require_discovery_complete` in an owned transaction before paid work. Source invalidation removes stale results, naturally relocking the gate; empty eligible sets and saved unknowns are complete. Optional transient `Status.discovery` metadata supplies step, rerun mode and unsuccessful counts; it is not persisted and defaults absent for older histories. Status stays active across stage transitions until ownership release. Cancel/Close uses the existing analysis task token and join/drain path, so no later stage starts after cancellation. Default relevance records fixed `all_missing` scope and skips current score rows, including null scores, using bounded rowid selection. Existing individual relevance starts retain full-rerun behavior; force rerun passes through the existing online options. There is no schema change or automatic restart scheduling.

The shared dashboard UI consumes committed overview snapshots, animates real progress changes, and shows sequence progress within its single summary area. It reads bounded cached aggregates only for visible idle loading/reconciliation, coalesces requests, and rejects stale reads after live totals. There are no per-frame SQL reads or paid navigation actions. System stages never run as part of Discovery.

`connect_cancel` observes the closing token before opening and after atomic migration/recovery work. Startup shows loading until canonical totals are read, then publishes the saved totals and ready state. Close waits for executing database work, releases ownership and exits; it does not wait for an index backfill. Storage opening and archive validation remain asynchronous.

## Explicit analysis

Migration 005 creates `analysis_runs`, `relevance_scores` and `analysis_revision`, plus a unique running-analysis guard and input-invalidation triggers. It leaves products, archives and observation times intact. Startup creates empty analysis tables and loads the latest run; it never computes a score. A new owner marks abandoned running analyses interrupted, retaining every committed result.

`analysis::relevance` implements the fixed, versioned `(rating × count + 60) / (count + 20)` policy and returns rounded integer millionths. Storage reads only rating/count fields from canonical JSON, parses numeric strings strictly, and records unusable inputs as an analyzed null score. Results are keyed by target/product, with version, inputs, run and calculation time. Changes to either rating input delete the prior result; unrelated reprocessing preserves it. Score removals and batch updates increment a revision inside the same transaction.

`controller::analysis` starts only on an explicit IPC command, obtains its own catalogue ownership, and blocks other mutating operations until release. Its cancellation token is a child of the desktop closing token. A separate watch status publishes committed progress without changing crawl counters. Each fenced transaction selects at most 500 records directly from products using a rowid cursor, computes scores and upserts them alongside run counters and revision. Weighted relevance and primary roles use the whole catalogue; player counts use a fixed current-Adventure subset. Each freezes an upper product rowid. Each algorithm implements fixed eligibility; no user query is accepted by the start commands. No task/channel is created per book, no HTTP/archive operations occur, and no full catalogue is retained in memory.

Cancellation is checked between batches. Closing awaits the task, which completes any executing SQL, records cancellation when possible and releases ownership. The UI offers explicit rerun of the catalogue after cancellation/interruption; there is no automatic resumption of analysis. Primary-role classification runs at logical-CPU capacity outside transactions, using a bounded JoinSet and a shared client admission semaphore. One coordinator owns all SQLite writes and mutable counters. It drains ready results before replenishing slots; even completed tasks occupy capacity until saved. The admission keyset cursor is separate from the monotonic completion high-water mark, so new explicit runs can recover unfinished earlier IDs by selecting books without current results. Duplicate commits are rejected. A fenced dispatch precedes HTTP; a fenced commit saves result, membership, attempt, usage and progress together. Current-version results are skipped unless explicit reclassification is selected. Relevant input changes invalidate memberships; prices/ratings/raw-body changes do not. In-flight dispatches become interrupted on takeover and may have been billed. Cancel/Close and local preflight/storage/ownership failures stop admission, cancel HTTP and drain every task; already-received results are persisted unless storage failed. Worker capacity is recorded in backward-compatible progress JSON; live activity reloads as zero. All API-request failures (every non-200 HTTP status, transport disconnects, timeouts and invalid/oversized responses) instead enter a bounded retry queue, retaining the same evidence and one slot per pending book. Per-book failure counts generate exponential delay (2 seconds to a 300-second base cap plus jitter), extended by Retry-After seconds/date. A shared monotonic cooldown prevents new and retry dispatches until the longest delay expires, without cancelling successful siblings. It uses remaining durations and bounded timer ticks so huge server hints cannot overflow an Instant or hide cancellation. Retry-wait state and each redispatch are fenced; dispatch/retry counts are durable, while waiting counts, countdown and the optional value-free failure reason are live only. HTTP status is optional so network/response failures have a meaningful wait display without inventing a status code. No attempt budget ends these retries. Missing/invalid local credentials, invalid local requests and cancellation never trigger replay. Refusals/oversized evidence remain per-book unsuccessful outcomes. Error bodies are never inspected or retained. Timed-out or invalid responses may be billed; recorded usage includes validated responses only.

## Recoverable archive protocol

1. Validate identity, title and full description before creating an intent.
2. Commit the intent, with a deterministic relative archive key and expected hash/lengths.
3. Write gzip bytes to `archives/.staging`, flush and publish without overwriting existing archives. Keys are `catalogue_uuid/target/product_id/sha256.json.gz`. Existing files are verified for idempotency; corruption is an error. Parent directories are synchronized on Unix. Paths reject traversal and symlinked archive components.
4. Verify decoded SHA-256 and lengths before opening the SQLite transaction. Commit the product, finalized intent and successful job together under ownership, then publish success counters.

SQLite and files do not share an atomic transaction. Pending intents with a valid archive are reparsed/finalized without network access. Missing archives restore eligibility only for never-successful work; corruption pauses recovery. A stale intent cannot downgrade a completed product. Failed live bodies never enter the protocol.

Each new exclusive owner streams the flat staging directory and removes only recognized regular temporary files before work begins. It never scans/deletes committed archive trees or follows staging symlinks. Normal completion/failure removes temporary files too. Archive work is bounded by active workers and the product-size limit.

## Network, parsing and shutdown

The adapter owns the fixed Firefox 158 request profile. The reused client sends it on website, sitemap, robots, product and validated redirects. `RuntimeSettings.robots_agent` separately retains `KaijuCrawl/{version}` for permissions and origin pacing. Neither identity is configurable. Run snapshots record both, without altering historical data. This owner-approved workaround changes neither cookie/failure semantics nor parser/schema versions.

Download/parser concurrency follows the available logical CPU count; claim/save batches remain bounded. There is no added request interval. Each origin has a separate robots interval and last request-start time. A request waits for its origin first, then any target-wide Retry-After cooldown, so website robots waiting does not delay an otherwise eligible API request. Only request starts on the same origin are serialized; downloads may overlap. Policy refresh replaces the affected origin's interval, while retained start times also pace robots refresh requests. Targets sharing a host still have independent limits. SQLite persists target cooldowns. Attempts begin only immediately before requests start.

The scheduler publishes website/API intervals from the live network state, plus the automatic worker count and remaining cooldown. The dashboard labels robots delays and unloaded policies. These status fields are transient and require no catalogue schema change.

Authentication failures suspend the target; storage failures suspend new network work until recovery. Successful products are never refetched normally. New runs rediscover sitemaps and retry never-successful products; Resume retains the current run's attempt budget.

The adapter classifies product API HTTP 403 in its request context: ordinary forbidden/access-denied/disabled/age-restricted responses become `Restricted` outcomes with status 403 and static, value-free reasons. Only explicit login/session/challenge evidence pauses the target. Failed bodies never reach parsing or archive persistence. These outcomes do not retry within the run, but the next run restores eligibility; required robots and website checks retain strict handling. No product extraction or database schema change is needed.

Parser workers use a private entry point in the same executable. They receive bounded response bytes and identity via pipes, with a cleared environment except required Windows process settings. No cookies/secrets are passed. Deadline/cancellation kills and reaps workers. Sitemap parsing uses cooperative cancellation in bounded blocking work; it does not claim hard thread termination.

Stop prevents new requests, drains for five seconds, then cancels remaining work. An executing filesystem operation cannot be forcibly interrupted; hung OS disk I/O may extend shutdown despite finite SQLite acquisition/busy limits. Tauri intercepts close/exit and handled Ctrl+C, cancels diagnostics, prevents late starts and waits for pending controller operations plus the running task before exiting. Forced termination recovers from local durable intents and OS locking.

## Metadata and diagnostics

JSON numbers use arbitrary precision; monetary normalization uses exact decimal strings. The complete accepted product and distinct source envelope remain structured metadata; exact decoded bytes remain archived. Diagnostics strip terminal controls and exclude secrets and failed live bodies.

Test Access accepts no catalogue/archive handles and cannot mutate jobs or launch discovery. Public HTTP 200 does not prove account authentication. Firefox import retains supported first-party partitions and current session cookies; unsupported contexts explain manual-paste fallback. HTML gate classification requires explicit headers or visible gate evidence, rather than incidental background scripts.

Reprocessing reads only immutable successful archives through isolated parsers. It preserves observation times/references; failure keeps the prior good record and records a separate outcome. It cannot contact the storefront.

## Technical references

- [SQLx SQLite options](https://docs.rs/sqlx/0.9.0/sqlx/sqlite/struct.SqliteConnectOptions.html)
- [SQLite WAL](https://www.sqlite.org/wal.html) and [synchronous settings](https://www.sqlite.org/pragma.html#pragma_synchronous)
- [SQLite FTS5 external-content indexes and triggers](https://www.sqlite.org/fts5.html#external_content_tables)
- [Rust OS file locking](https://doc.rust-lang.org/std/fs/struct.File.html#method.try_lock)
- [Tokio blocking-task cancellation](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html)

These references describe APIs, not live storefront compatibility. See [validation evidence](VALIDATION.md).

## Desktop boundary

Tauri 2 hosts a bundled Vite/TypeScript frontend. There is one native window and no TUI, remote frontend, web server in release, or general filesystem/shell/network plugin. CSP permits bundled assets and IPC only; external navigation and new windows are rejected. Browser profile IDs are resolved from backend-discovered/current choices, not arbitrary frontend paths. Returned snapshots contain counters and labels, never credentials or catalogue handles. External text is escaped before rendering.

The controller uses a short-held state mutex, a separate operation gate and a watch snapshot. Polling reads cached bounded state once a second, without rescanning SQLite or replacing focused wizard inputs. Stop/Close bypass the operation gate, invalidate pending start generations and cancel work. Shutdown waits for the pending configuration/storage operation and the scheduler drain, then allows native exit. Pending diagnostics receive the shutdown token. Repeated close requests are idempotent.

Configuration/authentication changes require loaded settings. Existing manual secrets remain in Rust; the UI gets a presence flag and can keep the current values or submit replacement input. Save-only has no network access. Save & test explicitly applies the draft first. Session-only/browser/anonymous choices clear old remembered manual values. No frontend localStorage persistence is used. Webview profile/cache files live under `.kaijucrawl/webview/`.

The same catalogue, archive layout and parser subprocess entry point serve both the former TUI and desktop controller. No data migration is required for the UI change. Startup validates and reads totals; a new crawl uses the existing recovery/new-run protocol. `--internal-parser-worker` executes before Tauri initializes, retaining pipe isolation and kill/reap behavior.

Official references: [Tauri commands](https://v2.tauri.app/develop/calling-rust/), [capabilities](https://v2.tauri.app/security/capabilities/), [Vite integration](https://v2.tauri.app/start/frontend/vite/) and [platform prerequisites](https://v2.tauri.app/start/prerequisites/).

## Adventure player extraction

`players` owns the versioned, nullable range policy and fixed Responses wire contract. It uses the existing OpenAI client for bounded HTTP, credentials, shared admission and value-free errors. `controller::classification::kind` selects the concrete online algorithms inside the same tested coordinator, so retry/cancellation/drain behavior stays shared. The historical `classification` progress envelope and `classification_attempts` table also carry player runs distinguished by algorithm/run ID; additive known/unknown counters default to zero in historical data.

Migration 007 adds `adventure_players`, its range index and invalidation triggers. A row with two nulls is a completed analysis, distinct from an absent row. Selection joins current primary-role Adventures and uses a keyset cursor. `storage::players` atomically commits nullable results, attempts, usage/counters and revision after checking unchanged evidence/eligibility. Counts cascade when their prerequisite classification is removed and invalidate on applicable role/source changes. Product records and archives are untouched.

The Responses prompt caches one stable developer block containing policy and calibration examples with explicit mode, a 30-minute breakpoint and a versioned cache key. Dynamic book JSON follows in a separate unmarked user message. The server's minimum cacheable length still applies. This extraction starts with owner-selected GPT-6 Luna, `store:false`, no tools, no reasoning, and a 256-token structured-output limit, with the model escalation policy described below; Decisions retains disabled caching. Provider usage is validated and reported, never fabricated.

## Independent solo-play classification

`solo` owns the three-way Decisions policy. `classification::system::System` selects the fixed primary-role or solo-play system for shared classification persistence. Both select canonical `classification_source` evidence and keep separate result/membership keys; neither reads the other's output. The common coordinator handles bounded workers, all-request-error retries, checkpoints and cancellation for both systems and player extraction.

Migration 008 adds only the solo-play group tree and expands source-evidence invalidation. Existing roles, player counts, relevance scores and captures are untouched. Changing another analysis result does not invalidate solo play. The frontend uses a native analysis dropdown with per-method options and catalogue totals.

## Shadowrun / Call of Cthulhu source context

`earth_context::Kind` exposes four fixed analysis identities and strict location/date schemas on the existing cached Responses transport. `DateRange` uses [Chrono calendar dates](https://docs.rs/chrono/latest/chrono/struct.NaiveDate.html) with inclusive optional bounds, source precision and explicit/era/unknown basis. Typed validation rejects nonexistent dates, reversed ranges and inconsistent precision; CoC fallback bases are normalized to fixed ranges in Rust. No computer clock or previous model output determines those ranges. Location outputs remain nullable strings.

Migration 012 builds `earth_scope` from `products.record.rule_systems`, keyed by target/product/system with a unique `(system, document)` index preserving product rowids. This is a narrow analysis eligibility projection, not general Search. Initial population is SQL-only within fenced asynchronous bootstrap; later capture/reprocess triggers maintain it. `earth_context` results reference eligible scope rows and cascade on loss of eligibility. A separate evidence-change trigger invalidates results for changed inputs while preserving them for unrelated rating/analysis changes.

`storage::earth_context` admits indexed keysets, checks current evidence/scope at commit and saves typed fields, provenance, attempts, usage and progress atomically. It participates in the common logical-CPU coordinator, all-API-error retry/backoff and cancellation/drain. Indexed result aggregates and bounded per-book contributions add four methods to the existing live totals. One typed desktop start command serves the four dropdown entries; there is no caller-supplied subset or automatic paid work.

## D&D setting and context analyses

`dnd_settings` supplies fifteen independent Decisions questions (fourteen settings plus setting independence). Shared classification persistence saves overlapping memberships, adds Forgotten Realms for Al-Qadim, and keeps Any/Unknown distinct. `dnd_context::Kind` defines three strict typed Responses contracts. `responses` shares the existing player transport, envelope/usage validation and stable-prefix caching without changing player-count behavior. Storage validates typed outputs again before saving.

Migration 011 adds setting groups, a typed context table and dependency triggers. `storage::dnd_context` selects original evidence through bounded rowid keysets, verifies positive D&D eligibility/source stability and uses owner-fenced commits. The coordinator reuses bounded admission, retry queues and cancellation/drain. Context nulls/empty lists are completed; refusals preserve prior good results. Overview baselines and per-book contributions include all fourteen methods, including dependent context/setting removals. Two narrow desktop start commands expose four dropdown choices; there are no new browsing endpoints.

## D&D decisions and catalogue coverage

`dnd` owns a three-way compatibility policy and nine independent edition choice questions. The common classification `Selection` carries a summary category plus a bounded membership vector; existing single-category systems supply one member. Edition results store nine assessments and multiple memberships in the existing classification tables. Migration 009 adds group definitions and prerequisite/canonical invalidation; no analysis is backfilled. The edition subset and commit both verify current positive D&D compatibility, and evidence remains the original allowlisted source projection.

`storage::overview` returns compact indexed aggregates for all fourteen analyses in one read transaction. `Controller::analysis_overview` caches by catalogue UUID and analysis revision; capture/delete triggers join existing result revision updates. This read-only path shares bounded catalogue read admission and is separate from cheap status snapshots. At analysis start, storage reads one aggregate baseline into transient `analysis::Status.overview`. Each online commit reads the affected book’s bounded analysis contributions before/after its write, including trigger-driven dependent removals; weighted batches join old scores in their existing bounded selection. A private copy receives the delta and becomes visible only after commit. The coordinator publishes these totals through its existing watch snapshot, including cancellation/drain. No schema/history migration or per-result aggregate scan is needed. The frontend applies those totals immediately at the regular one-second snapshot cadence and rejects older in-flight aggregate reads. Idle loading, completion and observed capture/reprocess changes use the cached aggregate command with one request plus one coalesced refresh, explicit error recovery and no hidden-tab reads. There is one totals block and no latest-run panel. The primary start button itself becomes Cancel, preserving position/focus and avoiding post-start scrolling. Backend run identity, counters, usage and recovery history remain unchanged.

## Shadowrun / Call of Cthulhu editions

`earth_editions::Kind` defines the two fixed systems and their seven/eight independent Decisions questions. It reuses the D&D edition-answer validator and generic `classification::System` persistence. Selection drives indexed `earth_scope` keysets joined to canonical evidence; the source-only allowlist, size bound and per-target identity remain unchanged. Commit rechecks current metadata and scope. Results, overlapping memberships, usage, attempts and progress share one fenced SQLite transaction.

Migration 013 adds group trees and source/scope invalidation triggers only. Existing context results and captures are preserved; edition results neither require nor invalidate other analyses. `storage::earth_editions` supplies bounded per-book contributions and indexed cached aggregates for the existing live overview. One typed desktop command adds two dropdown entries without changing the common worker/retry/cancellation transport. Requests are uncached Decisions; cached Responses extraction is unchanged.

## Structured-output model escalation

`responses::Model` owns the four fixed model identities, reasoning compatibility and bounded deadlines. Its request update changes only model/effort/budget; original evidence, strict schema, caching and store=false remain intact. `responses::attempt` returns either typed output or safe failure metadata with available validated usage. It never retains generated text or provider error bodies.

The existing single SQLite coordinator advances a work item's model on InvalidResponse/ResponseTooLarge, queues the same book through existing backoff/admission, and retains that model on transport/HTTP retries. Invalid Astra output commits an unsuccessful attempt and continues; accepted refusals follow the existing unsuccessful path. Other books retain independent model state. Cancellation still drains the common JoinSet. Rejected-output usage and escalation metadata use fenced progress updates; accepted output stores its actual returned model in existing provenance columns. Defaulted progress JSON fields preserve old history without schema or algorithm changes. Committed coverage counts are unchanged by rejected attempts.

## OSR metadata subset and multi-label systems

`osr` defines fifteen independent binary Decisions questions and maps the complete answer set to named memberships plus Other, or exclusive Not OSR. Source-only evidence and the existing uncached Luna client/coordinator are reused. `AnalysisWorkspace::Osr` selects `Kind::Osr` followed by `Kind::OsrLevels` through `start_system_analysis`; no additional individual UI/IPC action is needed. Discovery completion is checked before ownership and again after acquisition.

Migration 014 creates `osr_scope_source` and indexed `osr_scope` from canonical Rule System strings equal to Old-School Revival (OSR), case-insensitively after trimming. Source triggers maintain membership/invalidation. The classification storage path joins this bounded subset and uses `e.document` keysets, validating eligibility/evidence again before commit. Existing group/results/attempt tables hold output/provenance. `storage::osr` computes bounded per-book contributions and indexed idle aggregates; overlapping memberships never inflate distinct completed totals. Scope changes and source changes remove only dependent OSR results through its own triggers, alongside existing independent invalidation rules.

The shared frontend config inserts OSR between D&D and Shadowrun. Its two-card layout, original metadata subset explanation, per-system counts and monochrome light graph-paper CSS require no browser/network assets or native UI changes. Saved Not OSR results are reused and shown as complete.

## OSR Adventure levels

`osr_levels` owns the fixed schema, cached source-only prompt and validation against requested membership keys. The same Responses coordinator supplies model escalation, request retries, usage and cancellation. `storage::osr_levels` uses the schema-15 eligibility/source views over indexed OSR metadata scope and current role/membership rows. One canonical document remains one worker/checkpoint/result unit, even when it produces several system ranges. No transaction spans a model request.

`osr_levels` stores parent completion/provenance and bounded known/unknown/not-applicable counts; `osr_level_ranges` stores typed individual system/game bounds. A fenced transaction replaces all children and the parent, recording the attempt and live coverage only on commit. Trigger invalidation follows source/role/system membership changes. OSR classification skips rewriting an identical membership set, retaining dependent valid levels on cancelled/refused replacements. Per-book coverage reads use indexed target/product predicates without materializing the whole eligibility view; idle totals use cached narrow aggregates. The dashboard keeps classification and Adventure-level scopes separate and counts completion by distinct books.

## Warhammer family routing and context

`warhammer::Family` defines the two source-only Decisions contracts: one family check plus seven Fantasy or eight 40k game/edition checks. Typed selection preserves unknown family-confirmed games separately from uncertain/negative family outcomes. Contradictory answers are rejected inside the request path, retaining usage and shared retries. `warhammer_context::Kind` supplies four cached strict Responses contracts and reuses D&D's place/date data validation, without changing D&D behavior. The common coordinator handles workers, retries, structured-output escalation and cancellation.

Migration 016 backfills only a narrow Rule System eligibility index, including generic Warhammer in both families. Fenced classification admission/commit uses that index; context additionally requires a current positive family. A separate typed context table depends on scope and classification, with source/family invalidation. `storage::warhammer` computes indexed idle aggregates and bounded per-book deltas for six methods; no evidence is loaded for totals. Game replacement with unchanged evidence and continued family support retains context. Both UI workspaces invoke the existing sequence command; no additional per-book or Search IPC is exposed.

## 40k game tables and details

`library` defines eight game IDs, five sort IDs, typed keyset cursors and bounded list/detail DTOs. `storage::library` starts from `group_members`, joins canonical products and current Warhammer/role/context results, and binds every variable input. Count and page share a read transaction through `controller::library`; the existing reader semaphore limits concurrent commands. Sorts operate only on a selected category, with no new materialized index, source projection, migration or archive access. Details require the same current membership. Unknown status groups cannot be requested through the enum.

`ui/books.ts` owns per-game sort/cursor state, table/detail replacement, escaped source text and request epochs. The shell retains mounted Analysis content and adds nine accessible 40k tabs, with a separately named library panel to avoid dashboard count-ID collisions. Hidden workspaces do not fetch books; Back uses the retained page. Refresh resets the cursor and reads current saved results. The shared dark iron/red theme styles the table and full-page details. Production assets include no preview fixture or remote media.
