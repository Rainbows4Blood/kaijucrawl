# kaijucrawl specification

Version: 2.25 — Warhammer 40k game tables and saved book details
Date: 2026-10-30
Status: requirements baseline; initial Rust implementation present — see [validation evidence](docs/VALIDATION.md)

Initial target: DriveThruRPG (`drivethru`)

## Contents

1. [Purpose and scope](#1-purpose-and-scope)
2. [Decisions and invariants](#2-decisions-and-invariants)
3. [Desktop application](#3-desktop-application)
4. [Fixed local storage](#4-fixed-local-storage)
5. [Authentication](#5-authentication)
6. [DriveThruRPG adapter](#6-drivethrurpg-adapter)
7. [Fixed discovery and crawl lifecycle](#7-fixed-discovery-and-crawl-lifecycle)
8. [Product success and parsing](#8-product-success-and-parsing)
9. [Metadata contract](#9-metadata-contract)
10. [SQLite persistence and ownership](#10-sqlite-persistence-and-ownership)
11. [Successful response archives and consistency](#11-successful-response-archives-and-consistency)
12. [HTTP behavior and failure handling](#12-http-behavior-and-failure-handling)
13. [Automatic runtime policy](#13-automatic-runtime-policy)
14. [Reprocessing after parser updates](#14-reprocessing-after-parser-updates)
15. [Stopping and recovery](#15-stopping-and-recovery)
16. [Rust implementation structure](#16-rust-implementation-structure)
17. [Diagnostics and resource bounds](#17-diagnostics-and-resource-bounds)
18. [Acceptance criteria](#18-acceptance-criteria)
19. [Delivery sequence and validation](#19-delivery-sequence-and-validation)
20. [Legacy mapping and sources](#20-legacy-mapping-and-sources)

## 1. Purpose and scope

kaijucrawl builds a durable catalogue of product metadata exposed by DriveThruRPG. It discovers product IDs through sitemaps, retrieves product details from the product API, preserves the full extracted metadata, and archives successful source responses.

The application is written in Rust and operated through a Tauri 2 desktop application with a bundled TypeScript interface. All application configuration, remembered secrets, the authoritative SQLite catalogue, successful response archives and temporary staging live under the current user's home folder at `.kaijucrawl`. This location is fixed and cannot be changed through settings, environment overrides or command-line options. PostgreSQL, S3 and other network storage backends are not supported.

The first release implements only DriveThruRPG. Dungeon Masters Guild, Storytellers Vault, Ulisses eBooks, and other storefronts are future adapters. Do not expose nonfunctional targets as selectable checkboxes. Shared machinery must carry an explicit target identity so another adapter can be added without changing the meaning of existing data.

This is a catalogue of first successful captures. It is not an automatically refreshed mirror. Successful records may become stale; prices, descriptions, ratings, and availability are observations from their capture time.

### 1.1 Included

- Automatic local storage initialization, target selection, authentication, and dashboard.
- Sitemap discovery on every crawl run.
- Automatic resumption and renewed eligibility of previously unsuccessful work.
- Full DriveThruRPG metadata extraction, including HTML and plain-text descriptions.
- Independent network limits per target.
- One active mutating application instance per catalogue.
- Successful response archival and recovery across local database/filesystem interruptions.
- Reprocessing of saved successful responses specifically for parser updates.
- Explicit analysis with persisted weighted relevance, primary book roles, Adventure-only player counts, independent solo-play categories, D&D compatibility/editions and D&D setting/context analyses and Shadowrun/CoC context/editions and OSR systems/per-system Adventure level ranges and Warhammer games/context, with saved results and catalogue-wide totals.
- Read-only Warhammer 40k game tables with sortable saved metadata and isolated book details (§3.20).
- An OpenAI Decisions backend, explicit synthetic connection test (§3.5) and primary book-role grouping analysis (§3.6).
- Windows and Linux as first-class operating systems.

### 1.2 Excluded from the initial release

- General Search, metadata filters and catalogue browsing/book details beyond the 40k game views in §3.20; broader data views remain deferred.

- Automatic or manual product refresh and age-based recrawling.
- API collection/listing discovery, category-page crawling, manual seeds, and numeric ID enumeration.
- Product-count or discovery-page run limits.
- CSV/JSON export workflows and machine-readable dashboard output.
- An archive-disable switch or archival of unsuccessful product responses.
- Importing legacy Python catalogues or remote storage. Existing KaijuCrawl TUI data is supported in place.
- Distributed crawling, multiple cooperating writers, a daemon, a hosted web application, or a scheduler.
- Cross-store merging or deduplication.
- Custom group definitions, general group browsing/search outside the 40k views in §3.20, and further grouping algorithms beyond primary book role (§3.6), solo play (§3.8) D&D compatibility/editions (§3.9), settings (§3.11), Shadowrun/CoC editions (§3.13), OSR systems (§3.17) and Warhammer games (§3.19).
- Downloading books, preview documents, images, audio, video, or purchased assets.
- Browser-driven crawling, automated storefront login, challenge solving, or an HTML page fallback for product extraction.

Browser cookie import is an authentication convenience, not a browser automation crawling backend.

## 2. Decisions and invariants

The following are requirements, not defaults that an implementation may silently replace.

| ID | Requirement |
| --- | --- |
| INV-01 | Launching `kaijucrawl` opens the Tauri desktop dashboard without requiring a terminal. Local storage is initialized automatically; there is no storage configuration wizard. |
| INV-02 | Only `drivethru` is implemented initially. Target selection uses checkboxes. |
| INV-03 | Every new crawl run performs sitemap discovery, including after a prior interrupted run. |
| INV-04 | A successfully collected product is never fetched again by normal crawling, including when sitemap `lastmod` changes. |
| INV-05 | Outstanding and previously unsuccessful products become eligible on each new run. Attempts within one run are bounded. |
| INV-06 | Product success requires verified identity, a nonempty title, a nonempty full plain-text description, and durable SQLite/local archive persistence. |
| INV-07 | Preserve all metadata fields extracted by the legacy DriveThruRPG parser. |
| INV-08 | Identity is `(target_key, product_id)`. Cross-store merging is a separate future processing step. |
| INV-09 | Exactly one instance owns mutating work for a catalogue at a time. Internal parser subprocesses are part of that instance. |
| INV-10 | Automatic concurrency and server cooldowns are independent per target, even if future targets share an API host. Robots pacing is scoped to the origin supplying the policy; there is no added request interval. |
| INV-11 | Always archive successful product responses. Do not archive failed, blocked, malformed, mismatched, or incomplete product responses. |
| INV-12 | Authentication problems pause the affected target. Local database or archive failures pause the entire crawl. |
| INV-13 | Stopping or closing the application ends its work gracefully; durable state supports recovery after an abrupt termination. |
| INV-14 | Reprocessing reads saved responses and never fetches products or changes original observation timestamps. |

Failure classification, persistence protocols, menu layout, and runtime policy below make these decisions implementable. They are part of the initial design baseline and must remain consistent with the invariants.

## 3. Desktop application

### 3.1 Entry and navigation

`kaijucrawl` opens a native Tauri 2 desktop window on Windows and Linux. The TUI, terminal-only startup check, terminal keyboard shortcuts and terminal dependencies are retired. Help/version remain conventional non-operational options; there is no headless crawl mode or second operational interface.

Launch directly into Crawlers → Overview and initialize fixed local storage asynchronously. Never start a crawl just because the window opened. The main menu is a closed-by-default left flyout opened by a labeled menu button in the application header. Its destinations, in order, are **Crawlers**, **Discovery**, **Dungeons & Dragons**, **OSR**, **Shadowrun**, **Call of Cthulhu**, **Warhammer Fantasy** and **Warhammer 40k**. Each analysis destination retains its **Analysis** tab; 40k also has the eight game tabs in §3.20: Discovery uses §3.15 and the system dashboards use §3.16. Preserve existing analysis algorithms, IPC, tests and saved results. The old dropdown component remains in source/tests but is not mounted. Opening a workspace may read cached saved coverage; it never runs an analysis or sends a paid request.

Move the former sidebar views into horizontal Crawlers tabs: Overview, Authentication, Unsuccessful jobs, Local storage and About. Tab/workspace switches must not start, stop or otherwise change a crawl or analysis. Show the current crawl state beside the Crawlers heading on every tab; keep storage readiness and Close application in the application header. The flyout and workspace switches preserve each workspace's controls and any in-progress authentication draft. Selecting a destination closes the menu and focuses its selected tab.

The flyout supports an explicit Close button, Escape and outside-click dismissal, restores focus to its opener on dismissal, and contains keyboard focus while open with background content inert. This navigation flyout is the sole exception to the isolated-view presentation rule. Tabs use labeled tablist/tab/tabpanel semantics, one tab stop and Left/Right/Home/End switching. Narrow windows retain reachable tabs through horizontal scrolling. Keep the actual content views isolated:

| View/action | Behavior |
| --- | --- |
| Overview | Lifetime/current-run totals, target checkbox, Start, Stop and paused-run Resume, queue, origin pacing, parser work and bounded recent events. |
| Authentication | Guided cookies/browser configuration, explicit Save only or Save & test access, replacement and clearing. |
| Unsuccessful jobs | Bounded keyset pages of current-run failed jobs, IDs, classified reasons and timestamps. |
| Local storage | Fixed directory and file purposes, validation state and Retry; no editable storage configuration. |
| About | Application/catalogue/record/parser versions and Reprocess saved responses. |
| Discovery → Analysis | Four system-independent coverage cards, one Run analyses/Cancel action, and optional Rerun all; a backend sequence executes the four stages (§3.15). |
| Dungeons & Dragons / OSR / Shadowrun / Call of Cthulhu → Analysis | Themed system coverage dashboard with one gated Run analyses/Cancel action and optional Rerun all (§3.16). |
| Close application / window close | Stop gracefully, cancel diagnostics, wait for pending operations and release ownership before exit. |

No selected target disables Start with a reason. Target selection affects the next run. Start/Reprocess/storage checks and Analysis cannot overlap active mutating work or pending configuration operations. Stop and closing remain responsive even while initialization or a cookie import is pending. A late start completion cannot launch work after Stop/Close. Authentication replacement takes effect on an explicit next start or paused-run resume. Resuming an authentication pause requires a passed Test Access with the current saved credentials.

All normal flows support mouse and keyboard with native Tab/Shift+Tab focus traversal, Enter/Space activation, arrow-key choice controls, visible focus and labeled inputs. Long views scroll. Target a 1240×820 initial window and a 760×560 minimum; responsive layouts keep navigation, current question and actions reachable. No status relies on color alone. Network, filesystem and parser work must not block the webview event loop.

### 3.1.1 Guided authentication and visual style

Crawlers and Discovery keep the coordinated dark palette: cyan/green actions and success, violet navigation/progress, amber pending information and labeled errors. System workspaces use the palettes in §3.16. Authentication asks for a method first, then only its relevant questions: browser profile/container or masked website/API cookie headers and session-only/remembered storage. Back retains answers. Temporarily revealing new cookie inputs must not reveal them in review. Existing saved cookies are represented by presence and a keep-current choice, never sent back to JavaScript as values.

Validate each step and retain editable answers on failure. Save only explicitly applies configuration without network access. Save & test access explicitly applies the reviewed draft, then displays a separate result view. Testing itself never changes product, queue or archive state. Failed results provide Back to the retained review. Choosing session-only manual cookies clears previously remembered cookie values; choosing browser import or anonymous access also removes remembered manual headers. Browser selection remains remembered, with cookies reimported in Rust at each start.

Every secondary content view replaces dashboard content. Do not draw dialogs or result windows over the dashboard; the explicitly requested main-menu flyout (§3.1) is navigation only. Account authentication stays unverified unless supported by explicit account evidence.

### 3.1.2 Desktop boundary and compatibility

Keep the Rust domain, scheduler, authentication providers, parser subprocess protocol, SQLite and archive implementations. Tauri exposes narrow asynchronous commands for snapshots, target selection, storage validation, browser choices, authentication, diagnostics, run controls and failure pages. Status snapshots contain no cookie values or database handles. The frontend receives only browser IDs/labels/container choices; the backend resolves discovered or already selected profile paths. Render external text as text, not executable HTML. Use a restrictive content security policy, no remote assets or hosted frontend, no generic filesystem/shell/network plugin permissions, and reject external navigation/new windows.

Continue using the non-configurable existing `.kaijucrawl` directory. Preserve catalogue identity, products, observation times, immutable archive references/bytes, jobs, run history, save intents, settings and remembered cookies. No import, copying, reset or fresh catalogue is performed when existing files are present. Historical migrations 4–9 added derived Search indexes and separate analysis results, groups, checkpoints and invalidation. Schema 10 removes only the Search structures, preserving canonical data and every analysis result/history row. No indexing, score calculation or paid request runs during startup. Product record schema 2 and parser `drivethru-rust-1` stay unchanged. Webview cache belongs under `.kaijucrawl/webview/`.

Starting after a prior TUI run follows the same recovery/new-run protocol: reconcile pending intents before network work, skip successful products, retry never-successful work and rediscover sitemaps. Historical interrupted runs remain historical; desktop Resume applies to a paused run in the current process. OS locking and SQLite fencing prevent simultaneous TUI/desktop mutation. The existing executable's private parser-worker entry point must run before initializing Tauri and must not open windows or read credentials.

### 3.2 Dashboard

Show both current-run activity and durable catalogue totals, clearly distinguished:

- Run state: idle, discovering/crawling, waiting for retry, authentication paused, storage paused, stopping, finished, or interrupted.
- Per-target discovery state: sitemaps queued, processed, retrying, and failed; distinct products seen this run; newly discovered products.
- Product queue: pending, active, waiting for persistence, retry scheduled, successful this run, and unsuccessful this run.
- Lifetime successful product count and last successful save time.
- Active download/parser work, effective website/API robots pacing, automatic logical-CPU worker count, server cooldown, current-run average throughput, and elapsed time. Label robots-imposed delays and unknown policies explicitly. Show the active run's actual worker count and policy state.
- SQLite and local archive availability; sanitized failure reasons and next available action.
- A bounded recent-events panel and an inspectable list of unsuccessful jobs with IDs, reasons, and timestamps.

Discovery totals can grow. Do not report a global storefront completion percentage from an unknown denominator. A run can finish with unsuccessful products or discovery failures; display these prominently. A drained queue does not prove that every product on the storefront was discoverable or accessible.

Count a product as saved only after final persistence succeeds. Reimports of cookies, retries, and duplicate sitemap entries must not inflate success totals.

### 3.3 Retired Search and scoped data views

Remove the Search workspace entirely: navigation, text queries, metadata filters, multiselect suggestions, sorting, pagination, saved-book detail views and Analysis links to them. Remove their frontend components and public IPC/controller/query implementations. Do not retain a hidden Search mode. The later authorized 40k game tables in §3.20 are a narrow new browsing surface, independent of retired Search. The Crawlers workspace retains the operational Unsuccessful jobs view. Discovery exposes system-independent analyses; system dashboards and 40k tables use the navigation in §3.1.

Schema 10 retires only Search's derived projection, FTS index, facets, backfill checkpoint and maintenance triggers (§10.1). Preserve canonical products, original observations, archives, settings, cookies, crawl state, all analysis results, group memberships, provenance and history. The first new data views are the 40k tables/details in §3.20; broader catalogue views remain future work.

Analyses read directly from canonical products with bounded keyset pagination on `products.rowid`. Historical Search document IDs were those same rowids, so checkpoints/attempt identities stay compatible without conversion. No Search backfill or projection maintenance runs at startup, during captures, reprocessing or analysis. Keep historic run selection JSON as opaque history; no Search query parser or execution remains. New analysis starts accept only their fixed algorithm scope and explicit rerun choice.

### 3.4 Explicit Analysis

The following algorithm contracts and existing implementation remain intact. Discovery exposes the four system-independent methods as a single sequential action (§3.15). References below to a method dropdown and individual start buttons describe the retained standalone Analysis component; it is not mounted by the current shell. System methods are exposed through the themed run-all dashboards and the Discovery gate in §3.16.

Analysis is a separate workspace; weighted relevance runs entirely offline. Nothing computes scores or sends model requests on startup, when viewing totals or after a capture. The user chooses an analysis, not a subset. Weighted relevance, primary book role, solo play and D&D compatibility operate across the saved catalogue. D&D edition inference selects only current positive D&D classifications. Adventure player counts selects only current primary-role Adventures. These algorithm-defined selections are independent of target selection or cookies. Algorithms may define their own built-in subsets; no generic user subset selector or IPC query override is exposed. Freeze the algorithm's selection and upper product rowid at run start. Preserve any historical filtered-run queries in existing run history without offering them as new user selections.

The first algorithm, `weighted-rating-1`, uses the owner-approved formula:

`score = (saved average rating × saved rating count + 60) / (saved rating count + 20)`

This blends twenty baseline ratings at 3/5 with the observed rating. Use the same fixed baseline for all runs; do not derive it from the currently selected books. A 4.7 average with 1,000 ratings scores 4.666667, above a 5.0 average with 3 ratings at 3.260870. This is rating-based relevance, not text-query relevance, sales or a verified quality assessment. Accept finite ratings from 1 through 5 and integral counts from 1 through 1,000,000,000. Missing, malformed, nonfinite or out-of-range inputs produce no score; count them separately, never treat them as zero-rated books. Parse raw numeric/text fields strictly rather than interpreting failed SQLite casts as zero. Round once to six decimal places and persist millionths as integers for stable sorting; UI summaries may display three decimals.

Keep analysis definitions, orchestration and SQLite result storage separate from source metadata and crawling. Persist results by `(target_key, product_id)`, with algorithm version, input rating/count, calculation time and run identity, including a null score for analyzed books without usable inputs. Store the run's query/scope, start/end/state and total/processed/scored/skipped counts. Never modify canonical products, original observation times, jobs, crawl runs, credentials or archives. New captures remain unscored. Product deletion removes its results; changes to rating/count during parser reprocessing invalidate its score transactionally without recalculation. Unrelated metadata updates retain scores. An algorithm change requires a new algorithm version and explicit recalculation; count only current-version results in analysis coverage.

Permit one analysis at a time, mutually exclusive with crawling, reprocessing and storage initialization. Reuse the OS file lock and owner/generation fencing for every mutation. Process at most 500 books in each keyset batch, with scores, progress and score revision committed in one transaction. Do not hold transactions across external I/O, read the whole catalogue into memory, or transfer all results to the webview. Publish committed progress independently of crawl counters. **Cancel analysis** and Close stop between batches, await executing SQLite work, retain completed batches and release ownership before another operation starts. A successor marks an unfinished run interrupted; startup never resumes calculation automatically. The user can explicitly run again over the catalogue. Each algorithm implements its own fixed eligibility predicate; there is no general-purpose user query machinery.

Analysis progress and algorithm choices survive tab switches. Failure, cancellation and partial completion remain distinguishable from a completed run, and weighted relevance issues no external requests. Saved score input/version/time provenance remains in SQLite for future data views.

### 3.5 OpenAI Decisions foundation

Books may belong to zero, one or multiple groups/subgroups across analysis systems. The first system assigns one primary book-role category (§3.6). Group browsing, group-specific sorting and simplified search remain subsequent UI work; this revision retains classifications and their provenance without a per-book browsing interface.

Use the [OpenAI Decisions API](https://developers.openai.com/api/docs/guides/decisions), `POST https://api.openai.com/v1/decisions`, with `gpt-6-luna`. Keep this Rust client separate from storefront HTTP, robots, cookies, rate limits and error classification. Read `OPENAI_API_KEY` only from the app process environment when an explicit request starts. Never return it to the webview, put it in settings/catalogue/archives/logs, or inherit it in parser workers. A missing/invalid key affects only Decisions, not offline features or crawling. Environment credentials are the sole exception to the lack of runtime configuration overrides; no model, endpoint or spending overrides are exposed.

Disable prompt caching for all current Decisions requests (classification, its retries and the synthetic diagnostic): send `prompt_cache_options: {"mode":"explicit"}` with no marked input breakpoints. Book evidence changes per request and question instructions are outside the reusable cache prefix. This is a fixed transport policy, not a user setting. Preserve actual provider usage and historical cache counters; do not fabricate zeros or invalidate saved classifications.

The backend supports text or structured JSON evidence and named predicate, fixed-choice and ordered-score questions with text instructions. Images, tool histories, streaming and arbitrary frontend requests are not exposed. Questions have unique nonempty names up to 128 bytes and instructions up to 8 KiB; support 1–400 questions, 2–255 distinct typed string/boolean choices and 2–10 distinct named score levels. Text evidence is at most 64 KiB; JSON is an object or nonempty array of objects with at most 32 container levels and 10,000 values. Limit the complete encoded request to 128 KiB and response to 1 MiB. Reject oversized requests, never silently truncate evidence.

Reuse the client with shared admission capacity equal to the available logical CPU count (fallback one), across all clones. This is automatic and not user-configurable. Set a ten-second connection limit and thirty-second deadline per HTTP attempt including client admission waiting and response reads; structured Sol/Astra attempts use the 120-second exception in §3.14. Verify HTTPS certificates and reject redirects. The transport and connection diagnostic issue one attempt with no hidden retries. Classification coordinates retries for every API-request failure as described in §3.6; cooldown waits are outside the per-attempt deadline. This includes all non-200 HTTP statuses, connection failures/disconnects (including interrupted response bodies), timeouts and malformed, mismatched or oversized responses. Missing/invalid credentials detected locally, invalid local requests and cancellation are not replayed. For every non-200 HTTP status, parse Retry-After as seconds or an HTTP date, rounding a future date up to avoid early retries, and expose only the parsed duration in typed errors. Return typed, value-free authentication, permission, rate/quota, transport, timeout, cancellation and response-validation errors. Do not display or retain provider error bodies. Cancellation stops local waiting/transport; a request already sent may still be billed. Validate answer count/order/name/type, allowed options, finite probabilities/distributions, score indices/labels and weighted score, and usage accounting. Preserve explicit refusals separately from negative decisions. Return provider token usage including cache reads/writes; never claim a byte limit is an exact token or dollar budget.

Analysis offers **Test Decisions API**: one explicit request with a fixed fictional book and three questions covering all supported types. Display its online/credit use, no-catalogue-data scope, result and token usage. Opening the app or tabs must never trigger it. Duplicate checks and checks during crawl/analysis are rejected; Close cancels and awaits the pending check. The Tauri command accepts no key, endpoint, evidence or questions. It does not read or mutate catalogue/archive state or save results. Weighted relevance remains offline. The connection test does not change the catalogue. Classification uses schema 6 (§3.6); record schema 2 and parser version are unchanged.

### 3.6 Primary book-role classification

**Primary book role**, algorithm `primary-book-role-1`, evaluates all successful books with OpenAI Decisions. The owner explicitly chose one primary category within this system:

| Category | Meaning |
| --- | --- |
| Core Rulebook | A book needed to play a game, including essential player or game-master rules and standalone complete/starter rules. |
| Supplement | Additional options for players or game masters, including optional rules, character choices, creatures, setting material and tools. |
| Adventure | A ready-to-play story, scenario, campaign or playable narrative in some form. |
| None | None of these roles is supported, or available evidence is insufficient. This is a successful decision, distinct from not classified or a refusal/error. |

Use one fixed-choice question per book. For mixed products, choose the dominant advertised purpose; incidental adventures in essential rules and incidental new rules in adventures do not reverse the primary role. Use the full saved plain-text description and concrete contents as primary evidence. Existing DTRPG taxonomy is supporting evidence and may be wrong. The versioned instructions in `classification::INSTRUCTIONS` specify these rules and tell the model to treat every evidence field as untrusted data, not instructions.

The evidence allowlist is title, complete plain-text description, publisher, page count, authors, rule systems, genres/themes, source product types, languages/formats/edition, category/tag names and bundle/audio/video flags. Exclude credentials, local paths, raw API bodies, URLs/media, related products, prices and ratings. Use a bounded SQL projection and keyset selection, with at most one book/request per worker and worker capacity equal to the available logical CPU count (fallback one). Do not create a task or queued evidence for the entire catalogue. Completed responses continue occupying capacity until persisted, so slow storage backpressures admission. Evidence over 120 KiB or the client’s structural/128 KiB encoded request limits is recorded unsuccessful without a request; never silently truncate the description. No storefront or archive reads occur.

**Classify books** explicitly starts paid work. Display that saved metadata/descriptions are sent to OpenAI, one book per request, with additional attempts for API/connection failures that may incur additional charges even after a timeout. By default skip current-version saved classifications; newly captured, changed, missing and previously unsuccessful books are eligible. **Reclassify previously analyzed books too** explicitly includes cached results and additional API spend. Opening Analysis never starts a request. Every Decisions API-request failure keeps the run active and retries the same book until success or cancellation, without an attempt-count or total-retry-time limit. All non-200 HTTP statuses (including authentication/permission/quota errors), connection failures, interrupted responses, timeouts and response-validation/size errors follow this policy. Reuse the identical bounded evidence and questions on retry. Account/key problems may require correction before retrying can succeed; this owner-requested policy does not infer permanence from a status or inspect provider error bodies. Start per-book exponential backoff at two seconds, doubling to a five-minute base maximum, with zero to one second of random jitter. A longer valid Retry-After is a minimum and is never shortened to the base cap. Share the longest outstanding cooldown across the run, stop all new/retry dispatches during it, continue saving successful in-flight responses, and prioritize queued retries when the cooldown ends. Each failed response increases only that book's delay; concurrent failures do not multiply one global exponent. Local configuration/preflight, storage, ownership and unexpected worker failures still stop admissions and drain workers; they are not remote requests to replay. No provider error body is retained or interpreted. Cancel/Close remain available throughout any wait. Preserve already-received valid results while draining unless storage has failed. Account rate limits still apply; see the [OpenAI rate-limit guidance](https://developers.openai.com/api/docs/guides/rate-limits). Refusals and oversized evidence fail only that book, never map to None and never overwrite a previous good result. Keep successful, unsuccessful and remaining counts, each category's newly saved count, current results kept, worker capacity and active/saving work, dispatched request and retry counts, waiting-book count, fixed value-free failure reason with HTTP status when available, shared cooldown seconds and reported token/cache usage in the backend progress/history contract. The UI shows catalogue totals only (§3.10), with essential cancellation/retry/error feedback; it does not render per-run statistics. Retriable responses do not count as processed or unsuccessful books; the eventual saved/refused result counts once. Confidence and probabilities are model estimates, not calibrated guarantees.

Migration 006 adds hierarchical `analysis_groups`, many-membership `book_group_memberships`, versioned `book_classifications`, durable `classification_attempts` and classification progress in `analysis_runs`. The `book_role` system has a root and four category children. Currently each successful decision atomically replaces exactly one membership within this system; future systems can have independent memberships. Distinguish None from an unclassified book. Persist category, model, rule version, probability distribution/confidence, evidence hash, timestamp and run ID separately from canonical records. No source metadata, observation times, jobs or archive references/bytes are rewritten. Relevant evidence changes invalidate this system's result/membership; unrelated rating/price/raw metadata changes retain it. Product deletion cascades results and memberships.

Reuse exclusive catalogue ownership and the existing analysis mutation gate. A single coordinator owns selection, counters and SQLite writes; workers perform HTTP concurrently through the shared client. Keep the admission keyset cursor independent of completion order. The persisted last ID is a monotonic completion high-water mark, not proof that earlier books finished; a new explicit run selects eligibility from saved results starting at the beginning. Reject duplicate outcome commits. Before HTTP, commit an in-flight dispatch record under fencing; after any retryable request failure change that attempt to retry_wait without advancing progress. Redispatch requires this state and atomically returns it to in_flight while incrementing dispatched/retry counters. At most worker-capacity books may be active, awaiting persistence or queued for retry; hold no transaction during the request. After a validated response, verify unchanged evidence and commit result, membership, attempt, counters/usage and analysis revision in one fenced transaction. Cancel/Close stop admissions, cancel outstanding HTTP, drain every worker, finish any executing SQLite commit and release ownership. No background task survives ownership release. Received results are saved before observing the next cancellation boundary. Already-sent interrupted requests may be billed; Decisions token totals only cover received validated responses. Structured Responses additionally preserve valid usage from rejected outputs as described in §3.14. Retried timeouts/invalid responses can therefore cost more than the reported usage. A successor marks unfinished runs and both in-flight/retry-wait attempts interrupted; startup never resumes paid work. Explicit default rerun skips every committed current result. API billing and SQLite commits cannot form an exactly-once transaction. Optional worker/retry/activity fields extend the existing progress JSON without a schema change: historical runs default to one worker and zero retries, while live activity/waits always reload as zero and the optional live failure reason reloads as absent. Backoff applies to the active run; an interrupted run is not automatically resumed. New explicit runs reselect unfinished books. Model, policy version and saved classifications remain unchanged.

Weighted-relevance and primary-role start commands use the catalogue-wide algorithm scope; player-count extraction uses the fixed Adventure scope. No user query is accepted. Saved classifications, memberships and provenance remain available to future data views.

### 3.7 Adventure player counts

**Adventure player counts**, algorithm `adventure-players-1`, explicitly analyzes only saved books with a current `primary-book-role-1` Adventure classification. This is a fixed algorithm subset, never a user-selected subset. Classify book roles first; an empty eligible set completes without a request. No other category or unclassified book is sent. Use only the saved title and full plain-text description, with the same 120 KiB evidence and 128 KiB encoded request bounds; never truncate the description or infer from unavailable book contents.

Use the [Responses API with Structured Outputs](https://developers.openai.com/api/docs/guides/structured-outputs), fixed `POST /v1/responses` and owner-selected initial **GPT-6 Luna** (`gpt-6-luna`), with per-book escalation in §3.14. This is extraction of nullable integers rather than a fixed-choice Decision. Send `store:false`, nonstreaming requests, reasoning effort `none`, a 256-output-token limit and a strict object schema with required `min_players` and `max_players`, each a positive 32-bit integer or null, and no additional properties. Validate completed response/message status, refusal versus output text, field presence/types, bounds, min ≤ max when both exist, model and token accounting. Incomplete, malformed, mismatched, semantically invalid or oversized structured responses follow the model escalation policy in §3.14. A refusal is an unsuccessful book, not an unknown range, and preserves any prior valid result. Do not retain provider error/refusal bodies.

Count players excluding a dedicated GM. Explicit exact counts set both fields; explicit ranges set both endpoints; a one-sided bound leaves the other null. Solo adventures and a duet explicitly for one player plus a GM have one player. Character levels, optional pregenerated characters, chapters and play time do not establish a party size. Do not guess a conventional count from the game system, genre or prior knowledge. Conflicting or ambiguous evidence stays null. The fixed instructions and examples in `players_instructions.txt` define these rules and treat all book text as untrusted evidence. Both-null is a valid completed result and is skipped on subsequent default runs; the explicit reanalysis checkbox includes previous results and additional spend.

Enable [prompt caching](https://developers.openai.com/api/docs/guides/prompt-caching) for this Responses analysis only. Send a stable developer message containing instructions and useful calibration examples before the per-book user message. Mark the end of that developer content with one explicit cache breakpoint, use explicit mode, 30-minute TTL and the algorithm as the cache key. Leave changing book input unmarked. The prefix must meet the model's minimum cacheable length; token usage is provider-reported and cache hits are not guaranteed. Decisions classification/diagnostics keep their existing no-breakpoint cache policy. Preserve real input/cache-write/cache-read/output usage separately in backend history; omit per-run usage from the totals-only UI.

Reuse the OpenAI credential preflight, shared client, logical-CPU worker limit, bounded coordinator, exponential request retries with structured-output escalation (§3.14), longest shared cooldown, dispatch checkpoints, cancellation/drain and catalogue ownership from §3.6. Only the explicit start command may begin paid work. Both online algorithms use the existing dispatch/progress envelope and one SQLite coordinator; no transaction spans HTTP or cooldown waits. Preserve valid results during cancellation. New explicit default runs reselect unfinished gaps and skip all current results including nulls; startup never resumes paid work.

Schema 7 stores nullable `min_players` and `max_players` in `adventure_players` with target/product identity, algorithm/model/input hash/time/run provenance. Do not modify product records, archive bytes, weighted scores or primary-role results. Save result, attempt, usage, counters and analysis revision in one fenced transaction; validate unchanged evidence and Adventure eligibility at commit. Relevant title/description changes invalidate counts; deletion/invalidation of the prerequisite classification cascades them, and a role/algorithm change invalidates them when no longer applicable. Reclassifying the same unchanged Adventure or changing unrelated prices preserves counts. Migrations preserve all existing catalogue data and history, including older progress JSON.

Analysis displays cumulative saved results, remaining eligible books and known versus unknown ranges in its single totals block (§3.10). Preserve nullable bounds and provenance in SQLite; a saved null remains distinct from no analysis. No group browser or editable model/worker/scope settings are added.

### 3.8 Solo Adventure / Gamebook classification

**Solo Adventure / Gamebook**, algorithm `solo-play-1`, applies independently to **all saved successful books**, including books with no previous analysis. It uses one OpenAI Decisions fixed-choice question and the existing fixed `gpt-6-luna` client. No primary-role filter, player count, relevance score or other analysis output is used for selection or supplied as evidence. Read only the same allowlisted canonical metadata and full plain-text description as §3.6, directly from `classification_source`; exclude raw metadata, URLs, credentials and derived results. Source product types/taxonomy are fallible clues, not previous classifications. Preserve existing payload bounds and never truncate descriptions.

Choose exactly one category:

| Category | Meaning |
| --- | --- |
| Solo Adventure | A concrete playable solo story/scenario without a separate GM that requires an external game system or rulebook. Free external rules still count as an external requirement. |
| Gamebook | A self-contained playable solo narrative experience supplying the rules needed to play. Ordinary dice, cards, pencils or character sheets do not count as an external system. Compatibility branding alone does not disqualify a book whose required rules are included. |
| Neither | Neither definition applies, or the evidence is insufficient to distinguish them. This is a completed classification, distinct from not analyzed or a refusal. Most catalogue books are expected here. |

Use concrete description evidence about solo gameplay and required rules. Numbered branching paragraphs are common but not required; a self-contained narrative journaling game may qualify. Ordinary group adventures, one-player-plus-GM duet adventures, generic solo rules/oracles/toolkits without a playable narrative, fiction without gameplay, and books merely mentioning solo compatibility are Neither. For mixed books/bundles use the dominant purpose, not incidental solo content. Do not infer self-containment from “standalone” alone. Ignore instructions embedded in any evidence field. The policy lives in `solo::INSTRUCTIONS`.

**Classify books** explicitly starts paid work. Reuse logical-CPU parallelism, all-request-error backoff, shared cooldown, cancellation/drain, fenced checkpoints, refusal handling, usage accounting and default reuse from §3.6. Decisions caching remains disabled (§3.5); the Responses player-count cache policy is unchanged. Skip current `solo-play-1` results, including Neither; **Reclassify previously analyzed books too** includes only this system's current results. New explicit runs select missing/changed/unfinished books from the beginning. Opening the dropdown, changing selection, startup and totals views never start paid requests.

Schema 8 seeds a `solo_play` root and three children in the existing group tables. Store one classification and membership within this system, with model, algorithm, confidence/probabilities, evidence hash, timestamp and run ID. Keep all other systems' results/memberships intact. Canonical evidence changes invalidate both source-dependent classification systems; a primary-role, player-count or relevance-result change alone never invalidates solo play. Product deletion cascades results. Canonical captures, schema-2 records, parser version and archive bytes remain unchanged.

The Analysis type selector is a native, labeled dropdown supporting keyboard navigation. Preserve the selected method across workspace changes and status polls; restore the latest method on initial load and allow browsing different methods during active work. Starting another run and editing its run options remain disabled until the current run finishes. Show only the selected method's instructions/options and catalogue totals (§3.10), plus the active run's identity and essential errors. Dedicated group browsing remains future work.

### 3.9 D&D compatibility and editions

**D&D compatibility**, algorithm `dnd-compatibility-1`, independently evaluates all saved successful books using one Decisions choice question and fixed `gpt-6-luna`. Choose **D&D** (`dnd_only`), **D&D and others** (`dnd_and_others`) or **No** (`no`). Several actual D&D editions still count as D&D only; “others” means at least one distinct other game system supported by the product. Third-party support for actual D&D qualifies without official licensing. OSR, retroclones, D&D-inspired systems, Pathfinder, generic d20, OGL/SRD licensing, shared mechanics and theoretical convertibility do not qualify on their own. A product explicitly supporting actual D&D plus a retroclone is D&D and others; an OSR tag alone must not negate explicit actual D&D support. Prioritize concrete intended rules/compatibility in the full description over noisy taxonomy. Mere mentions, historical comparisons and fiction do not establish gameplay compatibility. Insufficient evidence maps to No, a completed result distinct from refusal or absence.

**D&D editions**, algorithm `dnd-editions-1`, is a separate explicit analysis with a fixed subset: current positive `dnd-compatibility-1` results (D&D or D&D and others). It performs one Decisions request per eligible book with **nine independent choice questions**, allowing multiple editions without a probability threshold. Each question chooses `supported`, `not_supported` or `uncertain`. The only editions are **0e, B/X, AD&D 1e, AD&D 2e, 3e, 3.5e, 4e, 5e 2014, 5e 2024**. Here 3e means original 3.0, never the whole third-edition family; 3.5-only does not imply 3e. Bare “5e” without distinguishing evidence leaves the two fifth-edition variants uncertain; do not infer both from backward compatibility or guess using observation/publication dates. Generic AD&D likewise does not establish both named editions. Do not map a retroclone or an unlisted edition to the nearest listed edition.

Store only supported editions as memberships. Save all nine typed assessments with confidence/distributions for provenance. A response with no supported editions is a completed unknown result, skipped by default; any refusal prevents applying the entire edition result and preserves the previous good result. Malformed answers follow shared API-error retries. Questions never depend on another answer within the same request; the prior compatibility result is used for eligibility only. Both analyses send the same allowlisted canonical metadata/full description as primary-role classification, with no earlier roles, scores, player counts, solo results or compatibility answer included in model evidence. Preserve encoded/evidence bounds without truncation, disabled Decisions caching, logical-CPU capacity, retries, cooldowns, cancellation and fencing from §3.6.

Schema 9 seeds independent `dnd` and `dnd_editions` group trees in the existing tables. Compatibility uses one category/membership. Edition result rows summarize `identified` versus `unknown`; identified results have one membership per supported edition under the identified subgroup, while unknown results have an unknown membership. The existing provenance JSON field holds nine structured assessments for edition results; its summary confidence is the minimum reported choice confidence, not a calibrated aggregate probability. Other systems keep their existing meaning and memberships. Save all memberships, result, checkpoint, usage and revision atomically. Verify current eligibility and unchanged evidence before commit. Losing positive D&D eligibility, changing its algorithm, removing its prerequisite or changing source evidence invalidates dependent editions. Switching between the two positive compatibility categories alone retains editions. New captures and product deletion invalidate cached catalogue totals. Existing canonical records, parser version, archives and all prior analyses remain unchanged by migration.

Edition counts/results can overlap; never count memberships as distinct books. Preserve supported editions and per-edition provenance, distinguishing a completed unknown result from not analyzed, for future data views.

### 3.10 Catalogue-wide analysis coverage

Directly below the Analysis dropdown, show **current saved results across all runs for the selected analysis**, not the latest run's processed count. Include total catalogue size, eligible books, completed/current saved results, remaining eligible books, category totals and an eligible-completion bar. Count saved null scores, null player/level ranges, empty locations, null dates, None, Neither, No, Any Setting and unknown-edition/setting results as completed. Refused, missing and invalidated results remain outstanding. For fixed subsets, show the eligible denominator plus the whole-catalogue size, and separately show books whose prerequisite classification is still missing. Do not call an eligible subset globally complete while prerequisite eligibility remains unknown. Edition and setting category totals overlap, but the completion count is distinct books.

Show exactly one analysis progress block: the selected method's catalogue totals. Remove the Latest Run Activity section and its processed/newly-saved/unsuccessful counts, timestamps, worker activity, request/retry counters and token-usage display. Preserve backend run history, checkpoints and usage accounting. Keep a compact active-analysis identity or actionable retry/failure notice in the totals block without a second progress summary. The primary Run analysis / Classify books / Analyze player counts / Analyze books button becomes Cancel analysis in the same DOM element and location while work is active. Restore the start label when it finishes. Do not move or auto-scroll away from that action after starting; disable duplicate cancellation while it is pending. Switching selected analyses during active work changes only the viewed totals/options; it must not start, stop or retarget that work. Cancellation remains available for the actual active analysis. Reclassification opt-in remains per method.

Publish live catalogue totals through the same cached analysis status/watch snapshot as committed progress, displayed on the existing one-second desktop snapshot cadence. Remove the separate five-second coverage timer. Initialize a compact fourteen-analysis baseline once at explicit run start, under the existing transaction/ownership protocol. Online commits read only the affected book’s indexed analysis contributions before and after the write, including prerequisite-trigger deletions; weighted batches read old scores alongside their bounded selection. Apply those deltas to a private totals copy and publish it only after a successful fenced commit. Reclassification replaces contributions instead of adding another completed book. Refusals, retries and rolled-back writes do not advance saved totals. No whole-catalogue recount runs for each result or UI frame. Keep these live totals transient; run history, schema and saved results are unchanged, and restart rebuilds from SQLite.

Retain the narrow read-only `analysis_overview` command for opening an idle Analysis workspace, observed capture/reprocessing/storage changes, completion reconciliation and explicit Refresh. Calculate compact indexed aggregates in one read snapshot and cache by catalogue identity/revision; never load product records/descriptions. While an analysis supplies live totals, use those snapshots for every method, including after tab switches, without further aggregate requests. Discard an older in-flight aggregate response if newer committed snapshot totals have arrived. Keep at most one aggregate request plus one coalesced refresh, use bounded catalogue read admission, skip reads while hidden, discard results after disposal and show refresh failures without stale completion claims. Snapshot/overview reads perform no writes, analysis execution or model/storefront requests.

### 3.11 D&D settings and source context

Add four separate Analysis choices: **D&D settings**, **D&D level range**, **D&D in-universe locations**, and **D&D in-universe date**. Each uses a built-in subset of saved books with current positive `dnd-compatibility-1` results (`dnd_only` or `dnd_and_others`). No user-selected subset, Adventure/edition prerequisite or automatic chaining. Send only the original allowlisted source metadata and full description from §3.6; compatibility controls eligibility but no earlier analysis output is supplied.

**Settings**, algorithm `dnd-settings-1`, uses fixed `gpt-6-luna` and one Decisions request with fourteen independent named-setting questions plus a setting-independence question. Each chooses `applies`, `not_applicable` or `uncertain`; no probability threshold chooses memberships. The named settings are:

| Key | Setting |
| --- | --- |
| `forgotten_realms` | Forgotten Realms — Toril |
| `greyhawk` | Greyhawk — Oerth |
| `blackmoor` | Blackmoor |
| `mystara` | Mystara / The Known World |
| `dragonlance` | Dragonlance — Krynn |
| `ravenloft` | Ravenloft |
| `dark_sun` | Dark Sun — Athas |
| `birthright` | Birthright — Aebrynis |
| `planescape` | Planescape |
| `spelljammer` | Spelljammer |
| `eberron` | Eberron |
| `nentir_vale` | Nentir Vale / Points of Light |
| `exandria` | Exandria |
| `al_qadim` | Al-Qadim — Zakhara, a subgroup of Forgotten Realms |

Require concrete intended setting content or unambiguously connected named places. Description evidence takes priority over noisy taxonomy. Rules editions, genre, shared monsters/deities, promotional comparisons, optional relocation advice and general convertibility alone do not establish a setting. Never map unlisted worlds to a listed world. Multiple settings may apply; Al-Qadim always adds Forgotten Realms membership and counts under both. A specific supported setting takes precedence over contradictory setting independence. Otherwise **Any Setting** (`any`) requires affirmative evidence of setting neutrality/general applicability; **Unknown** (`unknown`) means no allowed setting or setting independence can be established. Silence alone is Unknown. Any and Unknown are distinct completed results, never combined with named memberships.

Store named results as `identified` with memberships and all fifteen confidence/distribution assessments. Store Any/Unknown with their own category and membership. Summary confidence is the minimum question confidence, not a calibrated overall probability. Any refusal prevents applying the entire setting result and preserves prior good results. Decisions prompt caching stays disabled.

The remaining three analyses use strict Responses JSON starting on `gpt-6-luna`, `store:false`, no tools, initial reasoning effort `none`, and explicit 30-minute prompt caching at the stable instruction/example prefix only. Book evidence follows outside the breakpoint. Keep per-analysis cache keys and actual provider usage. Output limits are 256 tokens for levels, 4,096 for locations and 1,024 for dates. Incomplete or invalid responses follow shared API-error retries.

| Analysis / version | Saved result and inference rules |
| --- | --- |
| Level range / `dnd-levels-1` | Nullable integer `min_level` / `max_level`, each 0 through 2,147,483,647; ordered when both known. Intended player-character levels only, never party size, spell level, challenge rating or dungeon floor. A single stated level sets both bounds; one-sided evidence leaves the other null. Support explicit level zero and levels above twenty. Do not invent a 1–20 default or numeric tiers. |
| In-universe locations / `dnd-locations-1` | `locations`, an array of at most 64 supported place-name strings, each at most 160 Unicode characters. Lands, cities, regions, planes and landmarks qualify. Trim and deduplicate case-insensitively; do not add remembered parent geography or publication addresses. Missing evidence yields an empty array. |
| In-universe date / `dnd-date-1` | Nullable `in_universe_date` string, at most 1,024 Unicode characters. Preserve calendar names, eras, approximation and ranges instead of converting calendars. Exclude publication/copyright and edition years. Unsupported dates remain null. |

Schema 11 seeds setting groups and adds typed `dnd_context` results keyed by target/product/kind, with algorithm, model, evidence hash, calculation time and run provenance. Migration performs no backfill or model calls; canonical record schema 2, parser version, observations, archives and prior analyses remain unchanged. Fenced commits verify current positive D&D eligibility and unchanged evidence, then atomically save result, memberships where relevant, attempt, usage, revision and checkpoint. Source-evidence changes, losing/removing the D&D prerequisite or changing its algorithm invalidate all four dependent analyses. Switching between the two positive categories retains them; product deletion cascades results. Changes to unrelated prices/ratings or other analysis results retain them. Changing a setting alone does not invalidate levels, locations or dates.

Reuse logical-CPU workers, bounded admission, one SQLite coordinator, indefinite all-request-error exponential backoff, Retry-After, cancellation/drain and out-of-order recovery from §3.6. Local preflight/storage/ownership failures do not retry. Completed unknown/null/empty results are skipped by default; explicit per-method rerun includes them. Show live catalogue totals (§3.10), overlapping setting counts, known/unknown extraction totals and prerequisite coverage, with the same-position Start/Cancel action. Opening the tab or changing selection never starts paid work. Individual-book and group browsing remain deferred.

### 3.12 Shadowrun and Call of Cthulhu context

Add four separate explicit analyses: **Shadowrun in-universe location**, **Shadowrun in-universe date**, **CoC in-universe location**, and **CoC in-universe date**. Their fixed subsets come directly from canonical `rule_systems`, independently of every AI analysis. Shadowrun matches any string containing `shadowrun`, case-insensitively, including editions and Anarchy. Inspection of the existing Rule System labels found **Call of Cthulhu** and no CoC abbreviation. Match that label exactly after trimming and case folding; do not match Cthulhu Eternal alone or infer eligibility from title, description or another taxonomy. A book with both qualifying labels belongs to both systems. Missing/nontext rule metadata is excluded, with no AI prerequisite or automatic compatibility analysis.

Start with `gpt-6-luna` Responses and escalate invalid outputs per §3.14, with strict output schemas and stable instruction-prefix caching, following the source-only allowlist, bounded evidence, paid opt-in, logical-CPU workers, shared indefinite API-error retries, cancellation/drain and provenance rules in §3.11. No prior enrichment enters evidence. Each book uses one request per selected analysis; there is no automatic chaining. The four algorithms are `shadowrun-location-1`, `shadowrun-date-1`, `coc-location-1` and `coc-date-1`. Initial Luna/Terra responses allow 1,024 output tokens; Sol/Astra use the bounded reasoning budget in §3.14. Unknowns are completed and skipped by default; per-method reanalysis is explicit.

Locations are a **nullable string** named `location`, up to 1,024 Unicode characters, trimmed and without control characters. Preserve supported in-world names, including fictional places and alternate-world geography. Multiple supported places form one concise semicolon-separated string. Do not infer countries from publication language, invent parent geography, or confuse publisher addresses, promotional comparisons or optional relocation advice with the book's setting. No evidence yields null. Existing D&D location arrays are unchanged.

Dates use a typed range of `chrono::NaiveDate` calendar bounds, stored as `date_start` / `date_end` in canonical `YYYY-MM-DD` form. Bounds are **inclusive**, may independently be null, and must be valid ordered dates in years 0001–9999. No invented time of day or timezone. Save precision (`day`, `month`, `year`, `decade`, `range`, `era`, `unknown`) and basis (`explicit`, `modern`, `dark_ages`, `unknown`) with the range. Validate calendar dates, month lengths/leap years and consistency between precision and bounds. A single day has equal endpoints; months/years/decades cover the whole indicated period. One-sided or spanning intervals use range precision. Entirely unknown dates have both bounds null and unknown precision/basis.

| Source evidence | Saved inclusive bounds |
| --- | --- |
| `2084` | 2084-01-01 through 2084-12-31 |
| `1920s` | 1920-01-01 through 1929-12-31; the exclusive next-decade boundary is 1930-01-01 |
| `February 1924` | 1924-02-01 through 1924-02-29 |
| CoC `Modern`, `modern-day`, `contemporary`, `present day` without a more specific period | 1980-01-01 through 2030-12-31 |
| CoC `Dark Ages` without a more specific period | 0950-01-01 through 1050-12-31 |

The final two ranges are owner-approved, fixed application conventions, not moving dates or universal definitions. Chaosium describes its [Dark Ages setting around 1000 AD](https://www.chaosium.com/dark-ages/); the exact fallback endpoints are this application's policy. They apply only when the description actually establishes the playable era, never to phrases such as modern layout or dark-age art. The model returns the era basis with null bounds; Rust fills the fixed range deterministically. Explicit in-universe dates take precedence. Do not derive time from publication/copyright/edition years, general game knowledge, or incidental historical backstory. Unsupported eras and nonconvertible fictional calendars stay unknown. D&D dates remain strings.

Schema 12 adds `earth_scope`, a narrow indexed eligibility projection keyed by target/product/system with original product rowids, and separate `earth_context` results keyed by target/product/system/kind. Migration builds only that eligibility index from saved Rule System metadata; no analysis, model call, Search index or canonical rewrite occurs. Insert/update/delete triggers maintain eligibility and invalidate source-dependent results; reprocessing that changes classification evidence removes these results, whereas ratings, unrelated metadata and other analysis results do not. Result provenance includes model, algorithm, evidence hash, timestamp and run. Source and eligibility must still match at fenced commit. Preserve all previous analyses and archives.

Keyset admission uses `(system, document)`; no per-book scan of the full catalogue or new browse/query interface. Full eligible counts use the narrow scope index and result aggregates; live progress uses committed per-book deltas across all analyses. Show one selected-method total block, known/unknown counts, fixed Rule System scope and the same-position Start/Cancel control. Initial index construction runs under existing asynchronous, fenced bootstrap; Close awaits executing atomic database work. Opening Analysis and reading totals never contact OpenAI.

### 3.13 Shadowrun and Call of Cthulhu editions

Add **Shadowrun editions** and **CoC editions** as independent explicit dropdown analyses. Apply exactly the Rule System subsets in §3.12; no previous AI analysis is required. Use original allowlisted saved metadata and the full description only. Do not supply prior roles, editions, locations, dates or other enrichment. Algorithms are `shadowrun-editions-1` and `coc-editions-1`; group systems are `shadowrun_editions` and `coc_editions`.

Use fixed `gpt-6-luna` Decisions with one request per book and one independent supported / not_supported / uncertain choice per edition. CoC has seven choices, 1st through 7th edition. Shadowrun has eight: 1st through 6th edition, original Anarchy and Anarchy 2.0. Several editions may match; no positive choices yields a completed **Unknown** result. Save every assessment and positive membership atomically; a refusal applies no partial classification and preserves any prior result. This follows the [Decisions multiple-question contract](https://developers.openai.com/api/docs/guides/decisions#ask-multiple-questions). Caching stays explicitly disabled for Decisions, without changing cached Responses analyses.

Require concrete intended rules support. Do not infer compatibility from publication years, shared setting or assumed backward compatibility. A numbered printing or edition of a supplement is not automatically the game's rules edition. Bare Call of Cthulhu or Shadowrun cannot identify a numbered edition. Explicit fifth-edition CoC revisions named 5.x remain 5th edition; Fourth Edition Anniversary Shadowrun rules remain 4th edition. A rules-title use of Shadowrun: Sixth World identifies 6th edition, whereas a reference to the fictional Sixth World alone does not. Original Anarchy and [Anarchy 2.0](https://store.catalystgamelabs.com/blogs/news/shadowrun-anarchy-2-0-is-coming-to-kickstarter) are distinct rules editions; an ambiguous unversioned Anarchy reference stays uncertain rather than choosing a version by date.

Schema 13 seeds these group trees and adds source/Rule System invalidation without rewriting captures, existing enrichment, archives or history. Admission uses indexed `earth_scope` keysets with original product rowids. Fenced commits recheck source evidence and eligibility. Reuse the existing CPU-count workers, bounded admission, shared all-request-error retry/backoff, cancellation/drain, provenance and restart checkpoints. Unknowns are skipped by default; reclassification is an explicit per-method option. Edition changes must not invalidate locations/dates or unrelated analyses.

The single live totals block counts each eligible book once, including completed Unknowns. Edition category counts may overlap. Reuse the committed per-book delta mechanism and bounded idle aggregates across all sixteen methods. Keep Start/Cancel in the same element, allow viewing other methods while work runs, and never send paid requests on startup, selection or totals reads.

### 3.14 Escalation for invalid structured output

Every analysis using Responses Structured Outputs follows the fixed per-book model ladder **Luna 6 → Terra 6.1 → Sol 6.1 → Astra 6**: `gpt-6-luna`, `gpt-6.1-terra`, `gpt-6.1-sol`, `gpt-6-astra`. This covers Adventure player counts, D&D levels/locations/date, and Shadowrun/CoC location/date. Decisions classifications remain fixed to Luna with their existing retry behavior. No model chooser or configuration is added.

Each newly admitted book starts at Luna. Invalid JSON, mismatched/incomplete envelopes, wrong types/fields, oversized responses and domain-validation failures (such as a partial decade labeled decade) advance that book by one tier. Resend the original allowlisted source evidence, unchanged instructions and strict schema; never send another model's output or error body as evidence. HTTP errors, rate/capacity limits, network failures and timeouts retry the **current** tier indefinitely with the existing exponential backoff and shared Retry-After cooldown. A valid refusal is an unsuccessful result, not invalid output, and does not escalate. Local preflight/storage/ownership failures still stop and drain.

If Astra also returns invalid output, record the book unsuccessful and continue other books. Apply no rejected fields and preserve any prior valid result. A later explicit run retries missing results from Luna; current results are reused unless reanalysis is selected. Cancellation stops queued/escalated requests immediately and drains workers before ownership release. Never reset a book to Luna within a run after it has escalated; other books start independently at Luna. Startup/totals still make no paid requests.

Keep stable-prefix caching enabled for every tier; matching cache hits are model-specific and not guaranteed. Follow the [official model compatibility guidance](https://developers.openai.com/api/docs/guides/latest-model): Luna/Terra use `none` reasoning and their existing field-specific output limits; Sol/Astra use `low` reasoning with an 8,192-token output budget including reasoning. HTTP attempts have a 30-second total deadline on Luna/Terra and 120 seconds on Sol/Astra, all interruptible by cancellation; failed HTTP attempts retain the same tier. No tools, conversation history, storage or additional evidence are enabled.

Validate response identity against the requested model, permitting its dated snapshot suffix, and save the actual accepted model identifier in existing result provenance. Preserve the existing algorithm IDs and schema version 13: accepted fields and eligibility contracts are unchanged; no migrations or automatic reclassification. Account every dispatched retry in existing run history, and preserve valid provider-reported usage even when its output fails validation. Malformed/missing usage cannot be invented; dispatch counts still include those attempts. Persist bounded escalation/exhaustion metadata with backwards-compatible progress JSON, without storing raw rejected/provider content. Display the most recent escalation and exhausted-book feedback within the existing totals block, not a new activity/statistics panel.

### 3.15 Discovery analysis dashboard

The second main-menu destination is **Discovery**, with one **Analysis** tab. Present four cards with current saved whole-catalogue coverage, category totals and eligibility notes. Replace method selection with one **Run analyses** button and an unchecked **Rerun all analyses, including current results** checkbox. Run the existing methods in this fixed order:

1. Weighted relevance, across saved books.
2. Primary book role, across saved books.
3. Solo Adventure / Gamebook, across saved books independently of primary roles.
4. Adventure player counts, using the current Adventure subset after stage 2 has committed.

D&D compatibility and every system-specific method are excluded from this action even when their existing selection spans the whole catalogue. Do not change model prompts, evidence, eligibility, result versions, retry/escalation policy or worker capacity. There are no individual stage buttons or user-selected subsets. Explain that the three online methods use OpenAI credits and rerunning includes previously completed results.

Expose one narrow `start_discovery_analysis(rerun_all)` command. The Rust controller owns the full sequence under a single OS lock/SQLite generation ownership lifetime and the existing active-analysis gate. Each stage retains its existing durable analysis run/checkpoints/history. The frontend does not chain IPC calls or own scheduling. Reject overlapping crawls, reprocessing, storage checks or analyses, including stage transitions; keep the gate active until all workers/writes drain and ownership is released. Close and the same-position **Cancel analysis** button cancel the complete sequence, including backoff and later stages. Local credential preflight happens before starting the sequence; storage/ownership failures stop it. Per-book unsuccessful outcomes continue, preserve prior good results, and leave missing results eligible next explicit run.

Default execution skips current results for every stage, including saved unknowns. Weighted relevance adds a fixed `all_missing` run scope for skipping current-version score rows; it still selects from all canonical books. Historical/individual relevance starts retain their existing full-rerun behavior. Rerun all recomputes scores and passes the existing reclassification/reanalysis option to all three online methods. The Discovery sequence itself requires no migration and preserves algorithm identities, result storage and existing histories; current catalogue schema 16 includes OSR and Warhammer structures in §3.17–3.19. Transient sequence step/rerun/failure metadata is optional in snapshots, not a new persisted catalogue structure. A crash preserves completed stages/books; restart never starts paid work automatically, and the next explicit default run skips retained results.

Coverage cards use committed live overview deltas at the regular snapshot cadence. Keep them visible together with catalogue size, saved-result totals and the number of analyses up to date. Adventure eligibility may change as primary roles commit. During an active sequence show its current stage and processed fraction in the same summary area, so reruns have visible progress even when saved coverage is already complete. Animate bar width changes without inventing completion and honor reduced-motion preferences. Keep retry, escalation, unsuccessful-result and local failure notices here; do not restore a latest-run statistics panel. Permit navigation away and back without stopping work, losing rerun preferences or starting extra requests.

Reuse bounded cached aggregates only for visible idle loading, relevant catalogue changes and completion reconciliation. Coalesce overlapping reads, discard older responses after newer committed totals, skip hidden-workspace reads and never query aggregates on each progress frame. No analysis or paid request occurs on opening a menu/tab, reading totals or startup. Crawling continues to discover sitemaps as part of each new crawl; this workspace operates on saved book metadata only.

### 3.16 System analysis dashboards and prerequisite gate

The six system destinations each have one Analysis tab, current saved coverage cards and category totals, a single **Run analyses** button, and optional **Rerun all analyses, including current results**. There is no method selector or individual-stage action. Execute in the displayed order:

- **Dungeons & Dragons:** compatibility → editions → settings → level range → in-universe locations → in-universe date. Compatibility still evaluates all saved books; subsequent stages use current positive D&D compatibility, including classifications committed earlier in this sequence.
- **OSR:** system classification over its built-in metadata subset (§3.17), then per-system level ranges for current Adventures with positive OSR memberships (§3.18).
- **Shadowrun:** editions → in-universe locations → in-universe date, with the existing Rule System subset.
- **Call of Cthulhu:** editions → in-universe locations → in-universe date, with the existing Rule System subset.
- **Warhammer Fantasy / Warhammer 40k:** games and editions → in-universe locations → in-universe date, with family-specific Rule System candidates and confirmed-family context eligibility (§3.19).

**New prerequisite:** all four Discovery methods must have current results for every eligible saved book, with no unresolved prerequisite eligibility. Saved unknowns, None and Neither count as complete; a successful run-history entry alone does not. A zero-eligible method is complete, so an empty catalogue is vacuously complete. New captures and source invalidations may lock system starts again. Browsing existing results remains available while locked. Show a Discovery completion checklist, disable the system run action with an explanation, and provide **Open Discovery**. Fail closed when totals cannot be read. Never start Discovery automatically.

Enforce this gate for both `start_system_analysis(workspace, rerun_all)` and retained individual system-analysis commands. Check current coverage during preflight, then recheck under acquired catalogue ownership before starting a stage or sending paid requests. Hold that ownership for the entire sequence; no source or Discovery changes can race it. The shared active-analysis gate prevents overlapping mutators. This is a workflow prerequisite, not new model evidence: do not send previous enrichments or alter source-only inference, prompts, eligibility rules, algorithm versions or result storage.

Reuse the sequence lifecycle from §3.15: committed live totals and stage progress, same-position Cancel, default reuse including unknowns, explicit rerun, per-book unsuccessful continuation, shared backoff/escalation, immediate cancellation of pending requests/backoff, drain before ownership release and separate durable stage histories. Navigation never changes active work. Only the running workspace exposes Cancel; other workspaces identify the active analysis and disable their starts. Transient sequence metadata adds a workspace discriminator (old values default to Discovery); OSR systems/levels were added through schema 15 (§3.17–3.18), and Warhammer groups/context use schema 16 (§3.19).

Selecting a system changes the whole shell, including header, flyout, typography, backgrounds and dashboard surfaces. D&D uses warm parchment/gold accents, serif headings, ornamental borders and a d20 icon. OSR uses black ink, white graph paper, monochrome controls, typewriter headings and a dungeon-map icon. Shadowrun uses navy/blue/cyan, grid and circuit motifs, angular panels, monospaced headings and a circuit icon. Call of Cthulhu uses dark greens, muted jade, gothic arches, serif headings and an eldritch/tentacle icon. Warhammer Fantasy uses burgundy/brass chronicle styling and a hammer/comet emblem; Warhammer 40k uses dark iron/red imperial-gothic styling and an armored skull/shield emblem. Crawlers and Discovery restore the original palette. Use local CSS/SVG assets, clear text labels, visible keyboard focus, scrollable responsive views and reduced-motion support. Theme follows the viewed workspace, independently of any running sequence; switching preserves drafts and rerun preferences. No remote fonts/assets or browser-driven work is introduced.

### 3.17 OSR systems

Place **OSR** in the main flyout between **Dungeons & Dragons** and **Shadowrun**, with one Analysis tab, the shared Run analyses/Cancel control, optional rerun-all and live saved totals. The whole shell uses a black-and-white graph-paper design: light paper with fine/major square grid lines, black ink, squared panels, typewriter headings and a local dungeon-map SVG. Maintain readable contrast, keyboard focus, reduced-motion support and the existing other-workspace themes. OSR runs system classification followed by Adventure level ranges (§3.18).

**Eligibility is metadata-based, not catalogue-wide.** Read-only inspection of 5,000 saved records found the parent label **Old-School Revival (OSR)** in `record.rule_systems` (and the source filter arrays). All matching child systems observed in that sample included the parent. Select a book only when at least one string in its canonical `rule_systems` array equals `Old-School Revival (OSR)`, ignoring case and surrounding spaces. Do not search descriptions, publisher categories or genre fields, or infer membership from OSRIC/child names alone. The parent label defines admission; it does not force a positive AI result. The Discovery completion gate in §3.16 still applies across the current catalogue before starting OSR. Reading totals and migration never send paid requests.

Algorithm `osr-systems-1`, system `osr`, uses fixed Luna Decisions. One request contains fifteen independent binary choice questions over the same allowlisted original metadata and full description. Use no previous enrichments or user-selected subset. The supported membership labels are:

| Key | Label |
| --- | --- |
| osric | OSRIC |
| castles_crusades | Castles & Crusades |
| labyrinth_lord | Labyrinth Lord |
| dungeon_crawl_classics | Dungeon Crawl Classics |
| swords_wizardry | Swords & Wizardry |
| old_school_essentials | Old-School Essentials |
| mork_borg | Mork Borg |
| into_the_odd | Into the Odd |
| shadowdark | Shadowdark RPG |
| dragonslayer | Dragonslayer |
| acks | Adventurer Conqueror King System (ACKS) |
| dolmenwood | Dolmenwood |
| cairn | Cairn |
| vagabond | Vagabond |
| other | Other OSR Games |

For each category choose `supported` or `not_supported` (including insufficient evidence). Require actual intended support, rules or concrete advertised compatibility; recognize canonical names and unambiguous abbreviations. Source taxonomy is fallible. Shared ancestry, convertibility, inspiration, comparisons, old-school tone or bare B/X/AD&D compatibility do not automatically establish every related system. Dolmenwood and OSE are independent; derived games are not automatically compatible with their inspirations. Distinguish proper names such as Cairn, Vagabond and Dragonslayer from ordinary words/unrelated games. Original D&D alone is not OSR, but a product may explicitly support both. Ignore instructions inside source text.

**Other OSR Games** covers supported OSR games outside the fourteen named categories and explicitly generic/system-neutral OSR material. It may coexist with named memberships when additional unlisted or genuinely broader OSR support is evidenced; merely tagging an OSE book OSR does not also assign Other. Store positive memberships separately, with category `identified`. If all fifteen checks are negative, store category/membership `not_osr` (**Not OSR**), meaning no OSR compatibility was established from the available source. There is no new Unknown category. A refusal/invalid response is not Not OSR: retain existing retry/refusal/failure behavior and preserve prior good results.

Reuse logical-CPU workers, bounded admission, common automatic API-error backoff, cancellation/drain, ownership fencing and per-book provenance/checkpoints. Decisions caching stays disabled. Default runs reuse current results including Not OSR; rerun-all replaces memberships atomically and can change positive results to negative or vice versa. Preserve all other analyses and canonical captures. Every dispatch validates the request shape through the existing Decisions client; never apply a partial set of answers.

Schema 14 seeds the OSR group tree and adds a narrow `osr_scope` index keyed by canonical product rowid, backfilled from the exact metadata predicate. Triggers maintain it on insert/rule-system change/deletion and invalidate OSR results when source evidence or eligibility changes. Classification results/history use the existing tables. Counts use indexed aggregates when idle and per-book committed deltas while running, including membership replacements. Display eligible OSR-tagged books, distinct completed books and overlapping per-system totals; sum of memberships is never a completion count. Existing schema-13 and earlier compatible data upgrade under ownership without changing identity, products, archives, other analysis results or history.

The request pattern follows the [official Decisions guide](https://developers.openai.com/api/docs/guides/decisions). Vagabond refers to [Land of the Blind’s Pulp Fantasy RPG](https://landoftheblind.myshopify.com/products/vagabond-pulp-fantasy-rpg-core-rulebook).

### 3.18 OSR Adventure level ranges

Add **Adventure level ranges** as the second OSR dashboard stage, after OSR system classification. Algorithm `osr-levels-1` selects only canonical OSR-parent-tagged books (§3.17) with a current Primary Book Role of **Adventure**, a current positive `osr-systems-1` classification and at least one supported system membership. Supplements, Core Rulebooks, unclassified roles, Not OSR and books outside the metadata subset are excluded. Keep the Discovery completion gate, one run-all/rerun-all action, same-position Cancel, live committed totals and default reuse. Missing prerequisite results are counted separately; a failed classification cannot create level eligibility. Existing current positive memberships remain usable if a reclassification refuses to replace them.

Use cached Luna **Responses** with strict structured output, one request per eligible book covering all its supported systems. Reuse logical-CPU workers, bounded admission, shared request-error backoff and the Luna → Terra → Sol → Astra invalid-output ladder. The user message contains original allowlisted metadata/full description under `book`, plus `requested_systems` routing keys. These keys constrain which systems require answers; they are not evidence of a level range. Never send previous levels, player counts, scores, D&D results or other enrichment. The fixed instruction/example prefix is cacheable; per-book evidence/routing is not marked as a cache breakpoint. This uses the existing [Responses structured-output contract](https://developers.openai.com/api/docs/guides/structured-outputs), without changing older analyses.

Return `ranges`, at most 64 entries, each with `system`, nullable `game`, `status`, nullable integer `min_level` and `max_level`. Each requested named system needs exactly one entry, with `game: null`; unrequested or duplicate named systems are invalid. Bounds are nonnegative, preserve level zero and explicit values above twenty, and must be ordered when both exist. A precise level sets both bounds; a one-sided statement retains one null endpoint. `known` requires at least one bound. `unknown` means the source does not establish a numeric range; both bounds are null. `not_applicable` means the source explicitly establishes level-free play/no numbered character levels for that system; both bounds are null. Absence of a range or assumptions about a game's usual rules are not enough for not_applicable. Unknown and not_applicable are complete reusable results, distinct from refusals/failures.

Never convert scales or copy one system's range to another. Share a range only when the original description explicitly or unambiguously applies it to those versions. Preserve system-specific differences, such as OSE 3–5 versus a DCC level-zero funnel. Do not infer levels from player counts, page counts, dungeon floors, hit dice, spells, publication/edition numbers or vague difficulty/rank labels. Unsupported/conflicting level scales within one named system remain unknown instead of being merged into a false range.

**Other OSR Games is not a rules system.** If requested, return separate `other` entries with the original source's unlisted game names and their own ranges/statuses. Explicitly generic material may use `game: "System-neutral OSR"` only when the source establishes that applicability. If no unlisted game can be identified, use exactly one `other` entry with null game, unknown status and null bounds. Never combine that placeholder with named entries or merge unrelated unlisted games. Reject duplicate game names case-insensitively, empty/oversized/control-containing names, invalid fields, missing systems and partial answers. A refusal or exhausted invalid response preserves a prior valid result; the whole book's output commits atomically.

Schema 15 adds a per-book `osr_levels` completion/provenance row and typed `osr_level_ranges` children keyed by book/system/game. Admission uses existing indexed `osr_scope.document` keysets and current classifications/memberships; no new catalogue-wide projection or source rewrite. Source, role, OSR classification and membership changes invalidate dependent ranges. Identical OSR reclassification preserves unchanged memberships and levels, so a subsequent cancelled/refused level rerun retains the prior good result. Recheck the original evidence and exact requested system set at fenced commit; invalidation and replacements participate in the common committed coverage deltas. Keep result usage/history and existing data/archives intact. Migration creates no analysis results and sends no paid requests.

The second card shows eligible Adventures and distinct analyzed books, plus books with numeric, unknown and level-free system results. Those status counts may overlap for a book with several systems; completion always counts the book once. All-system unknown and level-free outputs are complete. The first OSR card keeps its metadata-subset classification totals. The summary counts saved results across both stages and reports smooth sequence progress without recounting on each result or adding a data browser.

### 3.19 Warhammer Fantasy and Warhammer 40k

Add adjacent **Warhammer Fantasy** and **Warhammer 40k** flyout destinations after Call of Cthulhu. Each has one Analysis dashboard, three ordered stages (games/editions → locations → date), the existing Discovery-completion gate, run-all/rerun-all and same-position Cancel. Use burgundy/brass fantasy styling for Fantasy and dark iron/red imperial-gothic styling for 40k, with local original SVG emblems. Keep the menu scrollable and keyboard focus contained across every destination. Navigation loads saved totals only.

Use saved **Rule System** metadata as the candidate filter, ignoring case and surrounding whitespace. Observed labels are `Warhammer`, `Warhammer Fantasy Roleplay` and `Warhammer 40k Roleplay`. The generic `Warhammer` label enters both candidate sets; the classifier must confirm the actual RPG family before context extraction. Fantasy also recognizes explicit WFRP labels (including space-separated edition suffixes), Warhammer Fantasy, Warhammer Fantasy Roleplaying Game, named Warhammer Old World RPG labels, Soulbound and named Age of Sigmar: Soulbound variants. 40k recognizes Warhammer 40k/40,000/40000 (with optional Roleplay), the requested named RPGs, Dark Heresy edition suffixes, and Wrath & Glory / Wrath and Glory aliases. Migration 016 is the exact finite alias/prefix contract. Titles, descriptions, publisher names and unrelated taxonomy alone never add candidates; bare `Old World` does not qualify. Missing metadata remains outside these analyses.

One uncached Luna Decisions request independently checks family support and every supported game/edition. Fantasy categories: **WFRP 1e, 2e, 3e, 4e, 5e; Warhammer: The Old World Roleplaying Game; Warhammer Age of Sigmar: Soulbound**. 40k categories: **Dark Heresy (original/1e), Dark Heresy 2nd Edition, Rogue Trader, Only War, Deathwatch, Black Crusade, Imperium Maledictum, Wrath and Glory**. Multiple memberships are allowed. Use original canonical metadata/full description only. Distinguish RPG support from miniatures rules, fiction, shared lore, occupations and incidental mentions. An Old World setting mention does not imply the separate Old World RPG. General backward compatibility does not assign WFRP 4e material to 5e. Bare Dark Heresy need not establish which edition. Never infer editions from publication dates.

Save `identified` when any exact game is supported, `unknown` when family support is clear but the exact game/edition is not, `not_applicable` when outside this RPG family, and `uncertain` when family membership itself is unestablished. Unknown and uncertain are distinct completed results. An explicit negative family answer plus a supported game is an invalid response and uses shared request retry; refusal applies no partial result. Algorithms are `warhammer-fantasy-games-1` and `warhammer-40k-games-1`.

Locations/date stages select only current `identified` or `unknown` family classifications in the metadata subset. An uncertain or negative family does not qualify. Responses starts at cached Luna, using the shared instruction/example prefix and existing Luna → Terra → Sol → Astra escalation for invalid structured output. Requests contain original source evidence only; previous memberships/dates/locations are never input. Locations are a unique bounded array of up to 64 names of at most 160 characters, like D&D; [] is a completed unknown. Dates are nullable strings of at most 1,024 characters preserving in-world calendars, ranges, millennia, eras and uncertainty. Preserve `2512 IC`, `123.456.M41` or `shortly after the Necroquake` as supported, without Gregorian conversion or assumed default setting years. Publication/edition numbers and the 40,000 brand are not date evidence. Algorithms are `warhammer-fantasy-locations-1`, `warhammer-fantasy-date-1`, `warhammer-40k-locations-1`, `warhammer-40k-date-1`.

Schema 16 adds `warhammer_scope`, family group trees and `warhammer_context`, preserving existing identity, captures, archives, queue, results and history. Use indexed product-rowid keysets, bounded workers, request backoff and cancellation/drain; never hold transactions across HTTP. Recheck source and family eligibility at commit. Source or candidate-scope changes invalidate both dependent classification/context as appropriate; leaving a family invalidates only its context. A still-positive reclassification with unchanged evidence retains context. Reuse current results, including unknowns, by default. Display distinct candidate/classified totals and overlapping game counts; context totals use confirmed-family eligibility. Cached idle aggregates and committed per-book deltas drive progress, including dependent invalidation. The new game-specific tables in §3.20 read these saved results. No general Search, level analysis or automatic paid work is added.

### 3.20 Warhammer 40k game tables

Keep **Analysis** first and add eight named game tabs in classification order: **Dark Heresy**, **Dark Heresy 2nd Edition**, **Rogue Trader**, **Only War**, **Deathwatch**, **Black Crusade**, **Imperium Maledictum**, **Wrath and Glory**. No tabs for Unknown, Uncertain, Not applicable or aggregate status categories. Each tab lists current positive game memberships in the 40k metadata subset. A multi-game book appears once in each applicable tab. Current classification algorithm and family eligibility are required; browsing itself does not require Discovery completion and never invokes analysis or network requests.

Use a themed table with **Release Date, Name, Book Type, In Universe Date, In Universe Location**. Each heading toggles ascending/descending sorting of the entire selected category, not only the visible page. Default to Name ascending. Book Type is current Primary Book Role (Core Rulebook, Supplement, Adventure, None), separate from storefront product-type tags. Date and location use current saved 40k results; missing/unknown values render as a dash, and details distinguish uncalculated from saved unknown. Locations display the saved names separated by semicolons.

Release Date uses the source `date_available`, then `dates.dateAvailable`, then `dates.publicationDate`, accepting nonempty text and numeric publication years. Never substitute capture, file-modified or record-created timestamps. Preserve source precision in details; the table shortens ISO timestamps to their calendar date. Sort ISO release dates by their normalized calendar day, with year/month values ordered from their first day; unrecognized source date text stays text. In-universe dates remain calendar-preserving strings and sort lexically, without invented chronology. Names, book types, dates and joined location text use SQLite NOCASE ordering, with stable target/product-ID ties and missing values last in both directions.

Use fixed **50-row keyset pages** with Previous/Next and a count, preserving per-game sort/page choices. Fetch only the selected category through existing membership indexes, never a whole-catalogue browser projection or client-side whole-list sort. Narrow async `warhammer_books` and `warhammer_book` commands validate an enum game/sort and bind all user values; a cursor is tied to its game/sort/direction. Page count and rows share a read transaction. Reads use the existing bounded reader slots and may coexist with crawling/analysis; they do not claim mutating ownership. Refresh returns to the first page using current saved results. Pages are current reads rather than a durable cross-page snapshot during concurrent analysis changes. Ignore late UI replies after game/sort/view changes. Opening the app or another workspace does not preload these lists.

Selecting a row or its keyboard-accessible name opens an isolated detail view in the tab, replacing the table. Back restores its sort, page and focused book. Show the five columns plus saved full plain-text description, publisher, authors, pages, canonical Rule System/source product-type labels, current named 40k memberships and catalogue/capture identity. Preserve the raw release-date value in details. Bound detail payloads (262,144 description characters with a visible truncation notice; 100 labels per metadata array; bounded text fields). Render source content as escaped text, never HTML, and expose no raw response metadata, cookies, archive payloads, remote images or automatic external requests. No overlay, new Search/filter UI, edits, exports or paid work is introduced. Tabs wrap and the table scrolls horizontally at narrow widths; use labeled tabs/panels, arrow/Home/End navigation, visible focus and `aria-sort`.

This is a read-only addition: catalogue schema remains **16**, record schema **2** and parser `drivethru-rust-1`. No migration, backfill or analysis prompt/result changes are needed. Existing saved memberships and context become browsable immediately.

## 4. Fixed local storage

### 4.1 Location and startup

Use `<Userfolder>/.kaijucrawl` on Windows and Linux (`%USERPROFILE%\.kaijucrawl` on typical Windows installations, `~/.kaijucrawl` on Linux). Resolve the current user's home directory through the OS. Do not offer another path, backend, database connection, storage provider, bucket, endpoint or credentials for storage.

| Path under `.kaijucrawl` | Purpose |
| --- | --- |
| `settings.toml` | Target selection and browser/profile/container selection. Runtime policy is not user configuration. |
| `secrets.json` | Explicitly remembered manual website/API cookies; never catalogue metadata. |
| `catalogue.sqlite3` | Authoritative products, runs, jobs, provenance, outcomes and save intents. |
| `catalogue.sqlite3-wal`, `catalogue.sqlite3-shm` | SQLite-managed WAL companions when open. |
| `archives/` | Immutable gzip files for validated successful response bodies. |
| `archives/.staging/` | Temporary archive/check files, cleaned after work and on the next exclusive acquisition. |
| `catalogue.lock` | OS-managed exclusive writer lock; file existence alone is not ownership. |
| `webview/` | Tauri webview cache/profile; no cookie configuration is stored in frontend localStorage. |

Create missing directories/database automatically and apply versioned SQLite schema initialization. Open the dashboard immediately and run storage initialization on background tasks. Show loading until validation and catalogue totals complete, then enable Start/Reprocess. Failure preserves settings and shows a disk/permission/ownership error with actions to retry and inspect the fixed location. Pending checks disable work and duplicate checks without blocking navigation or Quit. Initialization never starts a crawl or contacts a database/server/object-storage service.

Use real local read/write/flush/delete checks with temporary files under the same directory, cleaning them after the check. Acquire catalogue ownership before schema or catalogue mutations. No transaction may span filesystem validation. Later starts revalidate local storage without a configuration wizard.

### 4.2 Settings and secrets

Write settings and remembered cookies through same-directory temporary files, flush them and atomically replace the destination. Keep cookie values out of settings, logs, errors, structured catalogue records and parser workers. On Unix use owner-only directories/files (0700/0600); on Windows inherit the current user's profile ACL. The local secrets file is not encrypted. Authentication offers session-only cookies or explicit remembering in this directory; browser-imported values remain in memory and the selected profile is reimported as before.

When the new files do not exist, carry forward only applicable settings and remembered cookies from the former application config/keyring. Discard old storage settings and cloud credentials; do not use database URL or provider environment variables. Existing local files take precedence. The compatibility keyring read is one-time; new credential writes go only to `secrets.json`. Do not modify browser files or delete previous user configuration automatically.

A catalogue is created only when none exists. Existing TUI catalogue/archive/settings/secrets files are opened in place. There is no remote catalogue/archive migration, legacy Python importer, configurable data-directory flag or cloud provisioning. Existing remote data is not modified. Back up the complete `.kaijucrawl` directory with the app stopped so the catalogue and archives remain together.

## 5. Authentication

### 5.1 Target-level menu

Authentication is configured per target. DriveThruRPG has website and API request scopes. Support:

- **No cookies:** attempt anonymously, with access limitations reported normally.
- **Paste cookies:** paste a raw `Cookie` header value, with an optional leading `Cookie:` label, for the website or API scope.
- **Import from browser:** find supported browsers/profiles for the current OS user and import relevant cookies from a selected profile.
- **Test Access:** verify the available access without modifying catalogue product/queue state.
- **Replace / clear:** replace or remove the target's configured credentials.

Reject embedded line breaks, other HTTP headers, malformed cookie input, and inputs above 1 MiB. A raw pasted header applies only to its explicitly selected host; do not infer parent-domain permission or share it across hosts/targets. Raw headers have no trustworthy expiry metadata, so report rejection when observed rather than inventing an expiration date.

Accept browser-produced legacy values containing printable ASCII quotes, commas and spaces, including literal JSON. Preserve pasted cookie values and order exactly after removing the optional label and outer whitespace. Continue rejecting control characters, non-ASCII/truncated values and invalid cookie names.

For imported cookies, preserve domain versus host-only scope, path, secure flag, expiry, and any applicable container/partition context. Send only cookies that match the actual request. Omit unsupported partitioned cookies with a clear diagnostic instead of flattening their scope. A website cookie is not automatically valid for `api.drivethrurpg.com`.

### 5.2 Browser import

The initial browser discovery list includes Firefox, Chrome, Edge, and Brave on Windows and Linux, where installed. Discover profiles from documented/known profile locations and profile manifests. The user selects the browser and profile; expose Firefox container selection when relevant. Do not mix accounts across profiles or containers.

Import only cookies matching the selected target's website/API domains. Read the browser's data without modifying it. Handle SQLite locking/WAL consistently through a safe snapshot or supported read mechanism. If access is unavailable, report the reason and offer manual paste. Do not kill the browser to obtain its files.

Do not filter by cookie name or HttpOnly flag: include eligible `cf_clearance`, other challenge/session cookies, and valid empty-valued cookies. Firefox import combines the cookie database with relevant cookies from its current saved session (`sessionstore-backups/recovery.jsonlz4` or clean-shutdown `sessionstore.jsonlz4`, whichever is newer). Never fall back to historical backup/upgrade sessions. Apply the same selected-container, partition, domain, path, secure and expiry restrictions to session cookies; current database values win when identities overlap. Accept an empty default `geckoViewSessionContextId`, but reject unsupported nonempty contexts. Bound compressed snapshot reads to 32 MiB and decoded data to 64 MiB; validate the declared size before allocation. Do not retain unrelated session data, copy browser snapshots to application storage or modify browser files. Keep combined imported cookie data within 1 MiB.

Report the number of additional session cookies or that only database cookies were available, with a value-free warning for unreadable, corrupt, oversized or unsupported snapshots. Cookies not yet saved by Firefox remain unavailable; manual paste of the full current request header is the fallback. Order imported cookies by descending path length, then ascending creation time where available. Firefox snapshots omit creation times, so preserve their relative order after known-time cookies of the same path length; do not promise exact browser ordering. Preserve import warnings in the access report, including an explicit warning when unsupported partition/context rules omit `cf_clearance`.

Support Firefox cookies whose `partitionKey` or `firstPartyDomain` identifies the adapter's own first-party site (`https://drivethrurpg.com`). Decode URL-encoded origin attributes; accept matching schemeful keys, their explicit default-port form, and older host-only keys narrowed to the adapter's HTTPS context. Retain the normalized first-party site on each cookie and check it again for every request; import must not turn a partitioned cookie into an unrestricted one. Website and API requests share the adapter's first-party site, while ordinary cookie domain/host/path/secure/expiry rules continue to apply. Preserve selected-container isolation. Reject different sites/schemes/ports, foreign-ancestor contexts, private contexts, malformed/duplicate attributes and unsupported attributes with a value-free reason. Report matching clearance as eligible with its first-party partition noted. Honor Firefox expiry units: seconds through cookie schema 15, milliseconds from schema 16.

Remember the chosen browser/profile/container locally, then reload matching cookies at the start of each run. Provide an explicit reimport action while paused. Imported cookies can remain in memory for the run; their values need not be duplicated into permanent application storage. Remembered manual cookies use the local secrets file described above.

Browser detection is not a guarantee of import support. Firefox profile import and Linux Chromium keyring integration are supported paths to implement and test. Windows Chromium-family app-bound encryption can prevent direct import. Unsupported encrypted profiles must fail gracefully; do not require elevation, browser injection, disabled encryption, or disabled security policies. Manual paste remains available. Do not advertise a browser/OS combination as working until it has been tested against an identified version.

### 5.3 Test Access and session replacement

Test website/API access using bounded, target-specific requests. Prefer a known discovered product; an optional product ID/URL inside this diagnostic dialog is permitted only for testing and must never enqueue a manual seed. Validate requested IDs and host scope using the normal adapter rules.

Show separate outcomes for cookies loaded, endpoint reachable, product response parseable, and authenticated access confirmed. A public HTTP 200 does not prove authentication. If no reliable account/session signal exists, report authentication as unverified rather than inventing confirmation.

Test website and API robots policies independently and show each exact robots URL and HTTP outcome. Show the number of cookies eligible for each request and whether `cf_clearance` is eligible, missing, or loaded but expired/out of scope; never display cookie values. Eligibility follows the same matching rules as request headers, including manual-header precedence. All requests, including robots, attach only matching cookies. Identify an explicit `cf-mitigated: challenge` response as a Cloudflare authentication challenge, even with HTTP 200. If either required policy cannot be established, do not request website/product endpoints; explain this dependency and offer profile/container review, reimport or exact-host manual paste. Browser login and clearance do not guarantee acceptance of this application's HTTP client; a challenge alone does not prove cookies were missing.

Do not infer a challenge from ordinary HTML containing CAPTCHA assets, Cloudflare background detection scripts, JavaScript translations, comments, hidden dialogs or product prose. An HTTP-200 body-based denial requires explicit evidence: a recognized gate title/heading, a challenge submission form, or a complete short gate message. Inspect at most 64 KiB for HTML evidence; ignore script/style/template/noscript content and explicitly hidden/inert elements. Report the evidence category without echoing response text. HTTP denial statuses, explicit JSON gate flags and the Cloudflare challenge header remain authoritative. A reachable ordinary homepage leaves account authentication unverified. Omitting the optional diagnostic product ID reports the API/product check as not tested and does not itself fail Test Access.

Test Access does not save a product, create a local archive, change success timestamps, mark crawl jobs complete, or start discovery. It replaces the old standalone `probe` operation.

When an active run loses authentication, pause that target and show the Authentication action. After cookies are replaced/reimported and access is retested, the user can resume the same run. Replace the credential snapshot atomically for new requests; never mix old and new cookies within one request. Interrupted/blocked work remains recoverable. Unselected future targets do not receive these credentials.

## 6. DriveThruRPG adapter

These constants reproduce the inspected legacy adapter. They are historical integration inputs, not a claim that current live responses have been verified during specification authoring.

| Property | Initial value |
| --- | --- |
| Target key | `drivethru` |
| Display name | DriveThruRPG |
| Website | `https://www.drivethrurpg.com/` |
| Root sitemap | `https://www.drivethrurpg.com/sitemap.xml` |
| Product API | `https://api.drivethrurpg.com/api/vBeta/products/{id}?groupId=1&siteId=10` |
| Detail Accept header | `application/json` |
| All request User-Agents | Fixed Firefox 158 compatibility profile; see §12.1. Internal robots evaluation retains the KaijuCrawl identity. |
| Interface language | `en`; product language comes from product metadata |
| Canonical product URL | `https://www.drivethrurpg.com/en/product/{id}` |
| Legacy image base | `https://d1vzi28wh99zvq.cloudfront.net/` |
| Legacy full-preview base | `https://watermark.drivethrurpg.com/` |

Accept discovered website URLs only for the DriveThruRPG apex and `www` hosts. Recognize locale-prefixed `/product/{id}` paths, optional slugs, and legacy `product_info.php?products_id={id}` links. Normalize IDs to positive decimal strings without leading zeroes; do not impose an arbitrary 32-bit limit. Preserve original discovered aliases in provenance while using a canonical queue identity.

Reject userinfo, unexpected ports, invalid IDs, and foreign hosts. Normalize product query/fragment aliases without creating duplicate tasks. Exclude account, checkout, login, wishlist, download, and non-product asset routes.

The adapter owns website/API hosts, routing, response unwrapping, taxonomy mappings, media resolution, and access classification. The scheduler, desktop UI, and persistence backend must not contain hard-coded DriveThruRPG routing or currency assumptions.

Legacy taxonomy roots are `10 → genre`, `20 → product_type`, `30 → rule_system`, `40 → language`, and `44498 → format`. These belong only to this adapter. Unknown values are retained without guessed classifications.

Before declaring live compatibility, validate the sitemap/detail route and representative payloads with authorized, bounded live tests. Do not silently change the site/group pair or substitute another storefront if routing stops working.

## 7. Fixed discovery and crawl lifecycle

### 7.1 New run

Starting a crawl creates a run ID after storage validation and acquisition of exclusive ownership. Snapshot selected targets and the automatic runtime policy. For each selected target:

1. Recover unfinished work from the previous owner/run.
2. Make all known, never-successful products eligible again, including prior failed, incomplete, restricted, missing, or interrupted products.
3. Reset per-run attempt budgets, while preserving lifetime attempt and outcome history.
4. Begin a fresh traversal of the root sitemap and allowed sitemap declarations from robots.txt.
5. Process existing eligible products and products discovered during traversal through bounded queues.

Fresh discovery is part of every new run; it is not a configurable mode. Product processing may overlap sitemap traversal. A user-requested resume from an authentication/storage pause remains the same run and must not repeatedly reset retry budgets or restart discovery.

### 7.2 Sitemaps

Support sitemap indexes, nested sitemaps, ordinary URL sets, and gzip payloads. Detect gzip from the payload rather than solely from the filename. Use streaming XML parsing with external entity/network resolution disabled. Read primary `loc` and `lastmod` children; image/alternate-language extensions do not create product tasks.

Follow only target-approved sitemap URLs and reject cycles/duplicates within the run. Persist discovery jobs with `(run_id, target_key, canonical_sitemap_url)` uniqueness. Insert product discoveries in bounded batches. Product task uniqueness is independent of the run so repeated entries cannot create repeated product downloads.

Record sitemap aliases, source URL, first/last discovery time, and supplied `lastmod`. A changed `lastmod` updates provenance only. It must not requeue an already successful product. A product absent from a later sitemap remains in the catalogue/queue; absence is not deletion.

Do not crawl API listings, category pages, recommendations, bundles, description links, or every numeric ID. Related-product IDs are metadata only.

Each run starts with fresh robots policy checks and sitemap retrieval/revalidation. A valid conditional response is acceptable only if its corresponding cached sitemap body remains available and is traversed for that run; an unavailable cache requires a full request. Never treat a bodyless 304 as an empty sitemap. No product refresh is performed.

### 7.3 Retry and finish rules

Product and sitemap jobs have at most five attempts within one run. Transient failures may retry with backoff during that run. Definite missing products, incomplete metadata, parser schema errors, and individual access restrictions (including ordinary product API HTTP 403) become unsuccessful for the current run rather than looping immediately. They become eligible again on the next run. Verified robots exclusions remain excluded for the current policy and are evaluated again on the next run.

The same failing product rediscovered in another sitemap must not regain its exhausted budget in that run. Pending unstarted claims are released on Stop. A request that actually started remains an attempt in run history; it must not be recorded as a successful fetch when interrupted.

A run finishes when discovery is terminal and all eligible product jobs are either durably successful or terminal for that run. Scheduled transient retries are still work; show the next retry time. Paused runs are not finished. Report discovery failures and unsuccessful products in the final dashboard summary.

There is no elapsed-time, product-count, or sitemap-count run limit. Resource bounds and cancellation still apply.

## 8. Product success and parsing

### 8.1 Success predicate

A **valid capture** is a successful detail response whose parsed product:

- Has exactly one verified identity matching the requested product ID.
- Has a nonempty normalized title.
- Has a nonempty normalized full `description_text`.
- Is not an authentication/challenge/age-gate response masquerading as a product.

A short description alone does not satisfy the full-description requirement. Other metadata fields are optional. A legitimate source product without a title or description remains incomplete under this deliberate rule and can be tried again on the next run.

A **durable success** additionally has a verified successful-response file in local archives and a committed SQLite product/archive/task transaction. Only durable successes are skipped forever by normal crawling.

Incomplete attempts retain only structured failure/status information, such as missing field names, parser version, HTTP status, and timestamps. Do not persist their response bodies or partial extracted product documents as successful catalogue records.

### 8.2 Response structure and identity

Support the legacy direct object, `product` wrapper, `data` object/list, and JSON:API attributes/relationships envelopes. Examine direct product candidates; do not substitute a recommendation, related record, or arbitrary deeply nested object. Resolve explicit included publisher/creator relationships where supported. Match JSON:API relationships by both resource type and ID when type is supplied.

Reject conflicting IDs, multiple matching product candidates, wrong-product responses, and ID-only envelopes with no recognizable product fields. Preserve the accepted source object and original envelope in raw metadata. New shapes require fixtures and an explicit adapter/parser update, not recursive guessing.

The initial product path is API-only. The legacy unused HTML/JSON-LD product-page parser is not a required runtime backend. HTML parsing is still needed for product description cleanup and link/media extraction.

### 8.3 Normalization

Keep cleaned HTML and readable plain text separately. Remove script/style/noscript/template/iframe/object/embed elements, event-handler and `srcdoc` attributes, and dangerous schemes in relevant URL attributes. Preserve inline word continuity and paragraph/list/table boundaries. Use bounded traversal; do not introduce quadratic repeated tree rewriting.

This cleanup is not a promise that arbitrary source HTML is safe for every downstream renderer. Preserve original source descriptions within successful raw metadata and the archived response. The desktop UI displays text and never executes HTML. Strip terminal control sequences from displayed/logged source text without altering archived response bytes.

Normalize missing values to null or empty collections as appropriate; do not fabricate descriptions, creators, dates, ratings, or page counts. Preserve zero prices. Reject booleans/nonfinite values as numbers and fractional page/file-count values as integers. Preserve source date representations alongside UTC capture/reprocessing timestamps.

Use exact decimal handling for monetary fields and lossless decimal TEXT/JSON representation in SQLite. Do not route prices through binary floating point. Preserve source JSON numeric precision in raw metadata. This changes numeric implementation while retaining every legacy field and its meaning.

For DriveThruRPG, explicit returned currency takes precedence. Preserve the legacy fallback to USD only as an adapter-specific `frontend_base_currency` assumption, with `currency_source` recorded. Do not generalize that assumption to future targets. Headline price is usable special price when `onSale` is true, otherwise base price; preserve the original sale flag.

## 9. Metadata contract

All legacy extracted fields below remain available in the structured record. Additional lifecycle fields may be added without deleting source information. Nested groups use JSON text in SQLite; identity, common filters, quality, and timestamps must remain queryable without reading local archives. The public record uses target-scoped product IDs represented as decimal strings.

### 9.1 Identity, provenance, descriptions, and creators

| Fields | Required meaning |
| --- | --- |
| `target_key`, `product_id` | Storefront-scoped identity; required. |
| `title`, `slug`, `product_url` | Plain title, selected source slug, and canonical product link, with slug where available. |
| `source_url`, `source_kind`, `url`, `api_url` | Parser provenance, `public_api` source kind, canonical queue URL, and actual permitted detail response URL. |
| `publisher`, `publisher_id`, `publisher_metadata` | Publisher name, explicit identifier, and complete source publisher object/value. |
| `description_html`, `description_text`, `short_description` | Cleaned full markup, readable full text, and separate short text. |
| `authors`, `artists`, `editors`, `contributors` | Ordered deduplicated role-specific name lists. |
| `credits` | Combined `{role, name}` entries without crawling creator profiles. |
| `page_count`, `isbn`, `sku`, `edition` | Page count and explicit product identifiers/edition. |

### 9.2 Prices, formats, dates, and taxonomy

| Fields | Required meaning |
| --- | --- |
| `price`, `base_price`, `special_price`, `on_sale` | Selected headline, ordinary/sale amounts, and returned sale flag. |
| `currency`, `currency_source` | Currency and explicit/fallback provenance; no currency conversion. |
| `lowest_prices` | `digital`, `print`, and `any` values from the corresponding API lowest-price fields. |
| `offers` | One entry per option: `id`, `format`, `option_name`, `type`, `price`, `currency`, `price_to_add`, `msrp`, `enabled`, and `raw`. |
| `formats` | Ordered deduplicated option format labels. |
| `dates` | All returned fields starting with `date`, plus `fileLastModified` and `publicationDate`. |
| `date_created`, `date_modified`, `date_available`, `file_last_modified` | Convenience fields preserving the source values. |
| `categories`, `filters`, `primary_filters` | Entries preserving `id`, `name`, `kind`, `group`, `parent_id`, `ancestor_ids`, `slug`, `localized_labels`, and `raw` where supplied. Primary filters come from `storefrontPrimaryFilterValues`. |
| `filter_groups` | Labels grouped by the adapter's verified hierarchy/root mappings. |
| `genres`, `themes`, `rule_systems`, `product_types`, `languages` | Grouped taxonomy and explicit labels. |
| `language` | Comma-separated convenience representation of `languages`; never inferred from `/en/` in the URL. |

Select explicit taxonomy names first; otherwise prefer supplied English labels, then the first available localized label. Preserve all supplied localized labels and unknown filter information. Hierarchy determines root membership; a localized display label is not a reliable root identifier.

### 9.3 Ratings, files, media, and related products

| Fields | Required meaning |
| --- | --- |
| `rating`, `rating_count`, `review_count`, `review_ratings` | Rating and separate rating/review counts, plus full rating buckets. |
| `reviews`, `ranking` | Embedded returned review and ranking data; no separate review crawl. |
| `files` | Entries preserving `filename`, `title`, `size_bytes`, `size_mb_reported`, and `raw`. |
| `file_size_reported` | Product-level reported file size, kept independently of the file list. |
| `images`, `cover_image` | Main images, thumbnails, associated images, description images, and selected cover link. |
| `previews`, `description_links` | Quick/full/custom/publisher/audio/video preview references and description links with labels. |
| `is_bundle`, `bundle_id`, `related_products` | Bundle values and direct relationships from bundle, related, recommendation, and also-purchased source fields. |
| `flags` | All legacy flags listed below, preserving returned values. |

If explicit `ratingCount` is absent and all five nonnegative integer rating buckets exist, their sum supplies `rating_count`. Do not substitute `review_count`. A zero rating with zero ratings becomes null.

Image entries retain kind, URL, alt text, and raw associated-image objects where applicable. Cover selection prefers `webImage`, then `image`. Resolve known relative image/preview paths using the adapter bases; retain absolute HTTP(S) references. Description links/images resolve against the website. Media references are not downloaded.

Related-product entries preserve `product_id`, `title`, `relation`, `url`, and `raw`. They never enqueue discovery work.

Retain these flags when returned: `status`, `enabled`, `workflowStatus`, `isAudio`, `isVideo`, `isPwyw`, `isCommunityContent`, `isBundle`, `scannedPDF`, `watermarked`, `disableCp`, `disablePrint`, `giftCertificate`, `ai`, `handmade`, `isGiftCert`, `isAi`, `isHandmade`, and `contentRating`. They are source values, not independently verified classifications.

### 9.4 Quality, source preservation, and lifecycle

| Fields | Required meaning |
| --- | --- |
| `parser_version`, `record_schema_version` | Versions used to derive the current record. |
| `extraction_status`, `missing_fields` | Successful records are complete with no missing title/description. Failed attempt diagnostics retain missing-field information separately. |
| `fetched_at`, `last_checked_at`, `reparsed_at` | Original network observation/check time and optional latest successful local reprocessing time. Sitemap sightings do not advance network-check timestamps. |
| `http` | Selected response status/validators only; exclude Cookie, Authorization, and Set-Cookie. |
| `raw_metadata.public_api` | The complete accepted decoded product object, including unknown fields. |
| `raw_metadata.api_response` | Original decoded envelope when different from the accepted product object. |
| `raw_path` | Relative local archive path, `archives/{key}`, resolved beneath `.kaijucrawl`. |
| `archive` | Relative key, SHA-256 content hash, decoded/compressed byte lengths, compression, and archive timestamp. No bucket or provider version ID. |
| `discovery` | First/last seen, source sitemaps, aliases, and sitemap modification hints. |
| `crawl_status` | Current durable task state, attempts, check time, HTTP status, and sanitized error. |

Keep `raw_metadata` as part of the full structured metadata contract even though exact response bytes are also archived in local archives. Removing that field or replacing it solely with a reference would require an explicit future contract change.

Preserve provenance when exposing the record through database views or the desktop UI. Do not rewrite `fetched_at` during reprocessing or claim a recent sitemap sighting is a recent product fetch.

## 10. SQLite persistence and ownership

### 10.1 Durable catalogue

Use bundled SQLite with foreign keys enabled, WAL journaling and FULL synchronous commits. Apply versioned migrations under exclusive ownership; preserve existing compatible data and reject unknown/newer schemas without resetting the catalogue. The catalogue schema is version 16; product records remain version 2 and the parser remains `drivethru-rust-1`. The version-2-to-3 migration adds scheduling/dashboard indexes. The version-3-to-4 migration adds a derived search projection, normalized facets, FTS5 text index and backfill checkpoint. The version-4-to-5 migration adds analysis run/results/revision tables and invalidation triggers, without a backfill or score calculation. The version-5-to-6 migration adds classification/group/run-checkpoint structures. Version 6-to-7 adds nullable Adventure player counts and invalidation triggers. Version 7-to-8 adds solo-play groups and extends canonical-evidence invalidation to that independent system. Version 8-to-9 adds D&D/edition groups, eligibility dependency triggers and capture-driven summary revision updates. Version 9-to-10 removes Search projections, FTS/facet tables, checkpoints and maintenance triggers. Version 10-to-11 adds D&D setting groups, typed context results and prerequisite invalidation. Version 11-to-12 adds indexed Shadowrun/CoC eligibility and typed location/calendar-range results. Version 12-to-13 adds Shadowrun/CoC edition groups and their invalidation triggers, reusing existing classification/result/history tables and the eligibility index. Version 13-to-14 adds OSR groups and indexed metadata eligibility with source/scope invalidation. Version 14-to-15 adds typed per-system OSR Adventure level results and prerequisite invalidation, without backfilling results. Version 15-to-16 adds Warhammer family/game groups, indexed Rule System eligibility and calendar-preserving context results with source/prerequisite invalidation. These migrations do not backfill results or call OpenAI. They do not rewrite canonical captures, queues, intents or archive references. The initial remote-only schema version 1 is not an on-disk SQLite migration input.

Apply migration 010 in the same fenced bootstrap transaction as the schema version change. Drop only retired Search objects; retain analysis tables, results, memberships, run history, attempts and original product rowids. Upgrades from earlier versions pass through historical schema migrations without performing a Search backfill. Register the legacy exact-decimal collation only for compatibility when opening/retiring older schemas. Do not vacuum or rewrite canonical products as part of the upgrade. Dropped index pages may be reused by SQLite; shrinking the database file is not required.

Initialization remains asynchronous and cancellable. Show unknown saved-book totals as loading, never as an empty catalogue; distinguish opening from a storage failure. Close cancels initialization after any executing atomic database work and releases ownership. Do not abandon an executing SQLite mutation. Navigation and Close remain reachable while storage opens; crawl and analysis starts wait for successful initialization.

Store a stable catalogue UUID, runs/settings snapshots, per-target cooldowns, product/sitemap jobs, discovery sightings, attempts, pending/finalized save intents, products with archive references and reprocessing outcomes. A catalogue view combines records with discovery and crawl status. Preserve `(target_key, product_id)` identity and sitemap uniqueness per run. Use parameterized SQL, bounded transactions and keyset pagination. Exact price values use decimal TEXT, not SQLite REAL/NUMERIC coercion; nested/raw metadata remains complete JSON text. Numeric price calculations require explicit exact-decimal handling rather than lexicographic TEXT ordering.

SQLite operations run through asynchronous connection workers with bounded connection counts and busy/acquisition waits. Transactions must never be held across HTTP, parser execution or archive-file I/O. Start a new run by marking unfinished predecessors interrupted and resetting only never-successful product jobs; retain lifetime attempts and sightings. Every crawl enqueues fresh sitemap discovery. Persist cooldowns across restarts.

Queue refills must use indexed eligibility, attempt-budget and pending-intent lookups instead of sorting/scanning the entire pending catalogue. Choose due work by due time and ID. In the initial single-target crawler, admit at most one active sitemap job, giving eligible discovery the next available slot and filling remaining slots with product work. A large pre-existing product backlog must not postpone all discovery until that backlog is exhausted, and website pacing must not occupy every worker with sitemap jobs.

Dashboard totals use covering indexes; finding the next due job and last successful save must not scan wide product records. Collect already completed tasks into bounded save batches before another refill/metrics cycle. Keep archive verification and FULL synchronous commits enabled; optimization must not count a capture before its durable commit or allow unbounded buffered work.

### 10.2 Single writer and stale operations

Hold an OS exclusive lock on `catalogue.lock` for the lifetime of mutating work. Another process/handle must fail promptly while it is held. Do not infer ownership from a PID or the presence of a lock file, delete a held lock file, or require a timeout after an OS-killed process. The OS releases the lock when its process/handles close.

Each acquisition also increments a persistent generation and creates a random owner token in SQLite. Mutating transactions use `BEGIN IMMEDIATE`, check the held lock plus owner/generation before mutations and before commit, and reject released/stale handles. Releasing invalidates all clones of that owner; a successor can acquire immediately. Read-only dashboard pools may remain after release. A new owner reconciles save intents and interrupted claims before starting network work.

Heartbeat and pre-request ownership verification check the held OS lock and persisted owner/generation without acquiring SQLite's writer reservation. This read-only verification does not replace the transactional fence on mutations.

## 11. Successful response archives and consistency

### 11.1 File layout and content

Always archive validated, identity-matched, complete successful product response bytes. Never persist failed, blocked, incomplete or malformed live response bodies. The archive path relative to `archives/` is `{catalogue_uuid}/{target_key}/{product_id}/{decoded_sha256}.json.gz`. Store that relative key, gzip encoding, decoded/compressed lengths, SHA-256 and archive time with the product. `raw_path` is `archives/{key}`, not an absolute machine-dependent path. Reject absolute/traversal keys and symlinked key components.

### 11.2 Recoverable save protocol

1. Fetch bounded bytes and verify identity, title and full description in the parser runtime.
2. Persist a save intent with capture metadata, immutable relative key/hash/lengths and original observation time, without response bytes in SQLite.
3. Write gzip bytes to a temporary file under `archives/.staging` on the same filesystem, flush it, and atomically publish without replacing an existing archive. Flush newly created directories on platforms that support directory synchronization. Verify the complete decoded archive hash and lengths. Existing matching bytes are idempotent; corruption is an error and is never overwritten silently.
4. Verify the archive before opening the metadata transaction. Commit the product, archive reference, finalized intent and successful job transition together under ownership. Publish success counters only after commit.

SQLite and archive files do not share an atomic transaction. Keyset recovery checks pending intents: a verified archive is reparsed/finalized without storefront traffic; an absent archive restores eligibility for never-successful work; corruption pauses storage recovery. A completed product cannot be downgraded or refetched by a stale pending intent. Incomplete temporary files cannot count as success. Staging must remain beneath the fixed data directory, be bounded by active work, and be cleaned on normal completion; crash leftovers are removed from the flat staging namespace when exclusive catalogue ownership is next acquired, before new work. They must never be interpreted as committed archives.

Reprocessing reads these same immutable local files, preserves observation times and archive references, and keeps the previous good record on extraction failure. It cannot contact storefronts or produce new raw archives.

## 12. HTTP behavior and failure handling

### 12.1 Request rules

Use asynchronous HTTPS with certificate verification. Keep website and API cookie scopes explicit. Validate every redirect; product requests retain the approved API origin, product path/ID, and `siteId=10`/`groupId=1`. Reject unexpected destinations and never forward raw cookies to another host.

Send the exact fixed User-Agent `Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:158.0) Gecko/20100101 Firefox/158.0` on all DriveThruRPG HTTP requests: website, sitemap, both robots origins, product API, retries and approved redirects. This owner-approved compatibility profile is the same on Windows and Linux and is not configurable; it does not execute or detect Firefox. It addresses observed User-Agent-dependent HTTP 502 responses without changing transport, cookie scope, challenge handling or retries.

Keep `KaijuCrawl/{application_version}` as a separate internal robots identity. Evaluate every policy's permissions and origin pacing with that identity, regardless of the wire header. Do not select a more permissive browser group. New run snapshots record both `user_agent` and `robots_agent`; historical snapshots remain unchanged. Cookies apply only to the selected target and matching URL. Do not silently use ambient `.netrc` credentials, unrelated cookies or accumulated Set-Cookie values instead of configured authentication.

Evaluate robots.txt for the relevant origin with a maintained standards-aware parser. Refresh policy at the start of a run and when required after access changes. Honor applicable disallow, crawl delay, request-rate constraints, and server cooldowns. Verified exclusion is recorded distinctly from an unavailable policy. A missing robots.txt response (404/410) permits requests; an authentication/challenge/error policy response must not silently become allow-all.

Robots `Crawl-delay` and `Request-rate` apply only to requests to the origin that supplied that policy (scheme, host and port). Keep website and API robots pacing separate: a five-second website delay must not slow API requests whose own policy has no delay. Waiting for one origin must not block the other's pacing. Requests, retries, robots refreshes and redirects use the destination origin's limiter; refresh replaces that origin's delay, including reductions/removal, while retaining previous request-start times. This does not relax robots exclusions, automatic concurrency bounds or server cooldowns.

Do not add a request-start interval beyond the applicable robots policy. Bound target HTTP concurrency by the available logical CPU count, with one worker if detection fails. Future targets receive independent limiters even when they use the same host; do not introduce a global host-wide limiter contrary to INV-10. Server-requested waiting still applies across discovery, product requests, retries and redirects for the target that received it.

### 12.2 Outcomes

| Condition | Required behavior |
| --- | --- |
| Complete matching product and both storage commits succeed | Mark successful; never fetch again during normal crawling. |
| Transport failure, HTTP 408/425/429, or 5xx | Retry within the per-run attempt budget with backoff/cooldown; then record unsuccessful. |
| HTTP 404/410 | Record missing for this run; eligible again next run while never successful. |
| Missing title/description | Record incomplete and missing fields; discard body; eligible next run. |
| Malformed/unsupported/wrong-ID product payload | Record parser/schema failure; discard body; eligible next run. |
| Parser deadline exceeded | Terminate/replace isolated worker, record timeout, discard body; continue other jobs. |
| Explicit individual product age/content restriction | Record restricted for this run; do not archive; continue other products. |
| HTTP 401, invalid configured credentials, clear session expiry, or browser challenge | Pause the affected target; offer Authentication/Test Access. |
| Product API HTTP 403 without explicit authentication/session/challenge evidence | Record a restricted product failure for this run; discard its body and continue other products. Generic access denied, disabled and age-restricted responses follow this rule. Do not retry in this run; automatically retry next run if never successful. |
| Website or robots HTTP 403 | Retain authentication/policy failure handling; the product API exception does not establish a missing robots policy. |
| Verified robots disallow | Exclude that job under the current policy; reevaluate next run. |
| Robots policy cannot be established | Retry transient transport errors within budget; otherwise pause the target with the policy problem visible. |
| Sitemap XML/structure failure | Retry transient retrieval failures; record persistent parse failure for the run; do not label discovery complete without errors. |
| SQLite/filesystem operation failure or lost catalogue ownership | Pause the whole crawl and stop new upstream requests; reconcile/revalidate before resuming. |

Use bounded exponential retry delay starting at 10 seconds and capped at one hour, with small jitter. Honor a longer valid Retry-After duration/date. HTTP 429 imposes at least a 60-second target cooldown. Persist due times so restarting does not bypass a known future server cooldown. The max-attempts value counts actual job attempts, not hidden retries added independently by multiple layers.

Do not retain the legacy automatic five-parser-error stop threshold as a hidden user workflow. Product failures remain visible, bounded per run, and retriable on the next run. Authentication, policy, and storage pauses follow the explicit rules above.

## 13. Automatic runtime policy

There is no Advanced menu. Download and parser capacity each equal the logical CPU count available to the process as reported by the operating system, falling back to one if detection fails. Effective concurrency may be lower because of available work, robots limits or storage backpressure. Normal requests have no artificial spacing; robots pacing, transient retry backoff and server-requested cooldowns still apply.

Classification independently uses the same logical-CPU capacity policy, without added pacing or configurable workers. Its API failure/cancellation rules are in §3.5–3.6; storefront robots policies do not apply to Decisions. Weighted relevance remains a sequential local batch calculation.

The remaining resource bounds are internal policy, not editable desktop UI, file, environment or command-line settings:

| Area | Policy | Meaning |
| --- | --- | --- |
| Network | Logical-CPU download workers per target | Maximum concurrent target HTTP requests. |
| Network | Connect 15 s, read inactivity 45 s, total response 300 s | Separate bounds on connection establishment, stalled reads and the complete response. |
| Network | Five attempts per job per run | Bounded transient retries as described in section 12; ordinary product API 403 has no immediate retry. |
| Network | Fixed Firefox 158 request header; `KaijuCrawl/{version}` robots identity | Separate transport and policy identities, as specified in §12.1. |
| Parsing | Logical-CPU parser capacity; isolated processes | Hard deadline/cancellation kills and reaps the assigned process. |
| Parsing | 30 s product parse deadline | Includes starting an assigned worker, excludes waiting for a parser slot. |
| Classification | Logical-CPU request workers; one SQLite coordinator | Bounded evidence/tasks and out-of-order results; no whole-catalogue work queue. |
| Decisions | Connect 10 s, total 30 s per attempt | Classification retries all API-request failures with shared exponential cooldown; local preflight/storage/ownership failures still stop. Diagnostic and transport remain single-attempt. |
| Shutdown | 5 s drain grace | Stop/Quit/interrupt lets active work finish; a second interrupt ends grace early. |
| Database | Claim batches of 20; save batches of 10 | Bounded reservations and transactions. |
| Database | 500 ms partial-batch flush interval | Submission bound for pending saves, not a network pacing interval; storage latency/backpressure may extend completion. |

Former `[advanced]` settings and attempted `[runtime]` overrides are ignored on load, including obsolete worker counts, delays and User-Agent overrides. The next settings save removes them. Target/browser choices and remembered authentication are preserved. New run snapshots record the internal policy under `runtime`; historical snapshots remain unchanged. This does not change the product record or catalogue schema.

Do not restore discovery switches, seed inputs, refresh/retry switches, run limits, response-retention switches, exports, SQLite cache controls or machine-readable operational output.

## 14. Reprocessing after parser updates

Reprocess saved responses is a separate maintenance action for applying a newer parser to already successful captures. It requires the same validated storage and exclusive catalogue ownership as crawling. It cannot run concurrently with a crawl.

The desktop UI shows the current parser version and how many saved records have a different version. Process eligible saved captures in keyset batches and persist a checkpoint/progress record. Re-running an interrupted operation skips records already successfully processed with the intended parser version. Parser behavior changes must increment the parser version.

For each record:

1. Read its referenced successful response from local archives.
2. Verify object integrity and product identity.
3. Apply the current parser using the same bounds as live extraction.
4. If the resulting record still satisfies title + description completeness, atomically replace derived metadata and update parser/schema version and `reparsed_at`.
5. Preserve original capture/check timestamps, target/product identity, discovery provenance, archive bytes/reference, and crawl success state.

If the new parser fails, times out, produces incomplete data, or mismatches identity, preserve the previous complete record. Record the reprocessing failure separately and show it in the desktop UI. Do not delete the raw archive, overwrite a good record with a partial one, or turn parser maintenance into a network refresh. Missing/corrupt archives are storage failures requiring attention.

No new raw response archive is produced by reprocessing. Existing captures stay unchanged. Full version history of derived records is not required initially; the current record, original source, and reprocessing outcome/provenance are required.

## 15. Stopping and recovery

Stop, application/window close, and handled Ctrl+C enter one shutdown path:

1. Stop new scheduling and request starts immediately.
2. Continue ownership checks while draining work that can complete safely within the five-second grace period.
3. Flush already validated, archive-verified product batches when storage is available.
4. Cancel outstanding asynchronous requests and terminate/reap remaining isolated parser workers at the deadline.
5. Release unstarted reservations, preserve pending save intents, mark the run interrupted/stopped where possible, and release ownership after work is quiescent.
6. Close the desktop window and exit, or return to idle for Stop.

Repeated close/stop requests are idempotent; they do not start another shutdown or bypass persistence cleanup. SQLite pool acquisition and busy waits have finite limits. Archive work uses bounded data on blocking workers; the OS cannot guarantee cancellation of an already executing filesystem operation, so shutdown does not promise a hard deadline for hung disk I/O. If storage is unavailable, do not pretend unsaved candidates were committed. Already durable claims/intents and OS lock release permit next-start recovery.

An OS kill, power loss, or forced process termination may prevent orderly cleanup. Recovery must not depend on finalizers running: OS lock release, fenced claims, successful-record checks, and idempotent save-intent reconciliation are authoritative.

Default isolated parsing must have enforceable termination. Cooperative thread parsing is explicitly weaker and cannot promise hard cancellation of a noncooperating library call. Do not describe a timeout around a running blocking task as killing that task. A last-resort process exit must preserve local recoverability.

No background daemon remains after the user closes kaijucrawl. Credentials may need replacement after restart; authenticated session continuity is not guaranteed by durable queue state.

## 16. Rust implementation structure

Use a stable Rust toolchain, document the minimum supported Rust version when implementation begins, and commit `Cargo.lock` for the executable. Choose maintained libraries after checking their current APIs and Windows/Linux support; this specification does not pin unverified crate versions.

| Boundary | Responsibility |
| --- | --- |
| Desktop/controller | Bundled frontend views, narrow Tauri IPC, secret-free snapshots, configuration operations and graceful window shutdown. |
| Domain | Target/product identity, records, success predicate, outcomes, run/attempt state, versioning. |
| Scheduler | Fixed discovery lifecycle, bounded claims, target rate limits, cancellation, retry and pause rules. |
| Storefront adapter | Target-approved URLs, sitemap product recognition, API request construction, parsing and classifications. |
| HTTP/auth | Async transport, robots policy, redirect checks, cookie scopes, browser import providers, redaction. |
| Parser runtime | Pure extraction inputs/outputs and isolated worker or cooperative thread execution. |
| Catalogue store | Transactional jobs, products, intents, ownership fencing, metrics, and reprocessing checkpoints. |
| Archive store | Local immutable file writes/reads, storage checks, integrity, and immutable keys. |

An initial module layout can follow `app`, `controller`, `desktop`, `ui/`, `domain`, `scheduler`, `targets/drivethru`, `http`, `auth`, `parser`, `storage/sqlite`, `storage/archive`, and `telemetry`, with SQL migrations and sanitized fixtures in separate directories. This is guidance, not a requirement to create many crates before useful behavior exists.

Keep storage interfaces focused on domain operations such as claiming jobs and committing captures. SQLite transaction/fencing logic belongs inside its backend; the scheduler must not assemble ad hoc SQL. Archive persistence remains a separate interface because it has different consistency and failure behavior.

Use bounded async channels/semaphores between download, parsing, archive writes, persistence, and desktop UI event delivery. HTTP clients and database pools are reused. Do not spawn an unbounded task per discovered product, hold database transactions open across network/filesystem operations, or run CPU-heavy parsing on the desktop UI/async executor thread.

An isolated parser worker can be the same executable launched through an internal protocol. It receives only bounded body data, target/expected identity, and parser parameters; it does not receive authentication secrets or perform network requests. Cap input/output size, detect worker crashes, enforce the deadline, and replace failed workers. This internal worker protocol is not a public user-facing crawling mode.

Rust asynchronous cancellation does not automatically stop CPU work. In particular, Tokio documents that an already running `spawn_blocking` task cannot be aborted. Preserve process isolation for a hard deadline, or explicitly use cooperative checks in thread mode. See the technical references in section 20.

## 17. Diagnostics and resource bounds

Initial internal bounds inherited where useful from the old crawler:

| Resource | Initial bound/behavior |
| --- | --- |
| Product response | 16 MiB decoded body; fail clearly above the bound. |
| Sitemap | 128 MiB decoded data per sitemap; streaming XML/gzip processing. |
| robots.txt | 1 MiB decoded body. |
| Cookie paste/import result | 1 MiB relevant cookie material per configured scope. |
| Redirects | At most 8 normal redirects and 6 robots redirects; validate each destination. |
| Discovery writes | Bounded batches of at most 1,000 discovered entries. |
| Reprocessing scan | Initial keyset page size 100. |
| Recent desktop UI events | Bounded ring buffer; no unbounded per-request render history. |
| Body/parser staging | Bounded by active work and maximum sizes; clean on completion, failure, and next startup. |

Apply both encoded-input and decoded-output bounds so compression does not bypass limits. These are per-resource safeguards, not user-configurable run quotas. Whole-run memory must stay bounded independently of catalogue size. Use backpressure when SQLite, local archives, or parsing cannot keep up.

Store useful attempt information: target, product/sitemap identity, stage, classified outcome, attempt number, timestamps, HTTP status, parser version, and sanitized reason. Keep response bodies only in the successful local archive and successful structured raw metadata. Do not log cookie values, authentication headers, SQL credentials, browser databases, or full copied HTML/error bodies.

Local rotating logs may assist debugging; the authoritative progress/outcome state is in SQLite. The dashboard must remain useful without opening log files. Diagnostics are neither an export feature nor a machine-readable stdout mode.

## 18. Acceptance criteria

Every invariant requires meaningful tests. The following scenarios define the initial release boundary.

| ID | Scenario and expected result |
| --- | --- |
| AC-01 | First launch opens the dashboard without a storage wizard. Local initialization is asynchronous and enables work only after success. |
| AC-02 | A new fixed data directory initializes its SQLite catalogue and archive folder automatically; local validation files are cleaned. No remote storage requests or credentials are used. |
| AC-03 | Only DriveThruRPG is selectable; an empty selection disables Start. |
| AC-04 | Every new run scans root/nested sitemaps; indexes, gzip, cycles, duplicates, malformed XML, and foreign-host entries are handled correctly. |
| AC-05 | Repeated locale/slug/query aliases produce one target-scoped product job while retaining provenance. |
| AC-06 | A previously successful product is never fetched again, including changed `lastmod`, later discovery, and subsequent runs. |
| AC-07 | A failed/incomplete/missing/interrupted product is eligible next run; duplicate discoveries cannot reset its current-run budget. |
| AC-08 | Each job has finite attempts; Retry-After/429 cooldowns survive restart and the desktop UI shows waiting status. |
| AC-09 | Matching ID + title + full description is required. Missing optional metadata does not fail an otherwise complete product. |
| AC-10 | Full metadata fixtures exercise every field group, zero prices, exact decimals, rating buckets, taxonomy, media references, flags, and unknown raw fields. |
| AC-11 | Wrong IDs, ambiguous wrappers, nested recommendations, challenges, malformed JSON, and incomplete records never become successful products or durable body archives. |
| AC-12 | Successful response bytes round-trip through gzip/local archives with the same SHA-256; product rows refer to the correct immutable object. |
| AC-13 | Failures/crashes before and after intent creation, archive publication, and SQLite commit recover without duplicate success, stale writes, or lost successful records. |
| AC-14 | Two application instances compete for ownership; only one can mutate. Released handles and successor generations fence the previous writer. |
| AC-15 | Rate limits are independent per target. A fake second target sharing a host demonstrates this without shipping another storefront adapter. |
| AC-16 | Cookie domain/path/secure/expiry and host-only rules survive import; profile/container identities remain separate; redirected requests never leak credentials. |
| AC-17 | Browser/profile detection and manual paste work with fixtures; inaccessible/keyring-protected/app-bound cases offer a clear fallback and never crash the desktop UI. |
| AC-18 | Test Access performs no product/queue/archive mutations and does not infer authentication merely from a public HTTP 200. |
| AC-19 | Authentication failures pause the target; storage failure pauses all new crawling; replacing credentials/recovering storage resumes safely. |
| AC-20 | Reprocessing updates parser-derived metadata without storefront requests or changed capture times; a failing new parser preserves the prior complete record. |
| AC-21 | Parser hang/crash and forced shutdown leave the catalogue recoverable; process mode terminates/reaps workers within the implemented bound. |
| AC-22 | Stop/window close/interrupt shuts down work, releases ownership safely (or the OS releases its lock on process death), and recovers from the local catalogue and archives. |
| AC-23 | Dashboard remains interactive during slow network/storage operations and does not claim absolute storefront completeness. |
| AC-24 | Windows/Linux desktop builds and available native interaction checks are reported accurately; browser-import support claims identify actually tested browser/OS versions. |
| AC-25 | No old refresh, seed, retry-mode, run-limit, no-raw, export, or JSON-output controls reappear in the desktop UI or public operational CLI. |
| AC-26 | The Authentication wizard validates steps, preserves answers when navigating Back, skips irrelevant questions, masks secrets in reviews, and retains drafts after failed tests. Choices and navigation remain reachable at 760×560; color supplements text and selection markers. |
| AC-27 | Removed: database parameters/connection-string configuration. No equivalent setting or environment override remains. |
| AC-28 | Removed: bucket/endpoint/prefix binding and failed remote setup repair. The fixed local archive directory is not configurable. |
| AC-29 | Each launch initializes/checks fixed local storage, loads totals and enables Start without a wizard or automatic crawl. Failure permits a dashboard retry. |
| AC-30 | Every secondary view renders independently of dashboard metrics, events and shortcuts at 1240×820 and 760×560. Authentication results scroll and failed checks return to the retained draft. |
| AC-31 | Synthetic Firefox imports retain eligible clearance, HttpOnly and empty-valued cookies while respecting scope/context. Local robots requests receive only eligible cookies; diagnostics report both hosts, HTTP status, clearance eligibility and omissions without values or failure bodies. Challenges remain failures even with HTTP 200 and prevent product/website tests when robots policies are unavailable. |
| AC-32 | A Firefox clearance cookie partitioned to the adapter's own first-party site imports and reaches both website and API robots requests when its normal cookie scope matches. The normalized partition remains enforced at request time. Other sites, cross-site ancestor/private contexts, incorrect containers and malformed/ambiguous attributes never leak into requests. Firefox schema-specific expiry units are respected. |
| AC-33 | Firefox session-only cookies saved in its current snapshot supplement database cookies without overriding current values or relaxing scope. Old backups are never used; corrupt/oversized snapshots fail safely with manual-paste guidance. Browser-produced JSON cookie values survive import, manual paste and actual HTTP serialization. Creation-time/path ordering is respected where metadata exists. |
| AC-34 | Ordinary HTTP-200 HTML with Cloudflare detection scripts, CAPTCHA assets, translations and hidden login dialogs remains reachable. Explicit HTML/text gates, HTTP 401/403, JSON gate flags and `cf-mitigated: challenge` still fail with value-free evidence; ordinary product API 403 is a per-product failure as specified in AC-39. Test Access without a product ID reports the API/product check as untested, preserves authentication as unverified and never starts discovery. |
| AC-35 | Configuration and remembered cookies reside under the fixed home directory, remain separate from catalogue data, and survive atomic updates. Migration carries forward applicable old settings/cookies only. |
| AC-36 | Two processes cannot mutate the local catalogue concurrently. Forced termination releases the OS lock immediately; successor generations reject stale handles and recover at each intent/file/commit boundary. |
| AC-37 | Website/API robots pacing is independent in both directions; a waiting origin does not hold up the other. Target-wide server cooldowns still apply, separate targets remain independent, refresh can remove an old origin delay, and waiting/cancelled work does not consume an attempt. Dashboard pacing reflects actual origin delays, automatic workers and cooldowns, with robots reasons and unknown policies shown accurately. |
| AC-38 | The Advanced menu and shortcut are removed. Download/parser workers equal the available logical CPU count (fallback one), with no added request spacing. Old or manually added runtime settings cannot override this policy and disappear on the next save; target/browser choices survive. |
| AC-39 | Product API HTTP 403 alone, generic access denied, disabled-product and age-restriction responses fail only that product for the current run. No failure body is archived, no immediate retry occurs, and a new run retries it and may save a successful response. Explicit session/login failures or challenge evidence still pause the target; website and required robots checks retain strict failure handling. |
| AC-40 | A synthetic catalogue with 350,000 product jobs and 6,000 captures supports bounded indexed claims, accurate dashboard totals, attempt budgets, cooldowns and pending-intent exclusions. Eligible discovery gets one slot despite the old product backlog, with remaining capacity available to products. Measure local scheduling and end-to-end durable throughput without treating fixture rates as live-service guarantees. |
| AC-41 | Opening a version-2 local catalogue applies version-3 indexes under ownership while preserving catalogue identity, captures, observation times, archive references, unfinished work and recovery intents. Record/parser versions remain unchanged; stale ownership is still rejected and read-only guards do not reserve the SQLite writer. |
| AC-42 | All request scopes and approved redirects send the exact Firefox compatibility header. Robots permissions and pacing use the separate KaijuCrawl identity even when browser groups differ; cookie, challenge and cooldown rules remain enforced. |
| AC-43 | The desktop controller opens an existing TUI-format catalogue in place with unchanged catalogue identity, records, timestamps, archive bytes/references, settings and cookies. Pending saves recover, successful products remain skipped, failed jobs regain eligibility and each new crawl discovers sitemaps. Startup performs no crawl. |
| AC-44 | Desktop operations remain responsive and serialized where required. Stop/Close prevent late starts; close waits for pending operations and drain/ownership release. Snapshots never reveal secrets. Frontend tests cover start gating, wizard Back/masking/failed checks, isolated views and safe text rendering. Production assets contain no synthetic preview or test controls. |
| AC-45 | Schema-9 catalogues upgrade to 10 with unchanged identity, canonical records/rowids, observation times, archives, unfinished crawl work, saved analyses, memberships and history. All Search tables, FTS shadow tables, views and maintenance triggers are removed, including a partially completed old backfill. |
| AC-46 | All analyses work without Search projections. Use bounded product-rowid keysets with unchanged checkpoint/attempt identities. Default classification reruns reuse saved results and select unfinished gaps, including previously unindexed books; fixed prerequisites and aggregate/live totals remain correct. |
| AC-47 | The closed-by-default flyout contains Crawlers, Discovery, Dungeons & Dragons, OSR, Shadowrun, Call of Cthulhu, Warhammer Fantasy and Warhammer 40k. Crawlers retains five accessible tabs; each analysis workspace has one Analysis tab. Tab/menu focus, dismissal, selection/panel labels, draft retention, startup and Close remain correct. The old Analysis dropdown is not mounted; system-specific code/results remain intact. Navigation starts no analysis; visible analysis workspaces may read saved totals. The authorized 40k tables/details in §3.20 are the only book browsing views. General Search/filters and retired Search IPC/index maintenance remain removed. |
| AC-48 | Older catalogues receive separate analysis/group structures without changing canonical data or archive identity and without calculating scores. The approved formula ranks 4.7/1,000 above 5.0/3, uses a fixed baseline, rejects unusable inputs, and persists version/input/time provenance independently. |
| AC-49 | Current desktop analyses use the whole catalogue or their fixed built-in eligibility scope. Weighted relevance commits at most 500 books per batch and supports cancellation/restart without automatic work. Failed batches roll back scores and progress together. Ownership loss rejects stale writes. Close awaits executing work and releases ownership; duplicate analysis, crawl and reprocess starts are rejected while analysis owns the catalogue. |
| AC-50 | Opening Analysis or reading totals does not calculate. UI tests cover explicit catalogue-wide submission, no subset controls, progress/cancel/errors, and selection retention. Changed rating inputs invalidate scores while unrelated or rolled-back changes preserve them. Completed score inputs/provenance remain stored separately. |
| AC-51 | Decisions loopback fixtures verify all three answer types, JSON/text encoding with caching disabled, typed choices, refusals, bounds, mismatches, credential separation, safe errors, redirects, no retries and cancellation. The explicit desktop check uses synthetic data only, rejects duplicates, survives tab switches without another request and stops on Close. UI tests report usage/errors without secrets. Ordinary tests make no live API calls; any authorized live check records request count and actual reported usage. The connection test performs no catalogue migration or group assignment. |
| AC-52 | Schema-5 catalogues receive classification structures while preserving captures, archive identity, saved scores and history. Classification uses versioned full-description/allowlisted metadata evidence, one primary category, separate results/memberships and provenance. Tests cover None versus refusal, oversized inputs without requests, source invalidation, unrelated updates/rollback, default reuse, forced reruns, atomic failure, interrupted dispatch recovery and stale-owner rejection. |
| AC-53 | Primary-role analysis operates catalogue-wide with no user subset controls. Loopback controller tests cover explicit paid starts, eligibility reuse, mutual exclusion, read-only totals, safe local stopping errors without retries, missing-key isolation and Close cancellation/release. UI tests cover algorithm choice, paid scope, reclassification opt-in and cumulative counts. Live validation, if authorized, uses bounded synthetic books and records actual requests/usage. |
| AC-54 | Loopback fixtures hold multiple requests open to prove CPU-sized bounded concurrency, immediate refill after out-of-order completion, unique admission during normal/forced runs, exact counters/usage and default reuse. Cancellation and local storage/ownership failures stop admission, drain every worker and preserve committed results before ownership release. Storage tests verify monotonic completion checkpoints, unfinished gaps eligible after takeover, duplicate-commit rejection and historical progress compatibility. The UI keeps cancellation and essential error feedback without per-run statistics; ordinary tests never contact OpenAI. |
| AC-55 | Loopback tests return 429 then 503 then success and verify identical evidence, increasing minimum delays, exact dispatched/retry/token counts and a single saved result. Test shared Retry-After cooldowns while siblings finish, immediate Cancel/Close during long waits, all other HTTP errors, dropped connections before/during response reads, timeouts, malformed/oversized responses, local storage failures stopping without retries, bounded queued books, safe header parsing, delay-cap/overflow behavior and interrupted retry checkpoints. UI displays waiting reason/countdown/retries without marking the run failed. Normal validation uses no live API credits. |
| AC-56 | Synthetic Responses fixtures verify initial Luna model, strict nullable integers, bounded input/output, shared-prefix caching before variable book input, accurate usage, refusals, null/partial/exact ranges and rejection of invalid/incomplete responses. The shared coordinator retries request failures and cancels/drains without duplicate results. Any authorized live check uses bounded fictional descriptions and records actual requests/usage. |
| AC-57 | Schema 6 upgrades to 7 preserving identity, canonical data and prior analyses. Only current Adventures are eligible. Test saved-null reuse, forced reanalysis, role/source invalidation, unrelated changes, atomic persistence failures, refusal preservation, interrupted gaps, duplicate commits and stale-writer rejection. No automatic API work occurs. |
| AC-58 | UI tests cover explicit Adventure-only starts, reanalysis, cumulative null/known totals and cancellation. Storage tests preserve nullable bounds/provenance, distinguish saved unknowns from missing results and enforce Adventure-only eligibility without Search. |
| AC-59 | Solo-play tests select all successful books regardless of previous analyses; actual request evidence excludes derived fields and raw metadata. Wire tests verify the fixed Decisions model, three choices and disabled caching, with retries of identical evidence, independent counters, explicit reuse and cancellation/drain. |
| AC-60 | Schema-7 fixtures preserve identity, records, archives and existing analyses through upgrade. Solo classifications/memberships coexist with primary roles; Neither is reused, refusals preserve prior good results, canonical changes invalidate dependent results, and other analysis changes do not. Atomic rollback, stale writers and unfinished gaps remain enforced. |
| AC-61 | Frontend tests cover the native analysis dropdown, selection persistence while another analysis runs, independent catalogue-wide solo starts/reclassification and cumulative category totals. Selecting a method does not dispatch a request. |
| AC-62 | D&D tests cover the three-way compatibility contract, the exact nine-edition question set, multiple supported editions, unknowns, refusals and source-only evidence. Conditional edition workers exclude No/unclassified books and reuse the shared all-request retry/cancel protocol. |
| AC-63 | Schema-8 fixtures preserve captures and prior results through upgrade. Tests verify independent memberships, atomic multi-edition rollback, prerequisite/source invalidation, unknown-result reuse, stale ownership, unfinished gaps and saved provenance. |
| AC-64 | Coverage tests use results from multiple runs, distinct-book totals and overlapping edition counts; validate fixed-subset prerequisites, cache invalidation and unchanged observations. Frontend tests switch selected analyses during active work, show exactly one cumulative totals block with no latest-run activity panel or run-statistics display, verify immediate committed snapshot updates, reject stale aggregate responses, preserve same-element Start/Cancel behavior and actionable errors, and recover from totals-refresh errors without model calls. Live totals must match authoritative aggregates through replacements, dependent invalidation, nulls, cancellation and rollback. |
| AC-65 | Settings selects only current positive D&D books; supports all fourteen named choices, multiple memberships, distinct Any Setting/Unknown, and Al-Qadim under Forgotten Realms. Specific settings override contradictory Any; refusal preserves prior results. Evidence excludes prior enrichment. |
| AC-66 | Three strict extractions preserve nullable level bounds (including zero and above twenty), empty locations and calendar-qualified dates. Reject wrong types, extra fields, reversed/negative bounds, oversized/control-bearing text and invalid provider envelopes. Unknowns are completed. Cache only stable instructions; start with Luna and apply §3.14. |
| AC-67 | Schema 10-to-11 fixtures preserve identity, captures and previous analyses. Verify reuse/rerun, positive D&D scope, source/prerequisite invalidation, unrelated-result retention, rollback, out-of-order recovery and stale-writer rejection. Live totals match authoritative aggregates after replacements and dependent deletions. |
| AC-68 | Loopback/frontend tests start all four new methods explicitly, retry invalid API responses, retain unknowns, cancel/drain before ownership release, and show one live totals block with same-element Start/Cancel and per-method rerun options. Startup, selection and totals make no paid requests. |
| AC-69 | Rule System fixtures include mixed-case Shadowrun editions/Anarchy, Call of Cthulhu, overlapping systems, unrelated Cthulhu Eternal, absent values and nonmatching abbreviations. Only the fixed metadata predicates select books; no earlier AI result or description-based eligibility. |
| AC-70 | Typed date tests cover full years/decades/months, leap days, single/open-ended ranges, unknowns, invalid/reversed bounds and inconsistent precision. CoC era bases normalize to approved fixed bounds; Shadowrun rejects those bases. Location strings retain supported names and reject wrong types, extra fields and oversized/control text. |
| AC-71 | Schema 11-to-12 upgrades preserve identity, observations and earlier analyses while building indexed scope with no model work. Verify source/eligibility invalidation, unrelated-result retention, unknown reuse, per-system isolation, rollback, out-of-order recovery, refusal preservation and ownership fencing. Live totals match saved aggregates. |
| AC-72 | Loopback and UI tests exercise four explicit starts, source-only evidence, cached instructions, retries of invalid date responses, cancellation/drain, per-method reruns and one totals block. No automatic paid requests; authorized live checks use only bounded synthetic evidence. |
| AC-73 | Shadowrun/CoC edition requests use the fixed Rule System subsets, source-only evidence and eight/seven independent Decisions choices. Multiple memberships and completed Unknowns persist; Anarchy and Anarchy 2.0 remain separate; partial refusals preserve prior results. |
| AC-74 | Schema-12 upgrades preserve identity, captures, scope rowids, location/date results and run history. Edition writes are fenced and atomic; source/scope changes invalidate them while ratings and unrelated analyses retain them. Default reuse, explicit reruns, out-of-order completion and stale-owner/rollback behavior are covered. |
| AC-75 | Both edition UI/loopback routes use existing workers, API retries and shutdown drain, source-only uncached Decisions requests and live whole-subset totals with overlapping edition counts. No paid calls occur on selection or reading coverage. |
| AC-76 | All eight structured contracts preserve evidence/schema/cache settings across Luna → Terra → Sol → Astra. Verify model-specific reasoning/budgets, accepted model provenance, invalid-envelope/semantic escalation and usage for rejected outputs. Decisions retries remain on Luna. |
| AC-77 | Loopback tests cover HTTP retries retaining the escalated tier, independent books, Astra exhaustion continuing the run, preservation of previous valid results, retrying missing results next run, cancellation/drain and accurate dispatch/retry/usage counts. Refusals do not escalate. |
| AC-78 | UI feedback uses only the existing totals block for last escalation/exhaustion, keeps same-position Start/Cancel and never starts paid work implicitly. Existing schema-13 results/history deserialize unchanged. |
| AC-79 | A single Discovery command executes relevance → primary role → solo play → Adventure players, with one ownership lifetime and no mutation gap. Loopback tests cover HTTP retries, newly classified Adventures, no system-specific requests, default reuse, forced rerun, per-book refusal continuing, local failure stopping, Cancel/Close drain and subsequent ownership acquisition. Missing credentials start no partial sequence. |
| AC-80 | The Discovery dashboard has four coverage cards, one run action, optional rerun-all and same-position Cancel. Tests cover navigation/draft retention, duplicate-start prevention, gating, live coverage/stage transitions, stale aggregate rejection, retry/escalation/terminal feedback and aggregate error recovery without paid work or per-frame recounts. |
| AC-81 | Default Discovery relevance retains current scores and saved unknowns, selects only missing/invalidated results with bounded rowid batches, and preserves their prior provenance. Forced rerun processes every selected book. No schema bump, capture/archive rewrite or automatic run on restart occurs. |
| AC-82 | Each system dashboard executes its specified sequence through one backend command and ownership lifetime, with optional rerun-all, default unknown reuse, live coverage and same-position cancellation. Tests verify stage order, positive D&D eligibility, no overlap, saved-result reuse, rerun cancellation and ownership release. |
| AC-83 | System sequences and every retained individual system-analysis command reject incomplete Discovery coverage, including pending Adventure eligibility. Current unknowns count as complete. Recheck after ownership acquisition; new/changed source evidence relocks the gate. No automatic Discovery run or paid prerequisite check occurs. |
| AC-84 | System navigation applies the matching whole-shell theme and local SVG icon, isolates its Analysis tab, preserves state and restores the default theme for Crawlers/Discovery. Gate feedback and Open Discovery are accessible; background workspace work continues without additional starts or per-frame aggregate queries. |
| AC-85 | OSR is between D&D and Shadowrun and uses a monochrome graph-paper shell. The shared dashboard shows distinct subset completion and overlapping categories, retains run-all/rerun/cancel and the Discovery gate, and starts no paid work on navigation. |
| AC-86 | Only canonical Rule System strings matching Old-School Revival (OSR), case-insensitively after trimming, enter OSR analysis. Tests reject misleading titles/genres, child names alone, missing metadata and partial labels. One Decisions request independently checks all fifteen categories, supports multiple/Other memberships, and distinguishes a complete negative from refusal/malformed output. |
| AC-87 | Schema-14 upgrade preserves identity, captures, archive references and existing results/history. Tests cover indexed keysets, default Not OSR reuse, overlapping live totals, atomic membership replacement/rollback, evidence and eligibility invalidation, out-of-order commits, stale writers, API retries, cancellation and exclusion before request dispatch. |
| AC-88 | OSR runs system classification then level ranges. Only current Adventures with positive OSR memberships in the exact metadata subset are admitted. Supplements, Not OSR and missing prerequisites make no level requests. Routing keys are separate from original source evidence. |
| AC-89 | Per-system structured ranges preserve independent scales, zero levels, open bounds, unknowns and explicitly level-free play; Other retains separate unlisted game names. Reject malformed/partial/duplicate/cross-scope results and escalate through the shared ladder. No automatic conversion or copying between systems. |
| AC-90 | Schema-15 upgrade preserves all prior data/results/history. Verify indexed keysets, atomic multi-system replacement/rollback, out-of-order commits, default unknown reuse, role/source/membership invalidation, identical-membership retention, stale writers, live distinct totals, retry/escalation/cancellation and the two-stage dashboard. |
| AC-91 | Both Warhammer flyout dashboards use their own whole-shell themes and three-stage run-all sequence, the Discovery gate, rerun/default reuse, same-position Cancel and live distinct totals. All eight menu destinations remain keyboard-accessible. Navigation sends no paid requests. |
| AC-92 | Only the documented Rule System candidates enter Warhammer game decisions. Generic Warhammer is checked for each family; multi-game results, family-confirmed unknown, uncertain and negative are distinct. Context selects confirmed families only and uses original evidence; strict cached Responses retain fictional date strings/unknowns and the shared escalation/retry policy. |
| AC-93 | Schema-15 upgrade preserves identity, canonical captures and prior analysis history/results. Tests cover indexed candidate keysets, source-only evidence, unknown reuse, out-of-order completion, live aggregate agreement, family-specific/source/scope invalidation, refused replacement, changed evidence and stale-owner rejection. |
| AC-94 | 40k retains Analysis and adds exactly eight named game tabs. Current multi-membership books appear in each appropriate table; unknown/uncertain/negative/stale classifications never create category rows. Browsing performs no writes, paid requests or catalogue-wide projection work. |
| AC-95 | All five columns sort globally in either direction with stable keyset pagination and missing values last. Tests cover page boundaries/ties, source date fallback, category/cursor validation, current enrichment and indexed membership lookup. |
| AC-96 | Row/name activation opens isolated escaped-text details. Back retains page/sort/focus; tab navigation, errors, empty/deleted results and late replies are handled. Existing analysis/crawl flows remain functional; schema and saved data are unchanged. |




## 19. Delivery sequence and validation

Implement in reviewable increments:

1. Domain contracts, desktop UI skeleton, local settings/secrets, and automatic fixed-directory initialization.
2. SQLite migrations, ownership fencing, queue/record operations, and local archives save-intent recovery with integration tests.
3. DriveThruRPG sitemap adapter, bounded HTTP transport, and fixed new-run eligibility rules.
4. Full product parser and isolated runtime, validated against sanitized golden fixtures covering the legacy metadata contract.
5. End-to-end crawl/dashboard, manual cookies, Test Access, failure pauses, and shutdown recovery.
6. Browser import providers and parser-update reprocessing; platform compatibility verification.

These increments do not redefine the release scope: the first release includes all accepted features, including best-effort browser import with documented fallbacks. Do not label an early increment complete merely because anonymous product fetching works.

Use unit tests for pure parsing/state rules and integration tests against disposable SQLite/filesystem-compatible services for transactional behavior. Use a controllable local HTTP server for robots, redirects, retries, discovery, auth classification, and interruption. Browser fixtures must be synthetic or sanitized. Do not check real sessions, credentials, account data, or full unreviewed browser profiles into this public repository.

Normal tests must not crawl live storefronts. Explicitly requested live validation is separate, narrowly scoped, rate-limited, and documented with what was actually observed. Repository historical tests and the previous Python implementation do not prove the Rust application or current storefront works.

Expected routine checks are formatting, Clippy, relevant unit tests, and affected integration/platform checks. Record skipped checks and their concrete reason. Documentation changes also require Markdown link, field/requirement coverage, and consistency checks. See [validation evidence](docs/VALIDATION.md) for checks actually executed.

## 20. Legacy mapping and sources

### 20.1 Deliberate changes from the old crawler

| Old interface/behavior | New behavior |
| --- | --- |
| `--data-dir`, local SQLite checkpoint/archives | Fixed home-directory SQLite catalogue and local archives; no data-directory option. |
| Separate storefront executables | One Rust application; only DriveThruRPG implemented initially. |
| `--discovery`, `--seed`, `--seed-file` | Fixed sitemap discovery on each run; no seeds or API listings. |
| `--retry-failed`, `--retry-discovery` | Automatic next-run eligibility and bounded current-run retry. |
| `--refresh-days`, `--refresh-discovery`, `lastmod` product requeue | No product refresh; discovery always runs and successful products stay skipped. |
| Cookie file CLI flags | desktop UI paste and best-effort browser import, with explicit host scope. |
| Workers/delay/timeouts/attempts/User-Agent | Automatic logical-CPU workers, no added spacing, fixed internal timeouts/attempts; fixed Firefox request header and separate KaijuCrawl robots identity in §12.1. Limits remain per target. |
| Parser concurrency/deadline/backend | Automatic logical-CPU capacity, fixed deadline and isolated processes with hard cancellation. |
| Shutdown timeout | Fixed five-second grace and a unified graceful-stop path. |
| Claim/save batching | Fixed bounded batches backed by SQLite transactions. |
| SQLite cache size | Removed. |
| Product/discovery run limits | Removed. |
| `--no-raw` | Removed; successful response archives are mandatory. |
| Partial products saved as catalogue products | Incomplete attempt metadata only; successful products require title + description. |
| `export`, `--output` | Removed. |
| `status --json`, `probe --json`, `--no-progress` | Desktop dashboard and isolated views; no initial machine-readable operational output. |
| `probe` | Test Access diagnostic action without catalogue/archive writes. |
| `reparse` | Parser-update maintenance using local archives; preserve the prior good record if reprocessing fails. |
| Local writer lock | OS file lock plus SQLite generation fencing. |
| Legacy Python catalogue migration | Not implemented. Existing Rust TUI catalogues are used in place. |

### 20.2 Source baseline

Behavioral source: [Rainbows4Blood/drivethru_catalog](https://github.com/Rainbows4Blood/drivethru_catalog), inspected commit [`73b5e4699db4a0aa5c1c01c2bd2021a7c65f223f`](https://github.com/Rainbows4Blood/drivethru_catalog/tree/73b5e4699db4a0aa5c1c01c2bd2021a7c65f223f).

- [Legacy command definitions](https://github.com/Rainbows4Blood/drivethru_catalog/blob/73b5e4699db4a0aa5c1c01c2bd2021a7c65f223f/dtrpg_catalog/cli.py)
- [Metadata extraction and completeness](https://github.com/Rainbows4Blood/drivethru_catalog/blob/73b5e4699db4a0aa5c1c01c2bd2021a7c65f223f/dtrpg_catalog/parse.py)
- [Discovery rules](https://github.com/Rainbows4Blood/drivethru_catalog/blob/73b5e4699db4a0aa5c1c01c2bd2021a7c65f223f/dtrpg_catalog/discovery.py)
- [Scheduler and API routing](https://github.com/Rainbows4Blood/drivethru_catalog/blob/73b5e4699db4a0aa5c1c01c2bd2021a7c65f223f/dtrpg_catalog/engine.py)
- [Legacy persistence semantics](https://github.com/Rainbows4Blood/drivethru_catalog/blob/73b5e4699db4a0aa5c1c01c2bd2021a7c65f223f/dtrpg_catalog/db.py)

This specification intentionally supersedes legacy behavior where the decision table or requirements differ. Legacy code is a reference for extraction fidelity, not an instruction to recreate removed features. Private source links require repository access; the requirements above are self-contained.

### 20.3 Technical references

These references support implementation constraints; they are not promises of complete compatibility or prescribed dependency versions.

- [SQLite WAL](https://www.sqlite.org/wal.html) and [synchronous durability](https://www.sqlite.org/pragma.html#pragma_synchronous)
- [Rust OS file locking](https://doc.rust-lang.org/std/fs/struct.File.html#method.try_lock)
- [SQLx SQLite options](https://docs.rs/sqlx/0.9.0/sqlx/sqlite/struct.SqliteConnectOptions.html)
- [Tokio blocking-task cancellation behavior](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html)
- [Reqwest connection/read/total timeout definitions](https://docs.rs/reqwest/latest/reqwest/struct.ClientBuilder.html)
- [yt-dlp browser cookie import implementation](https://github.com/yt-dlp/yt-dlp/blob/master/yt_dlp/cookies.py)
- [Chrome application-bound cookie encryption](https://security.googleblog.com/2024/07/improving-security-of-chrome-cookies-on.html)
- [Microsoft Edge application-bound encryption](https://learn.microsoft.com/en-us/deployedge/microsoft-edge-policies/applicationboundencryptionenabled)

Specification requirements are separate from executed evidence. See the validation document for actual implementation checks and the limits of earlier bounded live authentication diagnostics.

The fixed local storage revision supersedes all earlier network-storage setup decisions. See [validation evidence](docs/VALIDATION.md) for the checks actually executed.
