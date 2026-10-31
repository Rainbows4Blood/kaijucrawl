# KaijuCrawl

A Rust + Tauri desktop application that collects the first complete capture of each DriveThruRPG product. [SPEC.md](SPEC.md) defines the contract; [architecture](docs/ARCHITECTURE.md) and [validation](docs/VALIDATION.md) describe the implementation and checks.

The desktop interface replaces the TUI. It uses your existing **`<Userfolder>/.kaijucrawl`** directory directly: catalogue, archives, settings and remembered cookies. There is no conversion or reset. Successful products remain skipped; the next crawl recovers interrupted saves, retries unfinished products and discovers sitemaps again.

## Build and launch

Rust **1.99.0** is pinned. The frontend uses TypeScript and Vite 8; Node **22.12+** is required (Node 26 was used for validation). Windows needs MSVC build tools and WebView2; Linux needs GTK 3/WebKitGTK 4.1 development libraries to build. See [Tauri's platform prerequisites](https://v2.tauri.app/start/prerequisites/).

```sh
npm ci
npm run desktop:build
```

Launch `target/release/kaijucrawl.exe` on Windows or `target/release/kaijucrawl` on Linux. No terminal is required. This builds a standalone executable with bundled frontend assets, not an installer. WebView2/WebKitGTK remains a system runtime prerequisite.

For development:

```sh
npm run desktop
```

`npm run build` checks TypeScript and builds only the frontend. `npm run dev` serves the frontend for development; it does not provide the Rust backend. The production application needs neither Node nor a development server. `--help` and `--version` remain the only public CLI options; there is no headless crawler or configurable data-directory flag.

## Continue an existing catalogue

Close the old TUI before starting work in the desktop app. Both use the same OS writer lock and SQLite generation fencing, so they cannot mutate the catalogue concurrently. Opening the desktop app validates storage and loads totals automatically; it never starts crawling by itself.

Select DriveThruRPG and choose **Start crawl**. This creates a new run using the existing queue and recovery protocol. **Resume run** continues an authentication/storage pause in the current process. A previous interrupted run remains in history, and its unfinished products are picked up by the new run.

Catalogue schema **16** adds Warhammer groups, narrow Rule System eligibility and separate context results. It retains schema 15’s per-system OSR Adventure levels and schema 14’s OSR groups and metadata index. It preserves schema 13’s Shadowrun/CoC edition groups, retaining schema 12’s indexed eligibility and separate location/calendar-range results and schema 11’s D&D enrichment. It retains schema 10’s removal of the old Search workspace and derived indexes. Saved books, archives, capture times, unfinished work, settings, cookies, analysis results, group memberships and history are preserved in place. Analyses now read the books directly; startup no longer builds Search indexes. Product record schema **2** and parser **drivethru-rust-1** are unchanged. Existing compatible catalogues upgrade automatically; older executables refuse the newer schema.

The app opens at **Crawlers → Overview**. The menu button opens a left flyout with **Crawlers**, **Discovery**, **Dungeons & Dragons**, **OSR**, **Shadowrun**, **Call of Cthulhu**, **Warhammer Fantasy** and **Warhammer 40k**. Within Crawlers, **Overview**, **Authentication**, **Unsuccessful jobs**, **Local storage** and **About** are horizontal tabs. Storage readiness and Close application stay in the top bar; the crawl state remains visible across tabs. Opening or closing the menu preserves your current view and authentication draft.

**Discovery → Analysis** now runs the four system-independent methods. Each system has its own themed Analysis dashboard and run-all sequence. All existing algorithms, the standalone dropdown component, tests, API integrations and saved results remain intact. The general Search workspace and filters remain retired. Warhammer 40k now has game-specific tables and saved book details.

While local storage opens, saved totals display as loading until read, and Close remains available. The upgrade removes derived indexes without compacting or rewriting the catalogue file; SQLite can reuse their space.

## Analyze your library

Open **Discovery → Analysis** for a dashboard of saved coverage and category totals. There is no method selector. **Run analyses** executes the following sequence:

1. **Weighted relevance** — all saved books, calculated locally.
2. **Primary book role** — Core Rulebook / Supplement / Adventure / None.
3. **Solo Adventure / Gamebook** — independent solo-play classification.
4. **Adventure player counts** — the current Adventures, including those just classified in step 2.

The default keeps current results, including unknowns, and processes missing or invalidated ones. Check **Rerun all analyses, including current results** to include previously analyzed books again. The three online stages use OpenAI credits; the existing parallel workers, retries and structured-output model escalation remain in effect. D&D, Shadowrun and CoC analyses are not part of this sequence.

Rust runs the entire sequence under one catalogue ownership lifetime, so switching workspaces does not interrupt it or start extra work. The action becomes **Cancel analysis** in the same position. Cancel or Close drains the current workers and keeps completed results without starting later stages. A local failure stops the sequence; individual unsuccessful books continue and can be retried by a later explicit run. Restart never automatically resumes paid work.

**Weighted relevance** blends the saved average with 20 baseline ratings at 3/5: `(rating × count + 60) / (count + 20)`. Thus 4.7 stars from 1,000 ratings scores **4.667**, while 5 stars from 3 ratings scores **3.261**. Books without usable ratings remain unscored. The fixed baseline keeps scores comparable between runs.

All four cards show current whole-catalogue totals across runs, including completed unknowns. Adventure eligibility updates as primary roles are saved. Progress arrives through the existing one-second snapshots of committed results, with animated bars and no per-frame catalogue recount. The summary also shows the current sequence step and its processed fraction, so reruns remain visible even when saved coverage is already complete. Retry, escalation and unsuccessful-result notices stay in this dashboard; there is no latest-run statistics panel. Reading totals or opening Discovery does not calculate results or send paid requests.

Scores retain the calculation time and saved inputs in SQLite for future data views. They describe captured ratings, not current storefront popularity.

## Classify books by role

**Primary book role** is the second Discovery stage. Decisions reads each book’s saved full description and supporting metadata and assigns one primary category: **Core Rulebook** (needed to play), **Supplement** (options for players or game masters), **Adventure** (a ready-to-play story), or **None**. Mixed products receive their dominant role. Source taxonomy is treated as a clue rather than trusted classification.

This is an online, paid analysis: one book per request, using `OPENAI_API_KEY`, with additional attempts for API/connection failures. Requests run concurrently with one worker per available logical CPU (fallback one), sharing a client; capacity is automatic. There is no worker setting or added request delay. Current saved classifications are skipped by default. Cancel or close to stop; a later explicit run handles the remaining books. To replace existing results too, select **Rerun all analyses, including current results** before starting Discovery. All Decisions request failures retry automatically until success or cancellation: every HTTP error (including 429/503), lost connections or Wi-Fi, timeouts, and invalid/oversized responses. Delays start at 2 seconds and double to a 5-minute base cap, plus up to 1 second of jitter; a longer `Retry-After` is honored. The shared cooldown pauses new requests across all workers while completed responses are saved. Analysis shows an essential retry notice while requests are waiting. Account/key errors also retry; they may need correction before requests can succeed. Missing/invalid local credentials, local request validation, storage and ownership failures are not retried. Cancel/Close interrupt backoff promptly, retain saved results and release catalogue ownership after workers finish. Model refusals and oversized evidence are unsuccessful, distinct from None; they preserve any previous good result. Progress shows current saved category totals across the catalogue. Run counters and token accounting remain in backend history. Timed-out, interrupted or invalid responses may still be billed, including retry attempts; reported usage covers validated responses only.

Prompt caching is disabled: each book has different evidence, so caching those inputs does not help this one-pass analysis. Existing classifications remain valid; usage history keeps its original cache counters.

Results are stored separately from source metadata. Changed classification evidence invalidates old results; unrelated rating changes do not. Model, rule version, calculation time and probability estimates stay stored with each result.

## Solo Adventure / Gamebook

**Solo Adventure / Gamebook** is the third Discovery stage. It independently classifies **every saved book** using Decisions and GPT-6 Luna:

- **Solo Adventure:** a playable solo story requiring an existing external game system or rulebook.
- **Gamebook:** a self-contained playable solo experience with its required rules included.
- **Neither:** other books or insufficient evidence. A one-player-plus-GM adventure, generic solo toolkit or ordinary group adventure does not qualify merely because it mentions solo play.

Only the original saved metadata and full description are sent. Previous book roles, player counts and relevance scores are never supplied or used to narrow this analysis. Each system retains its own result and group membership. Current results, including Neither, are reused unless you select reclassification. This uses the same logical-CPU workers, exponential retries, cancellation and disabled Decisions caching as Primary book role.

Discovery shows all four methods together; its single run action uses their fixed order.

## D&D compatibility and editions

Open a system from the main menu for its Analysis dashboard. **Dungeons & Dragons** runs compatibility, editions, settings, levels, locations and dates in order. **OSR** runs system classification followed by per-system Adventure level ranges. **Shadowrun** and **Call of Cthulhu** each run editions, locations and dates. **Warhammer Fantasy** and **Warhammer 40k** each run games/editions, locations and calendar-preserving dates. Each dashboard has one **Run analyses** action, optional rerun-all, live saved totals and the same-position Cancel button. Existing results, including unknowns, are reused by default. Discovery does not run system-specific stages.

All four Discovery methods must be complete for the current catalogue before a system analysis can start. Saved unknowns count as complete; new captures or changed evidence can lock the system run buttons again. A checklist shows what remains and links back to Discovery. This gate also applies to retained individual backend commands and is rechecked after catalogue ownership is acquired. Opening a page sends no paid requests.

The whole window follows the selected system: warm gold and serif fantasy styling for D&D, black-and-white graph-paper styling for OSR, navy and cyan circuit styling for Shadowrun, dark green gothic styling for Call of Cthulhu, burgundy/brass for Warhammer Fantasy, and dark iron/red for Warhammer 40k. The flyout uses matching local SVG icons. Crawlers and Discovery restore the original palette; switching pages preserves running work and preferences.

Run **D&D compatibility → Classify books** across the catalogue. It distinguishes **D&D**, **D&D and others**, and **No**. Actual third-party D&D support counts; OSR, retroclones, Pathfinder, generic d20 and similar mechanics do not qualify by themselves. Concrete description evidence takes priority over messy source taxonomy. Supporting several D&D editions still counts as D&D only.

Next, the sequence runs **D&D editions** for books with either positive compatibility result. GPT-6 Luna uses one Decisions request containing nine independent checks, so a book may support several of: **0e, B/X, AD&D 1e, AD&D 2e, 3e, 3.5e, 4e, 5e 2014, 5e 2024**. Here 3e means original 3.0, not 3.5. Bare “5e” is not enough to choose 2014 or 2024. Supported editions and uncertain assessments are saved; no supported editions is a completed unknown result, reused by default.

These analyses use original metadata and descriptions, not previous analysis outputs; compatibility is only an eligibility gate for editions. Both share the existing parallel workers, retry policy, explicit reclassification and disabled Decisions caching. Changed source evidence or lost D&D eligibility invalidates dependent edition results. Edition counts may overlap; catalogue completion always counts books once.

## D&D settings, levels, locations and dates

The D&D sequence runs these four independent analyses after compatibility and editions. Each processes only books currently classified as D&D or D&D and others:

- **D&D settings:** one or more of the fourteen named settings in [the specification](SPEC.md#311-dd-settings-and-source-context), including Al-Qadim under Forgotten Realms, counted in both. **Any Setting** requires evidence of setting independence; **Unknown** means insufficient evidence. These are distinct completed outcomes.
- **D&D level range:** nullable `min_level` / `max_level` for player-character levels. Missing bounds stay null; explicit level zero and levels above twenty are supported.
- **D&D in-universe locations:** supported place names such as lands, cities or regions. No evidence produces an empty list.
- **D&D in-universe date:** a nullable string preserving calendar names, approximate dates and ranges. Publication dates do not count.

Settings uses Decisions; the other three use Responses structured output, starting on GPT-6 Luna. Responses caches common instructions; Decisions caching stays disabled. Only original saved metadata and full descriptions are supplied, never previous enrichment. Each analysis uses the existing CPU-based workers, retries and cancellation, saves completed unknowns, and skips current results unless you opt into a rerun. The dashboard cards show live totals across all runs. Source changes or lost D&D eligibility invalidate dependent results; changing a setting alone leaves levels, locations and dates intact.

## Warhammer Fantasy and Warhammer 40k

Two new flyout destinations offer **Games & editions → In-universe locations → In-universe dates**, with the same Discovery prerequisite, **Run analyses**, optional **Rerun all**, cancellation and live catalogue totals. Fantasy uses burgundy and brass; 40k uses dark iron and red imperial-gothic styling.

Candidates come from saved **Rule System** labels. The generic **Warhammer** label is checked for both families; explicit Fantasy/40k and named RPG labels go to their matching family. Titles alone do not select books. Games are independently classified, so a book can have multiple memberships:

- **Fantasy:** WFRP 1e–5e, Warhammer: The Old World Roleplaying Game, and Warhammer Age of Sigmar: Soulbound.
- **40k:** Dark Heresy, Dark Heresy 2nd Edition, Rogue Trader, Only War, Deathwatch, Black Crusade, Imperium Maledictum, and Wrath and Glory.

The classifier distinguishes **family confirmed, game unknown**, **family uncertain**, and **not this RPG family**. Locations and dates use confirmed family members, including unknown editions. Locations are bounded place-name lists, as for D&D. Dates stay text in their fictional calendars—such as `2512 IC` or `123.456.M41`—with unknowns saved as null. An Old World setting mention does not automatically identify the separate Old World RPG.

All inference uses original metadata and descriptions. Decisions caching stays off; structured extraction uses the shared cached Luna prompt and model escalation/retries. Current results are reused, including unknowns. Schema 16 adds separate results and a small metadata eligibility index on first launch; it preserves existing captures, archives and analyses and does not run any models automatically.

### Browse your 40k books

Warhammer 40k keeps its **Analysis** tab and adds a tab for each of the eight named games. Unknown, Uncertain and other classification-status groups have no tabs. A book supporting several games appears in each matching tab.

Tables show **Release Date, Name, Book Type, In Universe Date and In Universe Location**. Click a heading to toggle its sort direction; sorting applies to the whole category. Lists use 50-row pages, with missing values last. Book Type comes from Primary Book Role. Release Date uses saved availability/publication data; fictional dates sort as text. **Refresh books** picks up newly saved analyses.

Click a row or activate its name to open the saved description and metadata in a separate detail view. **Back** retains your sort and page. Browsing makes no API requests, runs no analysis, and works with existing schema-16 results without a migration.

## OSR systems

Open **OSR → Analysis**, between Dungeons & Dragons and Shadowrun in the flyout. The app switches to black ink on white graph paper, with squared panels and a dungeon-map icon.

Only books whose saved **Rule System** includes **Old-School Revival (OSR)** enter this analysis. This is the parent taxonomy label found in the existing catalogue; comparison ignores case and surrounding spaces. It does not classify the entire catalogue or select books merely because their description mentions OSR. Discovery must still be complete before starting.

**Run analyses** makes one Decisions request per selected book with fifteen independent compatibility checks. It can assign multiple categories: OSRIC; Castles & Crusades; Labyrinth Lord; Dungeon Crawl Classics; Swords & Wizardry; Old-School Essentials; Mork Borg; Into the Odd; Shadowdark RPG; Dragonslayer; Adventurer Conqueror King System (ACKS); Dolmenwood; Cairn; Vagabond; Other OSR Games.

Other covers supported games outside the named list or explicitly system-neutral OSR material and can coexist with named systems. Inspiration, related rules and easy conversion alone do not imply compatibility. When no category is established, the result is **Not OSR**; model refusal or failure remains unsuccessful. All inference uses original metadata and description, independently of prior classifications.

The dashboard shows the eligible subset and distinct analyzed books, plus overlapping category counts. Current results, including Not OSR, are skipped by default; optional **Rerun all** includes them again. Existing parallelism, automatic API-error retries and cancellation apply. Prompt caching stays off. The first launch builds the small OSR eligibility index without running any model; existing data/results are retained.

### OSR Adventure level ranges

The second OSR card analyzes only books already classified as **Adventures** with positive OSR system memberships. **Run analyses** checks system classification first, then fills missing level results. Each system gets its own range: a book can be OSE levels 3–5 and a DCC level-zero funnel. No conversion or automatic copying between systems occurs. Unlisted games under **Other OSR Games** retain separate names and ranges.

Missing information stays **unknown**. Explicitly level-free play is **not applicable**; vague difficulty labels and player counts never become invented character levels. Both outcomes count as analyzed and are reused. Open-ended ranges retain a null endpoint. The card counts distinct books; numeric/unknown/level-free status counts can overlap across systems.

This uses Luna Responses with prefix caching and the existing retry/model-escalation flow. Only original source evidence and the requested system names are sent. Source, Adventure role or membership changes invalidate affected results; identical system reclassification preserves them. Cancelling or failing a replacement keeps prior valid levels when their prerequisites are unchanged. Schema 15 preserves earlier classifications, results, history and archives.

## Shadowrun and Call of Cthulhu

Each system’s dashboard runs editions, **in-universe location** and **in-universe date** after the Discovery gate is met. Book eligibility uses the saved Rule System directly, independently of earlier classifications:

- Shadowrun: any Rule System label containing `Shadowrun`, ignoring case, including editions and Anarchy.
- CoC: `Call of Cthulhu`, ignoring case and surrounding spaces. This was the matching label in the existing catalogue; no CoC abbreviation was present. `Cthulhu Eternal` alone is excluded.

Location is a nullable string; several supported places are joined with semicolons. Dates are validated calendar ranges with inclusive endpoints and saved precision. For example, **2084** becomes **2084-01-01 → 2084-12-31**, and **1920s** becomes **1920-01-01 → 1929-12-31** (the boundary is the start of 1930). Unknown information stays null; publication dates do not count.

For CoC descriptions giving only an era, **Modern = 1980-01-01 → 2030-12-31** and **Dark Ages = 0950-01-01 → 1050-12-31**. These are fixed application conventions. Explicit in-universe dates take precedence; the app applies era-only ranges deterministically.

All four start with GPT-6 Luna Responses with shared-instruction caching, CPU-based workers, retries, cancellation, saved-result reuse and live catalogue totals. They send only original saved metadata/descriptions, never earlier enrichment. The existing Rule System eligibility index remains in place; this dashboard update introduces no migration or automatic analysis. Existing books, captures, archives and analyses are preserved.

The first stage, **Shadowrun editions** or **CoC editions**, classifies rules compatibility within the same fixed Rule System subsets. Shadowrun supports 1st–6th edition, original Anarchy and Anarchy 2.0; CoC supports 1st–7th. Each book uses one Decisions request with independent checks, allowing multiple editions or a completed Unknown result. The analysis uses original metadata and description only. Ambiguous Anarchy versions stay unknown; publication dates, supplement printing numbers and assumed backward compatibility do not establish an edition. Current results are kept unless you choose reclassification. The existing workers, retries, cancellation and live totals apply; Discovery completion is a workflow prerequisite only; its results are not model input.

## Adventure player counts

**Adventure player counts** is the final Discovery stage. It analyzes books currently classified as **Adventure** after the book-role stage, starting with GPT-6 Luna (`gpt-6-luna`) through the Responses API. Saved title and full description are sent in one initial paid request per book; invalid output can trigger additional requests on larger models. Shared instructions and examples are cached; changing book descriptions are outside the cache breakpoint. Actual cache and output-token usage remains recorded in backend history.

The saved `min_players` and `max_players` fields count players excluding the GM. A range such as 3–5 sets both; “at least 3” leaves the maximum null. Unsupported information stays null, without assuming a typical party size. Saved null results are complete and reused by default. Discovery’s **Rerun all analyses, including current results** explicitly includes them again. This analysis shares logical-CPU parallelism, retries and cancellation with primary book roles.

Saved bounds and provenance remain in SQLite. Changes to the description or applicable book role invalidate dependent counts; canonical captures and other analysis results remain intact.

## OpenAI Decisions

The retained standalone Analysis component includes **Test Decisions API** for checking the connection; it is not mounted in the current Discovery dashboard. It makes **one paid request** with a fictional book and three small questions, then shows the returned token usage. It never sends your catalogue, changes books, or runs automatically. Weighted relevance remains offline. Primary book-role and independent solo-play classifications are available; the first saved group browsing views are available in Warhammer 40k.

Set `OPENAI_API_KEY` in the environment inherited by the app. Restart the app after setting it; a key set only in a terminal is available only to programs launched from that terminal, not an already-running Start menu/desktop session. The key stays in Rust and is not saved in application files or sent to the frontend. Errors are shown without provider response bodies. The connection diagnostic does not retry; classification retries all API-request failures as described above.

The backend uses the [Decisions API](https://developers.openai.com/api/docs/guides/decisions) with `gpt-6-luna`, typed predicate/choice/score results, text or JSON evidence, bounded payloads, cancellation and usage reporting. Adventure player and D&D context extraction separately use the [Responses API with Structured Outputs](https://developers.openai.com/api/docs/guides/structured-outputs) on the same model. Developers can explicitly run `cargo run --example decisions_check` for the same one-request synthetic check; normal tests use local HTTP fixtures only.

## Desktop views

| View | Actions and information |
| --- | --- |
| Overview | Target selection; Start, Stop and Resume; catalogue/current-run totals; queue/discovery; website/API pacing; worker/parser activity; recent events |
| Authentication | Guided browser import, manual headers or anonymous access; Save only; Save & test access; clear credentials |
| Unsuccessful jobs | Current-run failures with IDs/reasons/timestamps, in bounded pages |
| Local storage | Fixed paths, validation state and retry |
| About | Versions and reprocessing saved responses for parser updates |
| Discovery → Analysis | Four coverage cards, one sequential run action, optional rerun-all, live progress and cancellation |
| Dungeons & Dragons → Analysis | Six analyses in a fantasy-themed dashboard, unlocked by complete Discovery |
| Warhammer Fantasy / Warhammer 40k | Games/editions, locations and calendar-preserving dates; themed analysis dashboards and Discovery gate; 40k also has game tables and saved book details |
| OSR → Analysis | OSR systems, then per-system Adventure levels; metadata subset, graph-paper styling and the Discovery gate |
| Shadowrun / Call of Cthulhu → Analysis | Three analyses each, with cyberpunk / gothic themes and the same Discovery gate |

The first five views are tabs inside **Crawlers**; each other destination has its own single **Analysis** tab. Each view has its own content. The main-menu flyout closes when you select a destination, click outside, press Escape or use its Close button; keyboard focus stays inside while it is open and returns on dismissal. Use Left/Right/Home/End to switch tabs, Tab/Shift+Tab to move through controls, and Enter/Space to activate them. The default window is 1240×820, with a 760×560 minimum and scrollable compact layouts. Color supplements labels and focus outlines.

Closing the window or using Close application stops new work, cancels diagnostics and waits for pending operations, the current analysis batch and the scheduler's graceful drain. Stop returns to idle. Abrupt process death remains recoverable through durable intents and OS locking. An already executing OS disk operation cannot be forcibly cancelled and can extend shutdown.

## Local storage

All application-owned runtime files use the fixed home-directory folder: typically `%USERPROFILE%\.kaijucrawl` on Windows or `~/.kaijucrawl` on Linux.

| Path | Contents |
| --- | --- |
| `settings.toml` | Target and browser/profile/container choices |
| `secrets.json` | Explicitly remembered manual cookies, separate from catalogue metadata |
| `catalogue.sqlite3` and SQLite sidecars | Products, queue, runs, outcomes, discovery and recovery intents |
| `archives/` | Immutable gzip files for successful product responses |
| `archives/.staging/` | Temporary archive/check files |
| `catalogue.lock` | OS writer lock; do not delete to force takeover |
| `webview/` | Desktop webview cache/profile |

No PostgreSQL, S3 or other network storage is supported. There is no legacy Python or remote-catalogue importer. Back up the complete folder with the app closed so the database and archives remain together. Unix permissions are owner-only; Windows files inherit the user's profile ACL. Remembered cookies are plaintext, not encrypted.

## Authentication

The wizard asks for a method, then the relevant browser/profile/container or website/API cookie headers. Back retains answers. New cookie inputs are masked and can be temporarily revealed; reviews always mask them. Existing saved cookie values stay in Rust and are represented in the UI by a **keep current headers** choice.

Manual headers apply separately to the exact website and API hosts. An optional `Cookie:` label is accepted. Browser-produced JSON cookie values are preserved; newlines, injected headers, malformed/truncated values and input above 1 MiB are rejected. Choose session-only or explicit remembering. Saving session-only, anonymous or browser mode clears old remembered manual headers.

**Save only** applies settings without contacting DriveThruRPG. **Save & test access** explicitly applies the draft, then checks robots policies, website access and optionally a known product. The diagnostic itself never writes product/queue/archive state or starts discovery. A supplied product ID is diagnostic only, never a seed; otherwise an existing known ID is used when available. Failed results return to the retained review. During an authentication pause, save corrected credentials, pass Test Access, then Resume run.

Browser selection is remembered and reimported at crawl start. Import reads only the selected current-user profile/container and relevant DriveThruRPG cookies. Domain, host-only, path, secure, expiry and supported first-party partitions remain enforced. Eligible `cf_clearance`, HttpOnly, empty-valued and session cookies are included. Firefox's current session snapshot supplements SQLite without overriding current values; historical backups are excluded. Values still held only in browser memory require manual paste.

Firefox first-party cookies partitioned to `https://drivethrurpg.com` are supported when their normal scope matches. Unsupported cross-site/private contexts are omitted with a value-free explanation. Implemented import paths include Firefox and Linux Chromium v10/v11 with Secret Service; Windows encrypted/app-bound Chromium paths offer manual paste. The application never modifies or terminates browsers or disables their protections.

Public HTTP 200 and cookie loading do not prove account authentication. Explicit session/login/challenge evidence still pauses the target; browser clearance does not guarantee HTTP-client acceptance. Test Access reports both robots scopes, HTTP status, eligible-cookie counts and import warnings without revealing cookie values or failed response bodies.

## Requests and runtime policy

Every website, sitemap, robots and product API request sends the fixed **Firefox 158 User-Agent** from SPEC.md §12.1, including retries and approved redirects. Bounded diagnosis found User-Agent-dependent API 502s with KaijuCrawl and 200s with this exact header. The internal **KaijuCrawl robots identity** remains separate, so exclusions and origin pacing still use KaijuCrawl's rules. This is a fixed compatibility profile, not browser-driven crawling.

Download and parser capacity match available logical CPUs, with fallback one. There is no Advanced menu or added request spacing. Website and API robots delays apply independently; a website-only `Crawl-delay: 5` does not slow the API. Server `Retry-After` cooldowns remain shared within the target, and transient retries use bounded backoff. Old Advanced/runtime overrides are ignored and disappear on the next settings save.

A product API **403 alone does not pause the crawl**. Generic forbidden, disabled and age-restricted products fail for that run and become eligible next run. Explicit session/challenge evidence and required website/robots checks retain their pause behavior. Failed live response bodies are never saved.

## Persistence and parser updates

Success requires verified product identity, title, full description and a verified durable archive plus committed metadata. Exact decimal prices and complete nested/raw metadata remain in SQLite. A save intent precedes immutable archive publication and decoded SHA-256 verification; only a fenced SQLite commit publishes success. SQLite and files do not share an atomic transaction. Restart reconciles pending intents idempotently.

Large-queue claims and totals use dedicated indexes and bounded batches. Discovery receives a worker slot alongside the product backlog. The dashboard distinguishes lifetime captures, current-run activity and discovery, without claiming a storefront completion percentage.

About → **Reprocess saved responses** applies parser updates without network access. It preserves original observation times and archives; failed reprocessing preserves the prior good record. Parser subprocesses receive no credentials and can be killed/reaped on deadline or cancellation.

## Development checks

```sh
npm ci
npm test
npm run build
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
npm run desktop:build
```

`cargo test --no-default-features` exercises the backend without linking a desktop runtime; that build is not a user-facing application. Default-feature checks include Tauri and the parser-worker executable. With `npm run dev` running separately, `cargo run --example desktop_fixture` opens the real desktop backend on a disposable directory with the target unchecked. `ui/test/preview.html` is a synthetic development-only preview; neither preview nor fixtures are included in the bundled production frontend.

Tests use synthetic fixtures, loopback HTTP and disposable storage. They never crawl live services or use the user's real catalogue. See [VALIDATION.md](docs/VALIDATION.md) for platform results, historical bounded live diagnosis and remaining verification limits.

## Invalid structured outputs

Player counts and all level/location/date extractions start each book on **Luna 6**. Invalid output advances that book through **Terra 6.1 → Sol 6.1 → Astra 6**, keeping the original metadata, description and schema. If Astra also fails validation, the book is recorded unsuccessful and the analysis continues. Existing good results remain intact; missing results can be retried on a later explicit run. Larger models use additional API credits.

Network, HTTP, rate-limit and capacity failures keep retrying the current model with backoff. Refusals remain unsuccessful without escalation. Prompt caching stays enabled for shared instructions; cancellation drains every model's requests. The existing totals block shows escalation/exhaustion feedback. Decisions-based classifications keep their existing Luna-only retry policy. No database migration or rerun of already-completed analyses is needed.
