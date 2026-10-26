# kaijucrawl specification

Version: 0.1 — initial implementation specification  
Date: 2026-10-26  
Status: requirements and design baseline; implementation has not started  
Initial target: DriveThruRPG (`drivethru`)

## Contents

1. [Purpose and scope](#1-purpose-and-scope)
2. [Decisions and invariants](#2-decisions-and-invariants)
3. [Application and TUI](#3-application-and-tui)
4. [Storage configuration](#4-storage-configuration)
5. [Authentication](#5-authentication)
6. [DriveThruRPG adapter](#6-drivethrurpg-adapter)
7. [Fixed discovery and crawl lifecycle](#7-fixed-discovery-and-crawl-lifecycle)
8. [Product success and parsing](#8-product-success-and-parsing)
9. [Metadata contract](#9-metadata-contract)
10. [PostgreSQL persistence and ownership](#10-postgresql-persistence-and-ownership)
11. [Successful response archives and consistency](#11-successful-response-archives-and-consistency)
12. [HTTP behavior and failure handling](#12-http-behavior-and-failure-handling)
13. [Advanced settings](#13-advanced-settings)
14. [Reprocessing after parser updates](#14-reprocessing-after-parser-updates)
15. [Stopping and recovery](#15-stopping-and-recovery)
16. [Rust implementation structure](#16-rust-implementation-structure)
17. [Diagnostics and resource bounds](#17-diagnostics-and-resource-bounds)
18. [Acceptance criteria](#18-acceptance-criteria)
19. [Delivery sequence and validation](#19-delivery-sequence-and-validation)
20. [Legacy mapping and sources](#20-legacy-mapping-and-sources)

## 1. Purpose and scope

kaijucrawl builds a durable catalogue of product metadata exposed by DriveThruRPG. It discovers product IDs through sitemaps, retrieves product details from the product API, preserves the full extracted metadata, and archives successful source responses.

The application is written in Rust and operated through a terminal user interface (TUI). PostgreSQL is the first network database backend. S3-compatible object storage holds successful response bodies. Local files are limited to application configuration, optional protected credentials, bounded diagnostics, and disposable runtime staging; they are not the authoritative catalogue or checkpoint store.

The first release implements only DriveThruRPG. Dungeon Masters Guild, Storytellers Vault, Ulisses eBooks, and other storefronts are future adapters. Do not expose nonfunctional targets as selectable checkboxes. Shared machinery must carry an explicit target identity so another adapter can be added without changing the meaning of existing data.

This is a catalogue of first successful captures. It is not an automatically refreshed mirror. Successful records may become stale; prices, descriptions, ratings, and availability are observations from their capture time.

### 1.1 Included

- Interactive storage setup, target selection, authentication, dashboard, and advanced settings.
- Sitemap discovery on every crawl run.
- Automatic resumption and renewed eligibility of previously unsuccessful work.
- Full DriveThruRPG metadata extraction, including HTML and plain-text descriptions.
- Independent network limits per target.
- One active mutating application instance per catalogue.
- Successful response archival and recovery across PostgreSQL/S3 interruptions.
- Reprocessing of saved successful responses specifically for parser updates.
- Windows and Linux as first-class operating systems.

### 1.2 Excluded from the initial release

- Automatic or manual product refresh and age-based recrawling.
- API collection/listing discovery, category-page crawling, manual seeds, and numeric ID enumeration.
- Product-count or discovery-page run limits.
- CSV/JSON export workflows and machine-readable dashboard output.
- An archive-disable switch or archival of unsuccessful product responses.
- Importing the old SQLite databases or archives; the catalogue starts fresh.
- Distributed crawling, multiple cooperating writers, a daemon, a web UI, or a scheduler.
- Cross-store merging or deduplication.
- Downloading books, preview documents, images, audio, video, or purchased assets.
- Browser-driven crawling, automated storefront login, challenge solving, or an HTML page fallback for product extraction.

Browser cookie import is an authentication convenience, not a browser automation crawling backend.

## 2. Decisions and invariants

The following are requirements, not defaults that an implementation may silently replace.

| ID | Requirement |
| --- | --- |
| INV-01 | Launching `kaijucrawl` opens the TUI. Missing storage configuration forces the setup dialog before crawling. |
| INV-02 | Only `drivethru` is implemented initially. Target selection uses checkboxes. |
| INV-03 | Every new crawl run performs sitemap discovery, including after a prior interrupted run. |
| INV-04 | A successfully collected product is never fetched again by normal crawling, including when sitemap `lastmod` changes. |
| INV-05 | Outstanding and previously unsuccessful products become eligible on each new run. Attempts within one run are bounded. |
| INV-06 | Product success requires verified identity, a nonempty title, a nonempty full plain-text description, and durable PostgreSQL/S3 persistence. |
| INV-07 | Preserve all metadata fields extracted by the legacy DriveThruRPG parser. |
| INV-08 | Identity is `(target_key, product_id)`. Cross-store merging is a separate future processing step. |
| INV-09 | Exactly one instance owns mutating work for a catalogue at a time. Internal parser subprocesses are part of that instance. |
| INV-10 | Configured concurrency, pacing, and retry cooldowns are independent per target, even if future targets share an API host. |
| INV-11 | Always archive successful product responses. Do not archive failed, blocked, malformed, mismatched, or incomplete product responses. |
| INV-12 | Authentication problems pause the affected target. PostgreSQL or S3 outages pause the entire crawl. |
| INV-13 | Stopping or closing the application ends its work gracefully; durable state supports recovery after an abrupt termination. |
| INV-14 | Reprocessing reads saved responses and never fetches products or changes original observation timestamps. |

Failure classification, persistence protocols, menu layout, and advanced defaults below make these decisions implementable. They are part of the initial design baseline and must remain consistent with the invariants.

## 3. Application and TUI

### 3.1 Entry and navigation

`kaijucrawl` is the normal user-facing command. The public interface does not reproduce the legacy collection of crawl flags or subcommands. Conventional help/version handling may exist, but must not create a second operational workflow. When no interactive terminal is available, exit with a short explanation; do not silently launch a headless crawl.

After successful storage setup, show the dashboard with these accessible actions:

| View/action | Behavior |
| --- | --- |
| Targets | Checkbox list; initially only **DriveThruRPG**. Remember selection. |
| Start crawl | Validate storage, acquire catalogue ownership, snapshot settings, and start a new run for checked targets. |
| Stop | Stop scheduling, drain/commit valid work within the shutdown rules, then return to the idle dashboard. |
| Authentication | Configure cookies, import from a browser, test access, replace, or clear credentials for the selected target. |
| Storage | Configure and test PostgreSQL and S3 connections. |
| Advanced | Edit the settings in section 13. |
| Reprocess saved responses | Run the parser-update maintenance operation in section 14. |
| About | Show application version, record schema version, and parser version. |
| Quit | Stop gracefully if active, restore the terminal, and exit. |

No selected target means Start is disabled with a visible reason. Do not start crawling merely because the TUI opened. Target selection and advanced-setting edits affect the next run. Authentication replacement can take effect during a paused run. Changing storage while work is active requires stopping that work first.

All normal flows must work with a keyboard. Provide visible key hints, focus indication, scrollable dialogs, multiline paste handling where needed, terminal-resize handling, and useful text at small terminal sizes. Status must not depend on color alone. Network and storage operations must not block keyboard handling or screen updates.

### 3.2 Dashboard

Show both current-run activity and durable catalogue totals, clearly distinguished:

- Run state: idle, discovering/crawling, waiting for retry, authentication paused, storage paused, stopping, finished, or interrupted.
- Per-target discovery state: sitemaps queued, processed, retrying, and failed; distinct products seen this run; newly discovered products.
- Product queue: pending, active, waiting for persistence, retry scheduled, successful this run, and unsuccessful this run.
- Lifetime successful product count and last successful save time.
- Active download/parser work, request pacing/cooldown, recent throughput, and elapsed time.
- PostgreSQL and S3 connection status; sanitized failure reasons and next available action.
- A bounded recent-events panel and an inspectable list of unsuccessful jobs with IDs, reasons, and timestamps.

Discovery totals can grow. Do not report a global storefront completion percentage from an unknown denominator. A run can finish with unsuccessful products or discovery failures; display these prominently. A drained queue does not prove that every product on the storefront was discoverable or accessible.

Count a product as saved only after final persistence succeeds. Reimports of cookies, retries, and duplicate sitemap entries must not inflate success totals.

## 4. Storage configuration

### 4.1 Required initial setup

When either connection is missing, open a Storage setup dialog before allowing a crawl or reprocessing operation. The user may save incomplete settings for later or quit, but cannot start work with unvalidated storage. Existing settings remain editable after a connection failure; a transient outage must not reset configuration.

Use an existing PostgreSQL database and existing S3 bucket. Provisioning cloud accounts, servers, databases, and buckets is outside this application. The application creates/version-migrates its own schema in the configured database after validation.

| Connection | Fields |
| --- | --- |
| PostgreSQL | Connection string, including database and authentication/TLS parameters appropriate to the provider. Use an application schema named `kaijucrawl` by default. |
| S3 | Bucket, region/signing region, optional custom endpoint, optional key prefix (default `kaijucrawl/`), and credential source. |
| S3 credentials | Direct access-key ID/secret with optional session token, or an existing supported SDK credential/profile source. |
| S3 compatibility | Automatic/default addressing with an advanced path-style override when the provider requires it. |

There is no database-backend selector while PostgreSQL is the only implementation. Keep backend access separate from the TUI and adapter so additional implementations can be introduced later.

Test database connectivity and required application-schema read/write permissions. A read-only `SELECT 1` is insufficient. Test S3 by writing, reading/verifying, and deleting a tiny uniquely named test object under a dedicated setup prefix. Test objects are not product archives. Report the failing service and operation without exposing secrets. Clean up a test object if a later validation step fails, or report the exact non-secret cleanup issue.

Apply versioned schema migrations under exclusive catalogue/bootstrap coordination. An unsupported newer schema must produce an actionable error rather than an attempted downgrade. Never drop unrelated tables or reset an existing catalogue to resolve a setup problem.

### 4.2 Local settings and secrets

Store non-secret configuration and selected targets in the OS-appropriate application configuration location. Store credentials using the OS credential facility, or a configured environment/profile credential source. If protected persistent secret storage is unavailable, offer session-only entry; do not silently write plaintext secrets to a general configuration file.

Treat a PostgreSQL URL containing a password as a secret. Mask secret input by default and redact connection strings, cookie values, signed URLs, and authorization material from diagnostics. Never put authentication credentials in catalogue records, S3 objects, source control, or example fixtures.

No local data directory setting is exposed. Bounded temporary files and diagnostics may use OS application-cache/temp locations, with cleanup on completion and next startup. They must not be required to resume a catalogue from another installation using the same remote configuration and appropriate authentication.

Persist a generated catalogue UUID in PostgreSQL and use it in S3 object keys. The database's recorded bucket/prefix binding must agree with the configured archive location before work starts. Editing settings does not migrate an existing catalogue or retarget old archive references. Refuse incompatible bindings and explain the mismatch.

## 5. Authentication

### 5.1 Target-level menu

Authentication is configured per target. DriveThruRPG has website and API request scopes. Support:

- **No cookies:** attempt anonymously, with access limitations reported normally.
- **Paste cookies:** paste a raw `Cookie` header value, with an optional leading `Cookie:` label, for the website or API scope.
- **Import from browser:** find supported browsers/profiles for the current OS user and import relevant cookies from a selected profile.
- **Test Access:** verify the available access without modifying catalogue product/queue state.
- **Replace / clear:** replace or remove the target's configured credentials.

Reject embedded line breaks, other HTTP headers, malformed cookie input, and inputs above 1 MiB. A raw pasted header applies only to its explicitly selected host; do not infer parent-domain permission or share it across hosts/targets. Raw headers have no trustworthy expiry metadata, so report rejection when observed rather than inventing an expiration date.

For imported cookies, preserve domain versus host-only scope, path, secure flag, expiry, and any applicable container/partition context. Send only cookies that match the actual request. Omit unsupported partitioned cookies with a clear diagnostic instead of flattening their scope. A website cookie is not automatically valid for `api.drivethrurpg.com`.

### 5.2 Browser import

The initial browser discovery list includes Firefox, Chrome, Edge, and Brave on Windows and Linux, where installed. Discover profiles from documented/known profile locations and profile manifests. The user selects the browser and profile; expose Firefox container selection when relevant. Do not mix accounts across profiles or containers.

Import only cookies matching the selected target's website/API domains. Read the browser's data without modifying it. Handle SQLite locking/WAL consistently through a safe snapshot or supported read mechanism. If access is unavailable, report the reason and offer manual paste. Do not kill the browser to obtain its files.

Remember the chosen browser/profile/container locally, then reload matching cookies at the start of each run. Provide an explicit reimport action while paused. Imported cookies can remain in memory for the run; their values need not be duplicated into permanent application storage. Remembered manual cookies use the protected secret storage described above.

Browser detection is not a guarantee of import support. Firefox profile import and Linux Chromium keyring integration are supported paths to implement and test. Windows Chromium-family app-bound encryption can prevent direct import. Unsupported encrypted profiles must fail gracefully; do not require elevation, browser injection, disabled encryption, or disabled security policies. Manual paste remains available. Do not advertise a browser/OS combination as working until it has been tested against an identified version.

### 5.3 Test Access and session replacement

Test website/API access using bounded, target-specific requests. Prefer a known discovered product; an optional product ID/URL inside this diagnostic dialog is permitted only for testing and must never enqueue a manual seed. Validate requested IDs and host scope using the normal adapter rules.

Show separate outcomes for cookies loaded, endpoint reachable, product response parseable, and authenticated access confirmed. A public HTTP 200 does not prove authentication. If no reliable account/session signal exists, report authentication as unverified rather than inventing confirmation.

Test Access does not save a product, create an S3 archive, change success timestamps, mark crawl jobs complete, or start discovery. It replaces the old standalone `probe` operation.

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
| Interface language | `en`; product language comes from product metadata |
| Canonical product URL | `https://www.drivethrurpg.com/en/product/{id}` |
| Legacy image base | `https://d1vzi28wh99zvq.cloudfront.net/` |
| Legacy full-preview base | `https://watermark.drivethrurpg.com/` |

Accept discovered website URLs only for the DriveThruRPG apex and `www` hosts. Recognize locale-prefixed `/product/{id}` paths, optional slugs, and legacy `product_info.php?products_id={id}` links. Normalize IDs to positive decimal strings without leading zeroes; do not impose an arbitrary 32-bit limit. Preserve original discovered aliases in provenance while using a canonical queue identity.

Reject userinfo, unexpected ports, invalid IDs, and foreign hosts. Normalize product query/fragment aliases without creating duplicate tasks. Exclude account, checkout, login, wishlist, download, and non-product asset routes.

The adapter owns website/API hosts, routing, response unwrapping, taxonomy mappings, media resolution, and access classification. The scheduler, TUI, and persistence backend must not contain hard-coded DriveThruRPG routing or currency assumptions.

Legacy taxonomy roots are `10 → genre`, `20 → product_type`, `30 → rule_system`, `40 → language`, and `44498 → format`. These belong only to this adapter. Unknown values are retained without guessed classifications.

Before declaring live compatibility, validate the sitemap/detail route and representative payloads with authorized, bounded live tests. Do not silently change the site/group pair or substitute another storefront if routing stops working.

## 7. Fixed discovery and crawl lifecycle

### 7.1 New run

Starting a crawl creates a run ID after storage validation and acquisition of exclusive ownership. Snapshot selected targets and advanced settings. For each selected target:

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

Product and sitemap jobs have at most the configured number of attempts within one run. Transient failures may retry with backoff during that run. Definite missing products, incomplete metadata, parser schema errors, and explicit individual access restrictions become unsuccessful for the current run rather than looping immediately. They become eligible again on the next run. Verified robots exclusions remain excluded for the current policy and are evaluated again on the next run.

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

A **durable success** additionally has a verified successful-response object in S3 and a committed PostgreSQL product/archive/task transaction. Only durable successes are skipped forever by normal crawling.

Incomplete attempts retain only structured failure/status information, such as missing field names, parser version, HTTP status, and timestamps. Do not persist their response bodies or partial extracted product documents as successful catalogue records.

### 8.2 Response structure and identity

Support the legacy direct object, `product` wrapper, `data` object/list, and JSON:API attributes/relationships envelopes. Examine direct product candidates; do not substitute a recommendation, related record, or arbitrary deeply nested object. Resolve explicit included publisher/creator relationships where supported. Match JSON:API relationships by both resource type and ID when type is supplied.

Reject conflicting IDs, multiple matching product candidates, wrong-product responses, and ID-only envelopes with no recognizable product fields. Preserve the accepted source object and original envelope in raw metadata. New shapes require fixtures and an explicit adapter/parser update, not recursive guessing.

The initial product path is API-only. The legacy unused HTML/JSON-LD product-page parser is not a required runtime backend. HTML parsing is still needed for product description cleanup and link/media extraction.

### 8.3 Normalization

Keep cleaned HTML and readable plain text separately. Remove script/style/noscript/template/iframe/object/embed elements, event-handler and `srcdoc` attributes, and dangerous schemes in relevant URL attributes. Preserve inline word continuity and paragraph/list/table boundaries. Use bounded traversal; do not introduce quadratic repeated tree rewriting.

This cleanup is not a promise that arbitrary source HTML is safe for every downstream renderer. Preserve original source descriptions within successful raw metadata and the archived response. The TUI displays text and never executes HTML. Strip terminal control sequences from displayed/logged source text without altering archived response bytes.

Normalize missing values to null or empty collections as appropriate; do not fabricate descriptions, creators, dates, ratings, or page counts. Preserve zero prices. Reject booleans/nonfinite values as numbers and fractional page/file-count values as integers. Preserve source date representations alongside UTC capture/reprocessing timestamps.

Use exact decimal handling for monetary fields and PostgreSQL `NUMERIC` or lossless decimal text/JSON representation. Do not route prices through binary floating point. Preserve source JSON numeric precision in raw metadata. This changes numeric implementation while retaining every legacy field and its meaning.

For DriveThruRPG, explicit returned currency takes precedence. Preserve the legacy fallback to USD only as an adapter-specific `frontend_base_currency` assumption, with `currency_source` recorded. Do not generalize that assumption to future targets. Headline price is usable special price when `onSale` is true, otherwise base price; preserve the original sale flag.

## 9. Metadata contract

All legacy extracted fields below remain available in the structured record. Additional lifecycle fields may be added without deleting source information. Nested groups may use PostgreSQL JSONB; identity, common filters, quality, and timestamps must remain queryable without reading S3. The public record uses target-scoped product IDs represented as decimal strings.

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
| `raw_path` | Compatibility field containing a logical S3 object reference instead of a local filesystem path. |
| `archive` | Bucket, key, optional provider version ID, content hash, byte lengths, compression, and archive timestamp. |
| `discovery` | First/last seen, source sitemaps, aliases, and sitemap modification hints. |
| `crawl_status` | Current durable task state, attempts, check time, HTTP status, and sanitized error. |

Keep `raw_metadata` as part of the full structured metadata contract even though exact response bytes are also archived in S3. Removing that field or replacing it solely with a reference would require an explicit future contract change.

Preserve provenance when exposing the record through database views or the TUI. Do not rewrite `fetched_at` during reprocessing or claim a recent sitemap sighting is a recent product fetch.

## 10. PostgreSQL persistence and ownership

### 10.1 Logical entities

The following describes required information and constraints; SQL migrations may use additional normalized tables/indexes.

| Entity | Required information/constraints |
| --- | --- |
| Catalogue | UUID, schema version, configured object-store binding, creation time. |
| Ownership lease | One row per catalogue, owner UUID, monotonic fencing generation, database-clock expiry. |
| Runs | ID, selected targets, non-secret settings snapshot, start/end/state, discovery and product outcome summaries. |
| Target run state | Authentication/storage/cooldown state and target-specific pacing context. |
| Product jobs | Unique `(target_key, product_id)`, canonical URL, eligibility/state, current run, per-run/lifetime attempt counts, due time, claim owner/generation, outcome and timestamps. |
| Discovery jobs | Unique `(run_id, target_key, sitemap_url)`, state, attempts, parent/source, and sanitized failures. |
| Discoveries | Product identity, source/alias URL, first/last seen, and `lastmod`; deduplicated provenance. |
| Products | Unique `(target_key, product_id)`, typed commonly queried fields, full structured JSONB record, parser/schema versions, archive reference, observation times. |
| Archives/save intents | Successful-capture identity, hash/key/lengths, staged/finalized persistence state, capture time, owner generation. |
| Attempts/events | Bounded/redacted operational metadata; no unsuccessful response bodies or credential values. |
| Reprocessing runs | Parser version, progress/checkpoint, result counts and per-record failures. |

Use foreign keys and uniqueness constraints. Claim jobs and save each batch transactionally. Store product/archive reference and successful task transition in the same PostgreSQL transaction. A transaction failure must not leave a task marked successful without its product.

Use parameterized SQL, bounded query results, and indexes for eligible/due jobs, product identity, run status, and common metadata queries. Use keyset pagination for large scans. Do not load the whole catalogue into memory or repeatedly deserialize all records to refresh dashboard counts.

### 10.2 One instance and fencing

The initial implementation must coordinate one writer through PostgreSQL, not a local lock. Use an atomic lease claim with a monotonically increasing generation and database-server time. Initial internal defaults are a 30-second lease and renewal every 10 seconds; these are implementation constants, not additional user-facing settings.

Every mutating queue/product transaction must validate the current owner, generation, and unexpired lease in the same transaction. Lease takeover must serialize with those checks. A process that cannot renew or verify ownership stops starting requests and cannot commit new product/task changes until it safely reacquires ownership. New ownership invalidates stale claims; the new owner recovers them before scheduling.

An expired process may still have an in-flight network request, but it cannot publish authoritative success under its old generation. S3 keys are immutable/content-addressed so late object writes cannot overwrite another response. Do not use a session advisory lock through a transaction-pooled connection as if it were a durable lease.

Schema initialization also needs transactional bootstrap coordination before the lease row exists. A second instance may report the existing owner's status, but must not start crawling, reprocessing, schema changes, or queue recovery concurrently.

## 11. Successful response archives and consistency

### 11.1 What is retained

Always retain the body of every valid successful product capture in S3. Here, the body means HTTP transfer/content-decoded response bytes before JSON parsing/normalization; it is not a packet capture. Compress those bytes losslessly with gzip and compute SHA-256 over the uncompressed bytes. Preserve the complete source envelope.

Only a response meeting the valid-capture predicate may enter the durable archive workflow. Do not upload HTTP failure pages, authentication/challenge bodies, malformed JSON, wrong-product responses, parser-timeout inputs, or partial products missing title/description. These can occupy bounded transient buffers while classified, then must be discarded. Logs and diagnostics must not become an alternate body archive.

Do not archive Test Access responses, sitemaps, cookies, browser databases, or purchased/media files as product captures. A valid capture whose final database write is interrupted may temporarily have a pending object; it is recovered as described below, not presented as a completed product.

Use deterministic keys such as:

```text
{prefix}/{catalogue_uuid}/drivethru/{product_id}/{sha256}.json.gz
```

Treat successful objects as immutable. Repeated persistence of the same bytes reuses the key. If recovery ever produces a different valid body, never overwrite an existing successful object. No automatic retention expiry or history deletion is implemented. S3 bucket versioning is optional and is not required for correctness.

### 11.2 Save protocol

PostgreSQL and S3 do not share a transaction. The application must implement and test a recoverable protocol:

1. Fetch and fully validate/parse a candidate in bounded memory or disposable staging.
2. Under a valid lease, persist a small save intent containing target/product identity, capture timestamp, parser version, hash, expected sizes, and deterministic object key. It contains no unsuccessful response body.
3. Upload the compressed valid response and verify the upload outcome/identity. Do not assume an S3 ETag is a SHA-256 or MD5 content hash.
4. Commit the complete product record, archive reference, finalized intent, and successful task state in one PostgreSQL transaction, guarded by ownership fencing.
5. Update dashboard success counters and remove disposable staging after commit.

On an ambiguous upload result, inspect the expected object and verify recorded hash/size or read and hash it when necessary. On a failed database commit, leave the job recoverable and the object/intention available. Retry persistence before making another product request. On startup, reconcile pending intents: an available valid object can be read/reparsed and finalized; an absent object returns the product to fetch eligibility. Conflicting/corrupt objects are a storage error, never a successful record.

A crash may leave an object without a finalized product transaction. Do not solve this by deleting previously successful archives or by pretending the operation was atomic. Recovery must be idempotent and must never downgrade a completed product into fetch eligibility merely because it encounters an old intent.

Storage outages pause new discovery/product requests and apply bounded backpressure to completed candidates. If the application exits before unsaved staging is recovered, its durable intents/queue determine the next action. A successful catalogue record must always refer to an existing verified archive at commit time; later external deletion/corruption is reported as a storage problem.

## 12. HTTP behavior and failure handling

### 12.1 Request rules

Use asynchronous HTTPS with certificate verification. Keep website and API cookie scopes explicit. Validate every redirect; product requests retain the approved API origin, product path/ID, and `siteId=10`/`groupId=1`. Reject unexpected destinations and never forward raw cookies to another host.

Use an identifying `KaijuCrawl/{application_version}` User-Agent by default, with an advanced override. Cookies apply only to the selected target and matching URL. Do not silently use ambient `.netrc` credentials, harvested unrelated browser cookies, or automatically accumulated Set-Cookie values as a substitute for configured authentication.

Evaluate robots.txt for the relevant origin with a maintained standards-aware parser. Refresh policy at the start of a run and when required after access changes. Honor applicable disallow, crawl delay, request-rate constraints, and server cooldowns. Verified exclusion is recorded distinctly from an unavailable policy. A missing robots.txt response (404/410) permits requests; an authentication/challenge/error policy response must not silently become allow-all.

Within a target, share the configured request-start interval across discovery and product requests, including retries and redirects. Bound all target HTTP concurrency by its worker setting. Future targets receive independent limiters even when they use the same host; do not introduce a global host-wide limiter contrary to INV-10. Server-requested waiting still applies to the target that received it.

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
| HTTP 403 | Use response evidence: individual restriction is a product outcome; authentication/challenge or unclear broad denial pauses the target for inspection. |
| Verified robots disallow | Exclude that job under the current policy; reevaluate next run. |
| Robots policy cannot be established | Retry transient transport errors within budget; otherwise pause the target with the policy problem visible. |
| Sitemap XML/structure failure | Retry transient retrieval failures; record persistent parse failure for the run; do not label discovery complete without errors. |
| PostgreSQL/S3 operation failure or lost catalogue ownership | Pause the whole crawl and stop new upstream requests; reconcile/revalidate before resuming. |

Use bounded exponential retry delay starting at 10 seconds and capped at one hour, with small jitter. Honor a longer valid Retry-After duration/date. HTTP 429 imposes at least a 60-second target cooldown. Persist due times so restarting does not bypass a known future server cooldown. The max-attempts value counts actual job attempts, not hidden retries added independently by multiple layers.

Do not retain the legacy automatic five-parser-error stop threshold as a hidden user workflow. Product failures remain visible, bounded per run, and retriable on the next run. Authentication, policy, and storage pauses follow the explicit rules above.

## 13. Advanced settings

All of these settings are edited through the TUI. Initial values mostly inherit the old implementation for continuity; they are starting values, not measured Rust throughput guarantees. Units and scope must be visible. Save validation rejects nonfinite, negative, or out-of-range values as appropriate.

| Group | Setting | Initial value | Meaning |
| --- | --- | --- | --- |
| Network | Download workers | 10 per target | Maximum concurrent target HTTP requests. |
| Network | Minimum request interval | 100 ms per target | Spacing between request starts across that target's workers. Zero is allowed; robots/server limits still apply. |
| Network | Connect timeout | 15 s | Connection-establishment bound; separate from read inactivity. |
| Network | Read inactivity timeout | 45 s | Maximum wait for another read; not a whole-response deadline. |
| Network | Total response deadline | 300 s | Bound from starting a request through reading its body; slower-than-inactivity trickles cannot continue forever. |
| Network | Attempts per job per run | 5 | Positive integer; no unlimited retry mode. |
| Network | User-Agent | `KaijuCrawl/{version}` | Nonempty single line; optional contact detail may be supplied by the user. |
| Parsing | Parser concurrency | 2 | Concurrent parser slots, capped by available download workers for the active target. |
| Parsing | Product parse deadline | 30 s | Includes starting an assigned worker, excludes waiting for a parser slot. |
| Parsing | Parser backend | Isolated process | Isolated process by default; optional cooperative thread mode must explicitly describe its weaker cancellation guarantee. |
| Shutdown | Drain timeout | 5 s | Grace for active work after Stop/Quit/interrupt; second interrupt ends the grace early. |
| Database | Claim batch size | 20 | Maximum queue reservations per transaction; integer 1–1,000. |
| Database | Save batch size | 10 | Maximum complete products per PostgreSQL save transaction; integer 1–1,000. |
| Database | Partial-batch flush interval | 500 ms | Maximum intentional batching delay before a pending batch is submitted; storage latency/backpressure may extend actual completion. |

Network/parse/drain durations must be positive, except the request interval and flush interval may be zero. Zero flush interval means submit available records immediately. The total response deadline must not be below the connect or read inactivity timeout. CPU/process capacity and storage backpressure can reduce effective concurrency below a configured maximum.

For future targets, each target can hold its own network settings. The initial UI edits the single DriveThruRPG target. Database batching and shutdown settings apply to the application instance. Parser backend/capacity belongs to the application runtime, with bounded target allocation when more adapters are later introduced.

Do not expose discovery switches, seed inputs, refresh/retry switches, run limits, response-retention switches, export directories, SQLite cache controls, or JSON-output modes in Advanced.

## 14. Reprocessing after parser updates

Reprocess saved responses is a separate maintenance action for applying a newer parser to already successful captures. It requires the same validated storage and exclusive catalogue ownership as crawling. It cannot run concurrently with a crawl.

The TUI shows the current parser version and how many saved records have a different version. Process eligible saved captures in keyset batches and persist a checkpoint/progress record. Re-running an interrupted operation skips records already successfully processed with the intended parser version. Parser behavior changes must increment the parser version.

For each record:

1. Read its referenced successful response from S3.
2. Verify object integrity and product identity.
3. Apply the current parser using the same bounds as live extraction.
4. If the resulting record still satisfies title + description completeness, atomically replace derived metadata and update parser/schema version and `reparsed_at`.
5. Preserve original capture/check timestamps, target/product identity, discovery provenance, archive bytes/reference, and crawl success state.

If the new parser fails, times out, produces incomplete data, or mismatches identity, preserve the previous complete record. Record the reprocessing failure separately and show it in the TUI. Do not delete the raw archive, overwrite a good record with a partial one, or turn parser maintenance into a network refresh. Missing/corrupt archives are storage failures requiring attention.

No new raw response archive is produced by reprocessing. Existing captures stay unchanged. Full version history of derived records is not required initially; the current record, original source, and reprocessing outcome/provenance are required.

## 15. Stopping and recovery

Stop, Quit, Ctrl+C, and supported OS termination signals enter one shutdown path:

1. Stop new scheduling and request starts immediately.
2. Continue ownership checks while draining work that can complete safely within the configured grace period.
3. Flush already validated, upload-verified product batches when storage is available.
4. Cancel outstanding asynchronous requests and terminate/reap remaining isolated parser workers at the deadline.
5. Release unstarted reservations, preserve pending save intents, mark the run interrupted/stopped where possible, and release ownership after work is quiescent.
6. Restore terminal state and exit, or return to idle for Stop.

A second interrupt skips the remaining grace. Database and object-store operations also need finite deadlines; shutdown must not wait indefinitely for one. If storage is unavailable, do not pretend unsaved candidates were committed. Already durable claims/intents and lease expiry permit next-start recovery.

An OS kill, power loss, or terminal close may prevent orderly cleanup. Recovery must not depend on finalizers running: ownership expiry, fenced claims, successful-record checks, and idempotent save-intent reconciliation are authoritative.

Default isolated parsing must have enforceable termination. Cooperative thread parsing is explicitly weaker and cannot promise hard cancellation of a noncooperating library call. Do not describe a timeout around a running blocking task as killing that task. A last-resort process exit must preserve remote recoverability and restore terminal state where possible.

No background daemon remains after the user closes kaijucrawl. Credentials may need replacement after restart; authenticated session continuity is not guaranteed by durable queue state.

## 16. Rust implementation structure

Use a stable Rust toolchain, document the minimum supported Rust version when implementation begins, and commit `Cargo.lock` for the executable. Choose maintained libraries after checking their current APIs and Windows/Linux support; this specification does not pin unverified crate versions.

| Boundary | Responsibility |
| --- | --- |
| Application/TUI | Events, dialogs, rendering, command dispatch, settings snapshots, terminal restoration. |
| Domain | Target/product identity, records, success predicate, outcomes, run/attempt state, versioning. |
| Scheduler | Fixed discovery lifecycle, bounded claims, target rate limits, cancellation, retry and pause rules. |
| Storefront adapter | Target-approved URLs, sitemap product recognition, API request construction, parsing and classifications. |
| HTTP/auth | Async transport, robots policy, redirect checks, cookie scopes, browser import providers, redaction. |
| Parser runtime | Pure extraction inputs/outputs and isolated worker or cooperative thread execution. |
| Catalogue store | Transactional jobs, products, intents, ownership fencing, metrics, and reprocessing checkpoints. |
| Object store | S3-compatible put/get/head/delete-test operations, integrity, immutable keys, and bounded retry. |

An initial module layout can follow `app`, `tui`, `domain`, `scheduler`, `targets/drivethru`, `http`, `auth`, `parser`, `storage/postgres`, `storage/s3`, and `telemetry`, with SQL migrations and sanitized fixtures in separate directories. This is guidance, not a requirement to create many crates before useful behavior exists.

Keep storage interfaces focused on domain operations such as claiming jobs and committing captures. PostgreSQL transaction/fencing logic belongs inside its backend; the scheduler must not assemble ad hoc SQL. Object persistence remains a separate interface because it has different consistency and failure behavior.

Use bounded async channels/semaphores between download, parsing, upload, persistence, and TUI event delivery. HTTP clients and database pools are reused. Do not spawn an unbounded task per discovered product, hold database transactions open across network/S3 requests, or run CPU-heavy parsing on the TUI/async executor thread.

An isolated parser worker can be the same executable launched through an internal protocol. It receives only bounded body data, target/expected identity, and parser parameters; it does not receive database/S3 credentials or perform network requests. Cap input/output size, detect worker crashes, enforce the deadline, and replace failed workers. This internal worker protocol is not a public user-facing crawling mode.

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
| Recent TUI events | Bounded ring buffer; no unbounded per-request render history. |
| Body/parser staging | Bounded by active work and maximum sizes; clean on completion, failure, and next startup. |

Apply both encoded-input and decoded-output bounds so compression does not bypass limits. These are per-resource safeguards, not user-configurable run quotas. Whole-run memory must stay bounded independently of catalogue size. Use backpressure when PostgreSQL, S3, or parsing cannot keep up.

Store useful attempt information: target, product/sitemap identity, stage, classified outcome, attempt number, timestamps, HTTP status, parser version, and sanitized reason. Keep response bodies only in the successful S3 archive and successful structured raw metadata. Do not log cookie values, authentication headers, SQL credentials, browser databases, or full copied HTML/error bodies.

Local rotating logs may assist debugging; the authoritative progress/outcome state is remote. The dashboard must remain useful without opening log files. Diagnostics are neither an export feature nor a machine-readable stdout mode.

## 18. Acceptance criteria

Every invariant requires meaningful tests. The following scenarios define the initial release boundary.

| ID | Scenario and expected result |
| --- | --- |
| AC-01 | Fresh launch enters storage setup; cancelling exits; unvalidated settings cannot start a crawl. |
| AC-02 | Existing PostgreSQL and S3 configurations pass real required-operation tests; setup test objects are cleaned up. |
| AC-03 | Only DriveThruRPG is selectable; an empty selection disables Start. |
| AC-04 | Every new run scans root/nested sitemaps; indexes, gzip, cycles, duplicates, malformed XML, and foreign-host entries are handled correctly. |
| AC-05 | Repeated locale/slug/query aliases produce one target-scoped product job while retaining provenance. |
| AC-06 | A previously successful product is never fetched again, including changed `lastmod`, later discovery, and subsequent runs. |
| AC-07 | A failed/incomplete/missing/interrupted product is eligible next run; duplicate discoveries cannot reset its current-run budget. |
| AC-08 | Each job has finite attempts; Retry-After/429 cooldowns survive restart and the TUI shows waiting status. |
| AC-09 | Matching ID + title + full description is required. Missing optional metadata does not fail an otherwise complete product. |
| AC-10 | Full metadata fixtures exercise every field group, zero prices, exact decimals, rating buckets, taxonomy, media references, flags, and unknown raw fields. |
| AC-11 | Wrong IDs, ambiguous wrappers, nested recommendations, challenges, malformed JSON, and incomplete records never become successful products or durable body archives. |
| AC-12 | Successful response bytes round-trip through gzip/S3 with the same SHA-256; product rows refer to the correct immutable object. |
| AC-13 | Failures/crashes before and after intent creation, upload, and PostgreSQL commit recover without duplicate success, stale writes, or lost successful records. |
| AC-14 | Two application instances compete for ownership; only one can mutate. Lease loss and takeover fence the previous writer. |
| AC-15 | Rate limits are independent per target. A fake second target sharing a host demonstrates this without shipping another storefront adapter. |
| AC-16 | Cookie domain/path/secure/expiry and host-only rules survive import; profile/container identities remain separate; redirected requests never leak credentials. |
| AC-17 | Browser/profile detection and manual paste work with fixtures; inaccessible/keyring-protected/app-bound cases offer a clear fallback and never crash the TUI. |
| AC-18 | Test Access performs no product/queue/archive mutations and does not infer authentication merely from a public HTTP 200. |
| AC-19 | Authentication failures pause the target; storage failure pauses all new crawling; replacing credentials/recovering storage resumes safely. |
| AC-20 | Reprocessing updates parser-derived metadata without storefront requests or changed capture times; a failing new parser preserves the prior complete record. |
| AC-21 | Parser hang/crash and forced shutdown leave the catalogue recoverable; process mode terminates/reaps workers within the implemented bound. |
| AC-22 | Stop/Quit/interrupt restores the terminal, releases or expires ownership safely, and never relies on local files for durable resumption. |
| AC-23 | Dashboard remains interactive during slow network/storage operations and does not claim absolute storefront completeness. |
| AC-24 | Windows/Linux builds and terminal interaction are verified; browser-import support claims identify actually tested browser/OS versions. |
| AC-25 | No old refresh, seed, retry-mode, run-limit, no-raw, export, or JSON-output controls reappear in the TUI or public operational CLI. |

## 19. Delivery sequence and validation

Implement in reviewable increments:

1. Domain contracts, TUI skeleton, local settings/secret references, and required storage setup.
2. PostgreSQL migrations, ownership fencing, queue/record operations, and S3 save-intent recovery with integration tests.
3. DriveThruRPG sitemap adapter, bounded HTTP transport, and fixed new-run eligibility rules.
4. Full product parser and isolated runtime, validated against sanitized golden fixtures covering the legacy metadata contract.
5. End-to-end crawl/dashboard, manual cookies, Test Access, failure pauses, and shutdown recovery.
6. Browser import providers and parser-update reprocessing; platform compatibility verification.

These increments do not redefine the release scope: the first release includes all accepted features, including best-effort browser import with documented fallbacks. Do not label an early increment complete merely because anonymous product fetching works.

Use unit tests for pure parsing/state rules and integration tests against disposable PostgreSQL/S3-compatible services for transactional behavior. Use a controllable local HTTP server for robots, redirects, retries, discovery, auth classification, and interruption. Browser fixtures must be synthetic or sanitized. Do not check real sessions, credentials, account data, or full unreviewed browser profiles into this public repository.

Normal tests must not crawl live storefronts. Explicitly requested live validation is separate, narrowly scoped, rate-limited, and documented with what was actually observed. Repository historical tests and the previous Python implementation do not prove the Rust application or current storefront works.

When a Cargo project exists, expected routine checks are formatting, Clippy, relevant unit tests, and the affected integration/platform checks. Record skipped checks and their concrete reason. In this specification-only repository, validate Markdown links, field/requirement coverage, and consistency; do not claim Rust tests have run.

## 20. Legacy mapping and sources

### 20.1 Deliberate changes from the old crawler

| Old interface/behavior | New behavior |
| --- | --- |
| `--data-dir`, local SQLite checkpoint/archives | Mandatory PostgreSQL/S3 setup; remote authoritative state. |
| Separate storefront executables | One Rust application; only DriveThruRPG implemented initially. |
| `--discovery`, `--seed`, `--seed-file` | Fixed sitemap discovery on each run; no seeds or API listings. |
| `--retry-failed`, `--retry-discovery` | Automatic next-run eligibility and bounded current-run retry. |
| `--refresh-days`, `--refresh-discovery`, `lastmod` product requeue | No product refresh; discovery always runs and successful products stay skipped. |
| Cookie file CLI flags | TUI paste and best-effort browser import, with explicit host scope. |
| Workers/delay/timeouts/attempts/User-Agent | Advanced TUI settings, independently scoped per target. |
| Parser concurrency/deadline/backend | Advanced TUI settings with honest Rust cancellation guarantees. |
| Shutdown timeout | Advanced TUI setting and a unified graceful-stop path. |
| Claim/save batching | Advanced TUI settings backed by PostgreSQL transactions. |
| SQLite cache size | Removed. |
| Product/discovery run limits | Removed. |
| `--no-raw` | Removed; successful response archives are mandatory. |
| Partial products saved as catalogue products | Incomplete attempt metadata only; successful products require title + description. |
| `export`, `--output` | Removed. |
| `status --json`, `probe --json`, `--no-progress` | TUI dashboard and dialogs; no initial machine-readable operational output. |
| `probe` | Test Access diagnostic action without catalogue/archive writes. |
| `reparse` | Parser-update maintenance using S3; preserve the prior good record if reprocessing fails. |
| Local writer lock | PostgreSQL ownership lease with fencing. |
| Old catalogue migration | Not implemented; start fresh. |

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

- [Tokio blocking-task cancellation behavior](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html)
- [Reqwest connection/read/total timeout definitions](https://docs.rs/reqwest/latest/reqwest/struct.ClientBuilder.html)
- [AWS SDK for Rust endpoint configuration](https://docs.aws.amazon.com/sdk-for-rust/latest/dg/endpoints.html)
- [AWS SDK for Rust authentication](https://docs.aws.amazon.com/sdk-for-rust/latest/dg/credentials.html)
- [yt-dlp browser cookie import implementation](https://github.com/yt-dlp/yt-dlp/blob/master/yt_dlp/cookies.py)
- [Chrome application-bound cookie encryption](https://security.googleblog.com/2024/07/improving-security-of-chrome-cookies-on.html)
- [Microsoft Edge application-bound encryption](https://learn.microsoft.com/en-us/deployedge/microsoft-edge-policies/applicationboundencryptionenabled)

No live storefront, browser session, cloud database, or object bucket was exercised to author this specification. Actual implementation and acceptance results must be documented when available.
