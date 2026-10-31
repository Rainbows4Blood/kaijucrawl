# Validation evidence

Updated 2026-10-30 for the read-only Warhammer 40k game tables and book details in [SPEC.md](../SPEC.md) 2.25. Validation uses synthetic catalogues and UI fixtures. No production catalogue/archive reads or writes, API requests, paid analyses or computer-use tools were used for this change.

## 40k game tables and saved details

| Check | Result |
| --- | --- |
| Windows/MSVC workspace | 192 tests passed, none failed or ignored. |
| Linux/WSL2 workspace | 193 tests passed, none failed or ignored. |
| Rust quality | Formatting and workspace/all-target Clippy with warnings denied passed on both platforms. |
| Backend-only build | All 4 library integration tests passed with `--no-default-features` on Windows; 4 passed on Linux. |
| Frontend | All 47 Vitest/jsdom tests passed; TypeScript and Vite production bundling passed. |
| Windows release | Standalone Tauri executable rebuilt; help/version, isolated parser IPC, eight game labels, table/detail assets, new IPC identities, fixture exclusion and existing Start menu shortcut target checked. |

Four new backend tests use 113 synthetic books to verify the eight-game enum (including the second-edition wire name), rejection of status groups and invalid sorts, current membership/algorithm/scope filtering, overlap across game tabs, all five global sorts in both directions over multiple 50-row pages, tie stability, missing values last, source availability/publication fallback, cursor mismatch rejection, details scoped to the selected category, long saved descriptions, control stripping, raw metadata exclusion and indexed membership lookup. Read-only operations leave SQLite total_changes unchanged and remain available through a reader after another catalogue owner acquires the lock. Existing migration/capture/analysis suites pass without changing schema 16 or any saved analysis contract.

Four new UI tests cover deferred reads, all sort actions, paging state per game, table/detail replacement, Back restoring page/sort/focus, escaped hostile markup, empty/error/removed-book states, stale page rejection, eight named tabs plus Analysis, arrow/Home/End navigation and no paid command dispatch. The existing workspace test now expects nine 40k tabs and still verifies unique IDs, themes and the other dashboards. The description renderer retains plain text instead of interpreting HTML; no images or remote assets load. The production bundle excludes synthetic book previews.

Reads return current saved results and do not maintain a cross-page snapshot during concurrent analysis updates; Refresh restarts at page one. Fictional dates sort as stored text. Native WebView visual inspection was not performed under the owner's no-computer-use restriction; DOM/CSS tests do not establish native rendering.

## Previous Warhammer analysis baseline

Updated 2026-10-30 for Warhammer Fantasy and Warhammer 40k in [SPEC.md](../SPEC.md) 2.24. Automated tests use disposable catalogues and loopback HTTP. A read-only sample of 25,000 saved Rule System records reported aggregate label counts only; no production data was modified or sent to OpenAI. Six authorized synthetic requests checked the new API contracts. No storefront requests, catalogue analysis run or computer-use tools were used.

## Warhammer workspaces, games and context

| Check | Result |
| --- | --- |
| Windows/MSVC workspace | 188 tests passed, none failed or ignored. |
| Linux/WSL2 workspace | 189 tests passed, none failed or ignored; platform-specific tests account for the difference. |
| Rust quality | Formatting and workspace/all-target Clippy with warnings denied passed on both platforms. |
| Backend feature matrix | 2 system controller tests passed with `--no-default-features` on Windows; 2 passed on Linux. These include both new sequences. |
| Frontend | All 43 Vitest/jsdom tests passed; TypeScript and Vite production build passed. |
| Windows release | Tauri standalone release rebuilt; help/version, isolated synthetic parser IPC, production-asset exclusion, all six algorithm identities and existing Start menu shortcut target checked. |

Five new integration tests cover family-positive unknown versus uncertain/negative, multiple exact games/editions, contradictory/refused/partial output, all four strict context contracts, calendar-preserving strings, bounded names and unknown results; explicit shared-prefix caching and all four escalation models retaining source/schema; Rule System subset routing (including generic Warhammer in both families); source-only evidence; default reuse; per-book live deltas against aggregate totals; continued-positive reclassification preserving context; negative family/source/scope invalidation; indexed keysets; out-of-order commits; refusal preserving prior output; changed evidence and stale owner rejection. A schema-15 fixture retains UUID, product bytes/rowids/archive references/times and run history through schema 16. Existing older upgrade fixtures now pass through migration 016 while preserving their earlier result fixtures.

The common controller gate and sequence tests now include both Warhammer workspaces and every new route. They establish Discovery gating before requests, family-confirmed unknown eligibility, game → locations → date order under one ownership lifetime, saved unknown reuse without requests, cancellation of pending reruns and ownership reacquisition. Existing shared coordinator tests retain HTTP/network backoff, structured escalation/exhaustion, unsuccessful continuation and close/drain coverage. UI tests cover six themed system dashboards, eight menu destinations and keyboard focus wrapping, the Discovery gate, stage totals, rerun and same-position Cancel. CSS/DOM checks are not native WebView visual validation; that remains unperformed under the owner's no-computer-use restriction.

Exactly **six** fictional-book production API requests passed: two Decisions checks returned WFRP 2e + 4e and Dark Heresy 2e + Rogue Trader; four Responses checks returned the specified Fantasy/40k locations and retained `autumn 2512 IC` and `123.456.M41–125.001.M41`. Usage totaled **11,981 input** and **84 output tokens**, with **0 cached** and **0 cache-write tokens**. These one-per-method calls did not demonstrate cache reuse. Afterward, useful ambiguity examples expanded the stable extraction prefix above the documented [1,024-token caching minimum](https://developers.openai.com/api/docs/guides/prompt-caching); local schema/escalation tests passed with that final prompt, but it was not sent again. The opt-in `examples/warhammer_check.rs` helper caps each invocation at six requests, with no retries or catalogue access. These synthetic checks are contract evidence, not a general classification accuracy benchmark.

## Previous OSR Adventure levels baseline

Updated 2026-10-30 for per-system OSR Adventure levels in [SPEC.md](../SPEC.md) 2.23. Normal validation uses synthetic fixtures, disposable catalogues and loopback HTTP. Two owner-authorized synthetic Responses requests exercised the prompt. No production catalogue, archive or storefront was accessed; no computer-use tools were used.

## OSR Adventure levels per system

| Check | Result |
| --- | --- |
| Windows/MSVC workspace | 183 tests passed, none failed or ignored. |
| Linux/WSL2 workspace | 184 tests passed, none failed or ignored; platform-specific cases account for the difference. |
| Rust quality | Formatting and workspace/all-target Clippy with warnings denied passed on both platforms. |
| Backend feature matrix | Both OSR controller tests passed separately with `--no-default-features` on Windows and Linux. |
| Frontend | All 39 Vitest/jsdom tests passed. TypeScript checking and Vite production bundling passed. |
| Windows release | Tauri release build passed; help/version, isolated synthetic parser IPC, two-stage OSR dashboard assets, per-system prompt, migration, escalation and fixture exclusions passed. The existing launcher points to the rebuilt executable. |

Five new integration tests cover independent OSE/DCC ranges, level zero and values above twenty, open bounds, unknown versus explicitly level-free output, distinct unlisted games, refusal and malformed/partial/duplicate/out-of-scope rejection; fixed cached Luna requests and all escalation tiers retaining evidence/schema; exact OSR metadata plus Adventure/positive-membership admission; original evidence without unrelated enrichment; indexed keysets; out-of-order commits; distinct live versus aggregate totals with overlapping statuses; atomic multi-system replacement/rollback; unknown/default reuse and reopening; refusal/exhaustion preserving a good result; source/role/membership invalidation; changed-scope commit rejection and stale-owner fencing. A schema-14 fixture preserves catalogue UUID, canonical record bytes/rowids/archive references/times, classifications and prior relevance results across schema-15 upgrade. The previous OSR migration fixture now checks the current schema version.

The new controller loopback test runs system classification then levels under one ownership lifetime, dispatches a level request only for the Adventure, validates its exact requested system set and source-only evidence, retries HTTP 503 without changing the request, escalates invalid Luna output to Terra, saves both independent ranges, reuses the result without HTTP, and cancels a later pending level replacement. Identical OSR memberships and the earlier accepted Terra result survive cancellation; ownership is reacquired after drain. Existing shared tests retain Astra exhaustion/continuation and graceful Close coverage. No individual OSR-level UI/IPC or group browser was added.

Frontend tests now cover two OSR stages, separate Adventure eligibility/prerequisite counts, overlapping numeric/unknown/level-free totals without inflated book completion, stage transitions, current-result reuse, rerun and same-position Cancel. Native visual verification remains unperformed under the owner's no-computer-use restriction; DOM/CSS checks do not establish actual WebView rendering.

Exactly **two** fictional-book requests through the production Luna Responses path passed. The first returned six distinct entries: OSE 3–5, DCC 0–0, explicitly level-free Cairn, unknown Swords & Wizardry, Basic Fantasy RPG 5–7 and Knave 1–2. The second retained OSRIC 4–open and Swords & Wizardry open–7, ignoring player/page counts. Reported usage totaled **3,113 input**, **229 output**, **1,427 cached input** and **1,427 cache-write tokens**. The second request hit the shared instruction-prefix cache. `examples/osr_levels_check.rs` is an opt-in two-request helper with no retries, escalation or catalogue access. These bounded cases are integration evidence, not a general accuracy benchmark; production classification was not run.

## Previous OSR classification baseline

Updated 2026-10-30 for the OSR metadata subset, multi-system classification and graph-paper workspace in [SPEC.md](../SPEC.md) 2.22. Automated tests use synthetic fixtures, disposable catalogues and loopback HTTP. An owner-requested read-only inspection of saved Rule System metadata identified the eligibility label; three bounded synthetic OpenAI requests checked the new prompt. No production catalogue mutations, storefront requests or computer-use tools were used.

## OSR workspace and metadata subset

| Check | Result |
| --- | --- |
| Windows/MSVC workspace | 176 tests passed, none failed or ignored; the subsequently added OSR request-filtering/retry controller test also passed separately. |
| Linux/WSL2 workspace | 177 tests passed, none failed or ignored; the same additional controller test passed separately. Platform-specific cases account for the difference. |
| Rust quality | Formatting passed; Clippy across all workspace targets with warnings denied passed on both platforms, including after the final prompt adjustment. All four OSR storage/request tests passed again after that adjustment. |
| Backend feature matrix | The new OSR controller filtering/retry test passed with `--no-default-features` on Windows and Linux. |
| Frontend | 39 Vitest/jsdom tests passed. TypeScript checking and Vite production bundling passed. |
| Windows release | Tauri release build passed. Help/version, isolated synthetic parser IPC, bundled OSR policy/migration, subset/menu/dashboard assets, all system themes, reduced-motion support and fixture exclusions passed. The existing Start menu shortcut points to the rebuilt executable. |

The four OSR integration tests cover the fifteen-question shape, every supported category, multiple named memberships plus Other, complete negatives, refusals and malformed answers; exact case-insensitive/trimmed parent-label eligibility and duplicate metadata; rejection of child-name-only, genre/title and partial-label matches; source-only evidence; bounded indexed keysets; out-of-order commits; live versus aggregate totals; atomic replacements and rollback; default Not OSR reuse; source/eligibility invalidation; scope removal during a request; deletion and stale-owner rejection. A schema-13 fixture verifies preserved catalogue UUID, product rowid, record bytes, archive references, capture time and previous relevance results/history after migration to schema 14.

The shared system sequence/gate tests now include OSR, retaining current-result reuse, rerun cancellation, drain and lock reacquisition. The additional controller test proves that only the OSR-parent-tagged book reaches HTTP, that its request contains all fifteen independent checks and no prior enrichments/cache breakpoints, that a 503 retries the same evidence, and that a completed negative is reused without another request. Frontend tests verify the six-destination order, OSR theme selection, Discovery gating, run/rerun/cancel, and subset totals independent of the whole catalogue. Overlapping memberships do not inflate distinct completion; single-stage completion uses singular wording.

The read-only metadata sample contained 5,000 canonical books, including 125 with `Old-School Revival (OSR)` in `rule_systems`. All matching child systems observed in that sample included the parent. This establishes the chosen admission field, not complete taxonomy coverage. Production records were not sent to OpenAI.

Exactly three owner-authorized Decisions requests used fictional books and the production client. The first identified OSE and Swords & Wizardry but omitted Other for additional Knave support. The prompt was clarified; the two remaining checks then returned the expected OSE/Swords & Wizardry/Other memberships and Not OSR for an unrelated game with misleading metadata and inspiration-only references. Total reported usage was **19,049 input tokens**, **zero cached**, **zero cache-write** and **zero output tokens**. These are bounded integration examples, not an accuracy benchmark. The opt-in `examples/osr_check.rs` now contains the two final cases and performs no retries or catalogue access; normal tests never contact OpenAI.

Native visual verification was not performed under the owner's no-computer-use restriction. DOM tests and CSS bundling do not establish actual WebView rendering. The theme uses local CSS/SVG only; production migration and paid classification were not run against the user's catalogue.

## Previous system-dashboard baseline

Updated 2026-10-30 for the themed system dashboards and Discovery prerequisite gate in [SPEC.md](../SPEC.md) 2.21. This revision uses synthetic fixtures, disposable catalogues and loopback HTTP only. No production catalogue, storefront or OpenAI requests were used. No computer-use tools were used.

## System dashboards and Discovery gate

| Check | Result |
| --- | --- |
| Windows/MSVC workspace | 172 tests passed, none failed or ignored. |
| Linux/WSL2 workspace | 173 tests passed, none failed or ignored; platform-specific cases account for the difference. |
| Rust quality | Formatting and Clippy across all workspace targets with warnings denied passed on both platforms. |
| Backend feature matrix | Both new system coordinator/gate tests passed with `--no-default-features` on Windows and Linux. |
| Frontend | 36 Vitest/jsdom tests passed: 9 desktop flows, 5 Discovery cases, 8 system-dashboard cases and 14 retained standalone Analysis workflows. TypeScript and Vite production bundling passed. |
| Windows release | Tauri release build passed. Help/version, isolated synthetic parser IPC, production system menu/gate/theme assets, reduced-motion CSS and fixture exclusions passed. The existing launcher uses the rebuilt executable. |

New backend tests exercise all three fixed sequences, positive D&D prerequisite propagation, current unknown-result reuse without HTTP, explicit rerun cancellation, one ownership lifetime and lock reacquisition after drain. The gate tests reject every individual system entrypoint and all three system sequence commands with incomplete Discovery, then simulate stale preflight cache data to verify that both sequence and individual starts recheck owned current coverage before any request. A pure coverage test includes completed unknowns, missing methods/results, unresolved Adventure eligibility and zero-eligible methods. Existing individual system tests now seed their Discovery prerequisite through normal commit paths; their retry, extraction, escalation and cancellation assertions remain intact.

The final gate tests verify existing source invalidation through the canonical product fields. No extraction policy, algorithm ID, schema migration, canonical product or archive format changed.

Frontend coverage checks all five destinations, isolated Analysis tabs, matching whole-shell theme selection and local SVG icons, keyboard focus/dismissal, unique control IDs, default-theme restoration, locked checklist/Open Discovery navigation, completed-unknown acceptance, prerequisite-pending rejection, error-safe gating, one system start command, rerun-all, same-position Cancel, live stage transitions without aggregate polling and other-workspace activity blocking starts. The shared dashboard retains the existing Discovery stale-read and retry/error coverage. Synthetic development preview supports system sequences and is excluded from production.

Native visual verification was not performed under the owner's no-computer-use restriction. DOM tests and CSS bundling do not prove pixel layout or actual WebView rendering. Paid live testing was unnecessary because the model request builders and inference behavior did not change.

## Previous Discovery baseline

Updated 2026-10-30 for the Discovery dashboard and sequential analysis in [SPEC.md](../SPEC.md) 2.20. Validation uses synthetic data, disposable catalogues and loopback HTTP only. No real catalogue, storefront or OpenAI requests were used for this revision. No computer-use tools were used.

## Discovery dashboard and sequence

| Check | Result |
| --- | --- |
| Windows/MSVC workspace | 169 tests passed, none failed or ignored. |
| Linux/WSL2 workspace | 170 tests passed, none failed or ignored; platform-specific browser/ownership cases account for the difference. |
| Rust quality | Formatting and Clippy across all workspace targets with warnings denied passed on both platforms. |
| Backend feature matrix | 12 affected controller/analysis/progress tests passed with `--no-default-features` on each platform. |
| Frontend | 28 Vitest/jsdom tests passed: 9 desktop flows, 5 Discovery dashboard cases and 14 retained standalone Analysis workflows. TypeScript and Vite production bundling passed. |
| Windows release | Tauri release build passed. Help/version, isolated parser IPC, production menu/dashboard assets and fixture exclusions passed; the usual launcher uses the rebuilt executable. |

Three new controller tests exercise the complete four-stage order and newly classified Adventure eligibility; default score/classification/null-result reuse; explicit rerun; a refused replacement preserving the old good result while subsequent stages continue; HTTP 503 retry within the same stage; no system-specific requests; rejection of overlapping operations; whole-sequence cancellation; Close during a pending player request; durable per-stage history; local storage failure stopping later stages; and ownership reacquisition after drain. The existing missing-key test now also checks that Discovery starts no partial sequence before local credential preflight succeeds. A new SQLite test verifies bounded relevance reuse, null-score completion, missing/invalidated-only selection, force rerun, provenance retention and accurate live coverage. No schema migration or algorithm change was introduced.

Frontend coverage verifies all four cards and one run action without a selector, current category/eligibility totals, same-position Cancel, one backend sequence command with optional rerun-all, duplicate-start prevention, crawl/storage gating, live committed stage/coverage updates, stale aggregate rejection without progress-frame recounts, retry/escalation/unsuccessful feedback, and aggregate error recovery. Workspace navigation retains the masked crawler authentication draft and gates Start immediately after a Discovery start. The standalone analysis component remains tested independently. The synthetic development preview supports the new command and stays excluded from production.

Native visual testing is not performed under the owner's no-computer-use restriction. Automated DOM tests and CSS bundling do not establish pixel-level layout at particular window sizes. No paid live tests were needed because API requests, prompts and inference behavior were unchanged; sequence requests were inspected through local servers.

## Previous UI baseline

The crawler-only navigation evidence below records specification 2.19; its single-destination menu was subsequently extended by the Discovery change above.

## Crawler navigation restructure

All **22 frontend tests** passed: 8 desktop flows and the 14 retained Analysis workflows. The desktop coverage checks the single-destination flyout, focus containment/restoration, Escape/Close/outside-click dismissal, keyboard tab switching and wraparound during startup, correct tab/panel labels, masked draft retention when selecting Crawlers, isolated Test Access results, safe failure text/keyset paging, and continued Stop/Close access. Navigation makes no analysis or aggregate requests and does not start or stop crawling. The analysis tests mount the unchanged component directly rather than using the removed shell entry; its request, result, retry, totals, selection and cancellation assertions remain intact.

TypeScript checking and Vite production bundling passed. The Windows Tauri release is rebuilt for the existing launcher. Release help/version, isolated parser IPC and production asset exclusion checks use only synthetic input. Formatting, Markdown links/anchors, requirement numbering and credential-pattern review are checked separately. Rust source, migrations and the Analysis implementation/styles are byte-for-byte unchanged; the previous backend verification below was not rerun for this frontend-only change. Catalogue schema 13 and all existing results remain unchanged. Native visual verification is not performed under the owner's no-computer-use restriction; automated DOM tests do not prove pixel layout at particular window sizes.

## Previous backend baseline

The following platform results were recorded for the immediately preceding structured-output escalation revision (specification 2.18). Four bounded synthetic Responses requests exercised its model ladder; no production catalogue or storefront requests were made for that implementation. Earlier live evidence, including the separate owner-requested diagnosis of a failed book, is historical.

## Platforms and commands

Rust 1.99.0 is used on native Windows/MSVC and Ubuntu under WSL2. Formatting and Clippy with warnings denied passed on both platforms, together with the checks below:

| Platform | Result |
| --- | --- |
| Windows/MSVC | Full workspace suite: 165 tests passed, none failed or ignored. |
| Ubuntu/WSL2 | Full workspace suite: 166 tests passed, none failed or ignored; desktop linking and Linux-specific authentication/ownership regressions included. |
| Frontend | All 20 Vitest/jsdom tests passed (14 Analysis workflows and 6 desktop flows), using `npm test -- --maxWorkers=1 --testTimeout=15000`. TypeScript checking and the Vite production build passed. |
| Windows release | Schema-13 Tauri release build passed. Help/version, isolated parser pipe, production asset exclusions and bundled escalation model/feedback checks passed. The usual launcher uses the rebuilt executable; no production GUI was opened. |
| Backend feature matrix | All 50 affected tests passed with `--no-default-features` on Windows and Linux: 47 analysis/progress/classification/player/solo/D&D/Earth-context/Earth-edition/migration tests and 3 shared structured-response tests. |


The platform count differs because of conditional browser/lock tests. The complete suites include forced process termination at all four persistence boundaries.

The first Decisions Linux test build exhausted the tmpfs `/tmp` build directory before tests ran. The build cache was transferred to disk-backed storage, the temporary cache was cleaned, and the full suite plus the affected backend-feature tests then passed.

Parallel compilation during the classification revision hit the Windows paging-file limit (error 1455). Repeating the Windows workspace and affected backend-feature tests with two build jobs passed; no system settings were changed.

## Structured-output escalation, schema 13 unchanged

Five new Rust tests cover all eight structured contracts preserving original source/schema/cache settings across the exact four-tier ladder; compatible reasoning/output budgets; requested-model/snapshot identity; usage retained for rejected semantic output; Luna → Terra → Sol → Astra with a 503 retry remaining on Sol; cancellation while Terra is pending; and terminal Astra rejection continuing other books while preserving an earlier good result and leaving missing results eligible next run. The existing D&D-level and Shadowrun-date loopback tests now assert promotion to Terra after invalid fields/dates. Existing Decisions-only and refusal regressions remain in place. No algorithm versions or schema migration changed; older progress fields default cleanly.

The new frontend test displays last-escalation and exhausted-book feedback inside the existing totals block, preserving same-position Start/Cancel and avoiding a new activity panel. Production help/version, parser IPC and bundled-asset checks do not open the GUI or mutate the user's catalogue.

Exactly **four synthetic Responses requests**, one each using **gpt-6-luna**, **gpt-6.1-terra**, **gpt-6.1-sol** and **gpt-6-astra**, returned the expected valid 2055–2059 interval with range precision. Usage totaled **9,992 input**, **144 output**, **2,455 cached input** and **7,365 cache-write tokens**. `examples/structured_escalation_check.rs` is an opt-in four-request check with no retries or catalogue access. These samples verify model compatibility and the chosen semantic boundary, not general classification accuracy. Sol/Astra low reasoning and their bounded token budget were accepted by the API. Native visual testing remains unperformed under the owner's no-computer-use restriction.

## Shadowrun / CoC editions, schema 13

Five `tests/earth_editions.rs` cases cover metadata-only subsets, multiple independent choices, unknown reuse, atomic refusals, wrong/missing answer validation, overlapping memberships, independent reruns, indexed keysets, live per-book totals, preservation of canonical captures and context results, evidence/scope invalidation, unrelated-rating retention, transactional rollback, out-of-order completion, stale ownership and schema-12 upgrades. The shared D&D edition answer validator retains its regression coverage. Historical schema fixtures remove the two new group trees/triggers before reconstructing older versions. The Search-removal regression caught an unnecessary migration revision-counter update; that update was removed so unrelated existing state remains intact.

A new loopback controller case exercises both edition commands: one eligible book per metadata subset, seven/eight-question uncached source-only Decisions payloads, HTTP 503 retry of the same request, saved Unknown reuse, shutdown drain and ownership reacquisition. The new frontend workflow checks both dropdown entries, fixed scopes, overlapping counts, independent rerun flags, no implicit paid start, and the same-element Classify/Cancel action. Existing worker fan-out and all-request-error retry tests still apply unchanged.

Exactly **four synthetic Decisions requests** through the production client matched their expectations: CoC 6e+7e, an unspecified CoC edition despite a numbered scenario revision, Shadowrun 6e+original Anarchy, and Anarchy 2.0 alone despite a Sixth World setting reference. Total usage: **13,065 input tokens**, **0 output**, **0 cached**, **0 cache-write**. `examples/earth_editions_check.rs` makes at most four requests with no automatic retries or catalogue access. These small fixtures validate the request path and selected boundaries, not catalogue-wide model accuracy. Native visual testing remains unperformed under the owner's no-computer-use restriction.

## Shadowrun / CoC context, schema 12

Read-only, bounded 500-row keyset inspection of canonical Rule System arrays found one matching CoC label, `Call of Cthulhu`, and no CoC abbreviation. `Cthulhu Eternal` was separately identified and is not sufficient by itself. Shadowrun base/edition/Anarchy labels all satisfy the owner’s case-insensitive substring rule. Only matching label names/counts were reported; no book descriptions, credentials, response bodies or archives were read into test fixtures or sent to a model. The normal test suites never open the production catalogue.

Six new `tests/earth_context.rs` cases cover metadata-only scope (including mixed case, whitespace, both systems, nonmatching abbreviations, unrelated labels and absent metadata), location string/null contracts, valid typed calendar bounds, whole years/decades, leap-month endpoints, one-sided dates, invalid/reversed dates, inconsistent precision, and deterministic CoC era fallbacks. Both unknowns and known values survive restart/default reuse. Fixtures check unchanged canonical observations and prior analyses, explicit per-method reruns, live totals, source/eligibility invalidation, unrelated-result retention, rollback, duplicate commits, out-of-order interruptions, refusal preservation and stale owners. The schema-11 fixture upgrades with identity/data intact and verifies the scope keyset index. Historical migration fixtures remove the new schema before reconstructing older versions.

A loopback coordinator test starts all four routes without prior classifications, checks only eligible Rule System evidence reaches the model, retries an invalid calendar response, reuses saved unknowns and cancels an in-flight forced rerun before ownership release. The frontend workflow checks all fourteen dropdown choices, new Rule System descriptions without AI prerequisites, current catalogue totals, no automatic requests, the same Start/Cancel element, paid command arguments and independent rerun choices. An early migration-fixture failure referenced the wrong schema table name; the fixture now updates `catalogue`, and the final full suites pass. A UI assertion was also corrected to distinguish hidden D&D instructions from the visible selected analysis.

Exactly **six synthetic Responses requests** through the production client matched expected outputs: Shadowrun/CoC locations, the full year 2084, the full 1920s decade, CoC Modern and CoC Dark Ages. Recorded usage: **14,769 input tokens**, **154 output tokens**, **4,908 cached tokens**, **9,622 cache-write tokens**. The final two CoC date requests each reused 2,454 cached prefix tokens. `examples/earth_analysis_check.rs` is an opt-in six-request check with no automatic retries or catalogue access. No classification-accuracy or native visual-inspection claim is made from these fixtures.

## D&D settings and context, schema 11

Seven new tests in `tests/dnd_context.rs` cover the fifteen-question setting contract, overlapping labels, Al-Qadim parent membership, named-setting precedence, distinct Any/Unknown outcomes, refusals and malformed answers. Extraction fixtures cover nullable/partial ranges, explicit level zero, levels above twenty, invalid types/bounds, empty/duplicate/oversized location strings, calendar dates, strict schema/usage envelopes and stable instruction caching outside book evidence.

Disposable SQLite tests run all four methods over a fixed positive-D&D subset. They verify default reuse, forced reruns, current saved totals after each commit, independent results, source/prerequisite invalidation, retention after unrelated changes, atomic failure rollback, duplicate commits, out-of-order unfinished work, interruption/takeover and stale-writer rejection. The dependent-removal test also commits an actual positive-to-No D&D replacement and checks all four live totals against SQLite; this strengthened assertion passed separately on both platforms after the full suites. A schema-10 fixture upgrades with identity, canonical captures and prior analyses intact, adding empty results/groups without network access. Earlier migration fixtures now remove schema-11 objects when reconstructing historical versions; existing data-preservation tests still pass.

A new loopback coordinator test executes settings and all three structured extraction routes, verifies source-only evidence/caching, retries a malformed level response, reuses saved unknowns and drains a pending date request on Close while preserving its prior result. A frontend workflow checks all four dropdown entries, live saved totals including Any/Unknown and Al-Qadim/Realms counts, explicit paid commands, per-method rerun choices and the same Start/Cancel element. All existing player-extraction tests pass after extracting the common Responses envelope/transport.

Exactly **five synthetic live requests** used the production client: two Decisions requests and three Responses requests. The first fixture did not produce the expected setting membership; clarifying the fictional book description produced both Al-Qadim and Forgotten Realms. The final four checks then matched expected settings, levels 3–5, source place names and a calendar-qualified date. Reported usage totals **18,160 input tokens**, **62 output tokens**, **0 cached tokens** and **7,067 cache-write tokens**. Responses wrote each distinct instruction prefix once; no cache-hit claim is made from one request per extraction kind. These are integration checks, not a catalogue-wide accuracy evaluation. No source catalogue data or API key was printed or retained. `examples/dnd_analysis_check.rs` provides an opt-in four-request check, never invoked by normal tests or startup.

## Search removal and schema 10

The frontend has only Crawling and Analysis, with no Search, detail/filter components or result links. Tests cover both-tab keyboard navigation, disabled starts while storage opens, Close availability, explicit starts for all six analyses, reclassification choices, retained workspace selection, retries/errors, one live totals block and the same Start/Cancel button. Obsolete Search tests were removed; persistence and provenance assertions formerly reached through Search now read the saved analysis results directly.

`tests/remove_search.rs` reconstructs populated schema 9, including real FTS/facet data and a partially completed backfill. It checks removal of every Search object while preserving exact canonical rows, sparse rowids, identity, all six kinds of result, memberships, history (including opaque retired query fields), revision and dispatch identity. Unfinished dispatches are marked interrupted as before. Explicit reruns find the formerly unindexed gap and reuse current classifications; all six analyses and canonical invalidation work without Search tables. Existing desktop/persistence fixtures cover older TUI catalogues, archives, unfinished jobs, intents, settings/cookies, recovery and stale ownership. A controller test closes during blocked storage initialization and confirms a successor can open safely.

The migration test initially hit SQLx's dynamic-SQL audit requirement in its fixture snapshot helper; table names and schema columns are fixture constants, and the audited helper now passes. Clippy also required removing explicit auto-dereferences in test-only aggregate reads. No production code failure remained. The documentation link/anchor, requirement-numbering and credential-pattern review passes. No native visual inspection was performed, per the owner's no-computer-use instruction.

## Earlier feature validation

The following sections record prior revisions. Search-specific tests, UI, IPC and indexing are no longer present; retained analysis/persistence behavior is covered by the current suites above.

## Live totals and stable Start/Cancel placement

Two tests in `tests/analysis_progress.rs` compare transient live totals with authoritative SQLite aggregates after commits across all six analyses. They cover existing results, forced reclassification, known/unknown player-count replacements, overlapping edition replacements, dependent-result removal after prerequisite changes, refused results, and injected writes rolling back without publishing false progress. A 501-book score fixture verifies bounded batches, saved nulls, cancellation and reruns without double-counting. Run history does not persist these transient totals, and canonical records stay unchanged. Existing controller tests additionally assert the totals published through cancellation and the out-of-order D&D worker flow.

The frontend applies live totals immediately from the existing cached snapshot stream without a separate aggregate query or five-second gate. A deferred older aggregate response cannot overwrite newer committed totals. Tests switch the viewed analysis and hide/reopen the workspace without triggering aggregate reads during live work, retain a single progress bar, and assert that the same button element and parent represent Run and Cancel. Starting does not scroll to a different section. Full platform suites, affected backend-only tests and all 25 frontend tests passed, along with formatting, Clippy, TypeScript/Vite and documentation/credential checks. No live API calls, production data access or computer-use checks were needed. The schema and persisted result/history contracts are unchanged.

## Previous totals-only Analysis presentation

The preceding UI correction removed Latest Run Activity completely, including its progress bar, per-run counts, timestamps, worker activity and usage. At that revision, cancellation and saved-result navigation belonged to the single catalogue totals block; only active-analysis identity and actionable retry/local-error notices remain alongside those totals. Backend run history, storage and API behavior are unchanged.

All 24 frontend flow tests passed after updating the existing Analysis workflows. They verify one visible catalogue-wide progress bar during active work, no latest-run heading or per-run counts, cumulative category totals instead of newly-saved counts, selection changes without retargeting work, cancellation, actionable errors, refresh recovery and Search navigation. TypeScript/Vite production checks, Rust formatting, Markdown link/requirement consistency and credential-pattern review passed. That earlier change touched only frontend/CSS and documentation, so Rust Clippy and backend suites were not rerun then; the platform results above now cover the subsequent live-totals implementation. No live API requests, production data access or computer-use checks were needed.

## D&D classifications and catalogue coverage

AC-62–64 add four disposable SQLite tests and two controller tests. A 108-book fixture saves compatibility in separate runs, applies editions only to 72 current positive D&D results, stores multiple memberships or completed unknown results, and verifies whole-catalogue counts independently of latest-run counters. Edition category counts overlap while completed-book counts stay distinct. Search exercises matching any selected edition, status filters, keyset pagination, saved assessment provenance and revision-sensitive cursors. Evidence excludes all previous analyses and retains the original full description and allowlisted source metadata.

Schema-8 fixtures upgrade to 9 with catalogue identity, canonical captures and prior analyses intact. Tests cover independent groups, default result reuse, prerequisite/source invalidation, retention across positive-to-positive compatibility changes, unrelated metadata updates, refusal preservation, atomic membership failure, stale writers and unfinished out-of-order gaps. Loopback HTTP checks three-way compatibility and nine separate edition questions, identical evidence after HTTP 502/503, bounded workers, null/unknown reuse and Close draining workers before ownership release. A 2,501-book controller fixture verifies cached coverage reads, invalidation after offline results/capture deletion and no automatic model client or run.

The two additional Analysis frontend workflows verify all six dropdown methods, selecting another method during an active run, lifetime totals instead of last-run counts, prerequisite coverage, failed-refresh handling and explicit recovery. They also check separate paid start commands and offline D&D/edition filtering and provenance. No native GUI, computer-use or production catalogue was opened for this revision.

Exactly **14 owner-authorized live Decisions requests**, totaling **43,906 reported input tokens**, used fictional descriptions through the production client. The three compatibility cases (retroclone only despite misleading D&D taxonomy, actual D&D across multiple editions, and actual D&D plus Pathfinder) matched their expected categories. Initial edition checks exposed the model treating 3e as an umbrella for 3.5; a diagnostic and follow-up checks refined the exact target definitions and original-3.0/third-edition aliases. The final three checks all passed: 3.5-only returns only 3.5e, ambiguous “5e” returns no supported edition, and an explicitly nine-edition compendium returns all nine memberships. These small checks are integration evidence, not a catalogue-wide accuracy benchmark. No real catalogue data or storefront requests were used; only fixed labels and token totals were printed, and the key remained private in the test process environment.

## Independent solo play and analysis dropdown

AC-59–61 add three disposable SQLite integration tests and a loopback controller flow. A 72-book fixture includes previous primary roles, saved player counts, a relevance score and unclassified books; all 72 remain eligible. The actual evidence projection excludes prior analyses and raw metadata while preserving the source taxonomy and full description. Results and group memberships coexist independently, Neither is complete and reused, and Search filters, keyset pages, provenance and stale cursors use saved results only.

Schema-7 fixtures upgrade to 8 preserving catalogue identity, records, observation times, archives and previous analyses. Tests distinguish canonical-evidence invalidation from changes to other analyses, cover deletion cascades, preserve good results after refusal, roll back injected persistence failures and recover unfinished out-of-order gaps while rejecting a stale writer. The controller checks three concurrent requests without prior classifications, the three-choice Decisions wire contract with disabled caching, shared backoff after HTTP 503, identical retry evidence, exact counters, zero-request reuse and Close draining reclassification.

The eighth Analysis frontend workflow covers the native dropdown, no requests on selection, retained selection (active-run selector locking was replaced by browsable coverage in the D&D revision), per-method reclassification opt-in, catalogue-wide solo analysis despite Search filters, category totals and independent offline result navigation/filtering/provenance. Existing radio-based tests were updated to use the dropdown. No computer-use or browser automation is used.

Exactly four owner-authorized live Decisions requests used `examples/solo_check.rs` and fictional products: an external-system solo adventure, a self-contained gamebook with optional compatibility, a one-player-plus-GM duet and a generic solo oracle. All four matched the expected category, reporting **2,355 input tokens in total**. This is an integration smoke test, not an accuracy benchmark. The key was passed privately from the environment; no real catalogue, book description, cookie or storefront request was involved, and no provider bodies or credentials were printed or retained.

## Disabled prompt caching

The common Decisions encoder now sends `prompt_cache_options: {"mode":"explicit"}` with no marked input boundaries, following the official [Decisions caching contract](https://developers.openai.com/api/docs/guides/decisions#reuse-shared-input-with-prompt-caching). Existing loopback request assertions verify the actual JSON POST field and absence of a content breakpoint; existing text-encoding checks verify the same policy. Classification and retries share this encoder. Evidence, question instructions, saved-result versions and usage accounting are unchanged. Normal validation uses no real API key or production catalogue; no live cache-hit/write claim is made for this revision.

## Adventure player counts

AC-56–58 add three Responses/policy tests, five SQLite integration tests and one controller flow. Loopback assertions check Luna, strict nullable fields, developer-before-user input ordering, the shared cache breakpoint, no breakpoint on book evidence, bounded input/output, credentials, usage and no hidden request replay. Validation covers null, one-sided and exact ranges, refusals, invalid/negative/reversed counts, missing/extra fields and incomplete responses. The controller selects Adventures, handles an HTTP 502 with shared backoff while siblings save, retries identical evidence, skips saved nulls on rerun and drains pending reanalysis on Close.

Disposable schema-6 catalogues upgrade to 7 without changes to identity, records, timestamps or primary roles. An 80-book fixture selects only 60 Adventures and saves 40 known/partial ranges plus 20 unknown ranges. Default rerun selects none. Tests cover complete-known-range party filters, stable null-last pagination across both player sorts, saved provenance, description/role/deletion invalidation, unrelated changes, refusal preservation, failed writes rolling back counters/results, interrupted retry gaps and stale-writer rejection. The additional frontend workflow exercises explicit paid starts, fixed scope, reanalysis, unknown-result counts, cancellation, Search navigation, party-size filtering and nullable detail fields. No native GUI/computer-use checks were run, respecting the owner's restriction.

Exactly **four owner-authorized live Responses requests**, using `examples/players_check.rs`, sent fictional descriptions through the production client and policy. All returned their expected ranges: 3–5 plus a GM, unknown count despite character levels/pregenerated heroes, solo, and a lower bound only. Reported usage totaled **5,439 input tokens, 3,972 cached input tokens, 1,324 cache-write tokens and 80 output tokens**. Request 1 wrote the stable 1,324-token prefix; requests 2–4 each reported 1,324 cached tokens despite changing descriptions. This verifies this bounded smoke test, not guaranteed hit rates or catalogue-wide extraction accuracy. The Windows user-environment key stayed private, and no real catalogue/storage/storefront data was accessed or sent.

## All Decisions request errors

AC-55 now covers all API-request failures. Loopback recovery cases return HTTP 302/400/401/403/408/409/422/500/502/504, drop the connection before headers and midway through a response body, stall until timeout, return malformed JSON, or advertise an oversized response. Each case keeps the analysis running, waits at least the two-second base delay, retries identical evidence and questions, and saves exactly one result with two dispatches/one retry. Fixed failure reasons never expose provider text. The same transport still makes just one attempt per call, and the separate connection diagnostic has no automatic retry.

Shared two-hour Retry-After tests now include HTTP 500 and 401 as well as 429/503; successful siblings persist during the cooldown and Cancel/Close interrupts it promptly. An injected SQLite write failure stops admission, cancels outstanding HTTP and preserves the earlier committed result without retrying storage. Missing-key preflight still blocks paid work without affecting offline analysis. Fenced retry checkpoints now exercise connection and response errors, including interrupted-run eligibility. The frontend renders network, timeout, invalid-response and generic HTTP reasons alongside the existing countdown/counts and keeps Cancel enabled. All checks use fictional books and disposable storage; this revision made no live API requests or used API credits. No computer-use tools or production GUI/catalogue were used.

## Rate-limit and model-capacity retries

AC-55 adds two loopback controller tests, a fenced-storage test, a backoff-policy test, a Retry-After parser test and one frontend flow. A single fictional book receives 429, then 503, then a successful decision: the requests contain identical evidence, satisfy increasing delay minima, count three dispatches/two retries, and save exactly one result with only the received usage. A default rerun sends nothing. Shared two-hour Retry-After fixtures for both statuses block fresh work while successful siblings still commit; Cancel and Close interrupt the wait, clear live activity and release ownership with unfinished books eligible next run. No real API requests or production data were used.

Policy tests cover 2/4/8-second growth, the 300-second base cap, jitter, failure counts through `u32::MAX` without an attempt cutoff, longer server hints, sibling waits that cannot shorten a cooldown and extreme numeric durations without Instant overflow. Header tests cover integer seconds, future/past HTTP dates, rounding, invalid values and overflow. The transport remains single-attempt and never displays provider bodies. Storage tests reject duplicate/wrong-state redispatches, retain dispatch/retry counts across takeover, mark retry-wait rows interrupted and reject stale owners. The frontend shows 429/503 reasons, localized countdowns, pending books and retry totals while retaining Running/Cancel; cancellation clears the wait display without starting a request.

## Parallel classification workers

AC-54 adds three controller loopback tests with three-worker fixture capacity. They hold three HTTP connections open and reject a fourth until a slot is freed, deliberately complete requests out of order, and verify immediate refill before earlier requests finish. Five distinct evidence inputs complete exactly once, with exact saved counters and token totals; default rerun dispatches nothing and a forced rerun admits each book once. Separate tests close with several requests outstanding, or inject a SQLite write failure after a committed success (replacing the original API stopping fixture as request errors became retryable). Both stop admission, drain workers, report zero live activity and retain saved results; the next owner can process remaining books. No live API requests were made for this concurrency revision.

An additional SQLite test commits IDs 3 then 1 while leaving ID 2 outstanding. The completion high-water mark never regresses, duplicate result commits are rejected without incrementing counters, takeover preserves results and the next explicit run selects the unfinished gap. Historical progress without worker fields loads as one worker; activity reloads as zero. Existing atomic-failure and stale-writer tests remain enabled. The frontend flow verifies worker capacity and live activity, including zero activity after cancellation. Current result policy/model and catalogue schema remain unchanged.

## Primary book-role classification

AC-52–53 add seven disposable SQLite integration tests and four loopback controller tests. Schema-5 fixtures upgrade to 6 without changing catalogue identity, canonical captures, weighted scores or history. Storage tests classify 71 synthetic books across all four roles, verify one primary membership per book, traverse filtered Search pages and inspect saved model/confidence provenance. They cover default reuse with zero requests, explicit reclassification, None versus refusal, preservation of prior good results after an unsuccessful reclassification, relevant-source invalidation, unrelated updates, rollback/deletion, oversized evidence without truncation or dispatch, and stale Search cursors.

Injected persistence failure rolls back results, memberships, counters and token usage together. Durable dispatched-attempt state survives interruption; a successor reports interruption, preserves completed results and rejects stale writers. Controller tests verify explicit paid work, saved full-description evidence with raw metadata excluded, mutation exclusion while Search remains available, API-request recovery with safe failure details (replacing the original HTTP 500 stopping fixture), missing-key isolation from offline analysis, and Close cancelling a stalled request before releasing ownership.

The five Analysis frontend workflows cover both methods, catalogue-wide invocation independent of Search filters, absence of subset controls or a Search-to-Analysis handoff, reclassification opt-in, progress/usage/cancellation, role filtering and saved classification provenance. No computer-use or browser automation was used for this revision, respecting the owner's restriction. New controls reuse the existing responsive layout; earlier visual checks below are historical evidence only.

Four owner-authorized live `POST /v1/decisions` requests were made using `examples/classification_check.rs`, the production client/policy and four fictional books with deliberately misleading taxonomy. All four returned the intended primary role, with **1,850 reported input tokens in total**. This is a small integration smoke test, not an accuracy benchmark. The key was read privately from the Windows user environment and passed only to the test process. No real catalogue metadata, credentials, response bodies or storefront requests were printed, saved or sent as evidence. Normal tests never use the real key or contact OpenAI.

## OpenAI Decisions integration

AC-51 adds nine Rust tests against synthetic JSON and loopback HTTP. They verify the actual POST path/model/input representation, predicate/choice/score decoding, typed boolean versus string options, refusal preservation, answer identity/distribution/score/usage validation, invalid credentials/requests, payload bounds (including streamed bodies), safe 401/403/429/500/503 handling, redirect rejection, no automatic retries, cancellation while queued/in flight, a stalled-response deadline and desktop shutdown. A controller check rejects duplicates and Close waits for cancellation without even creating catalogue files. Tagged enum decoding is tested through response bytes with the crate's arbitrary-precision JSON feature enabled.

The frontend check verifies no request on opening Analysis, explicit click only, prevention of duplicate clicks, displayed input/cache/output usage and refusals, preservation across tab switches, and a missing-key error with no automatic retry. This revision did not use desktop or browser automation, respecting the owner's meeting restriction; new visual controls use the existing panel/button/scroll layout. The normal and compact visual checks below refer to earlier revisions.

The preceding Decisions foundation revision made one owner-authorized live `POST /v1/decisions` on 2026-10-28 using `examples/decisions_check.rs`, the same client and synthetic book/questions as the desktop connection test. It returned three validated typed answers, no refusals, **454 input tokens, zero cached tokens, zero cache-write tokens and zero output tokens**. The key was read privately from the Windows user environment and passed to the test process; no credentials or live response body were printed or saved. No real catalogue/book metadata, cookies or storefront requests were involved. Normal test suites never use this key or contact OpenAI. That connection test performs no group assignment or schema migration.

## Explicit Analysis and weighted relevance

AC-48–50 are covered by a pure formula test, five disposable SQLite integration tests, two controller lifecycle tests and the Analysis frontend workflows. The formula tests confirm the approved 4.7/1,000 versus 5.0/3 ordering and reject missing, nonfinite and out-of-range inputs. A genuine schema-4 fixture upgrades through the analysis migrations to the current schema with identical catalogue UUID, canonical records/times and existing Search results, with no scores calculated.

Storage tests analyze 65 internally filtered matches across multiple Search pages and all 130 captures, preserving source records and scores outside the subset. Internal selection remains available for future algorithm-owned scopes and historical run compatibility; the desktop exposes no subset choice. Relevance paging traverses scored ties and nulls exactly once; score updates reject an old cursor. Read-only queries/details leave score timestamps unchanged. Strict invalid-input handling, input invalidation, unrelated metadata updates, rollback and deletion are exercised. An injected failure on row 250 rolls back the entire batch and its counters. A committed 500-book batch survives an interrupted owner; a successor reports interruption, rejects the stale handle, does no automatic calculation and supports an explicit rerun. Cancellation retains completed scores across reopening, and empty internal selections complete cleanly.

Controller tests with 2,501 synthetic books prove explicit catalogue-wide start, duplicate/crawl/reprocess/storage-check exclusion, Search availability during analysis, cancellation between batches, and Close awaiting ownership release. Current frontend tests verify catalogue-wide submission independent of applied/draft Search filters and page cursors, progress across tabs, cancellation, failed starts, saved-score sort selection and displayed input provenance. In the earlier weighted-relevance revision, BrowserOS inspected normal and compact layouts at 1240×820 and 760×560 with synthetic data and no horizontal overflow.

A native Windows `desktop_fixture` run used six disposable captures and real Tauri IPC: opening Analysis performed no calculation; Run analysis completed six scores with zero skipped books; View scores in Search opened the Search workspace. Desktop automation then stopped at the owner's request. Saved-score ordering and detail provenance are covered by the backend/frontend tests above. No production catalogue, live storefront or Linux native UI was used for this revision.

## Desktop migration and compatibility

AC-43 uses a schema-3 fixture containing settings, remembered cookies, a completed capture, a failed job and an interrupted save with its archive already published. Desktop startup upgrades derived search storage while preserving catalogue UUID, record bytes/timestamps, archive bytes/references, settings, cookies and pending intents. The old capture is searchable through the controller. Recovery completes the interrupted save, the next run retries the failure and enqueues discovery, and successful products remain ineligible. Startup does not start network work. No production catalogue was opened by these tests.

AC-44 controller tests cover pending initialization, storage failure/retry, credential retention/clearing, secret-free snapshots, late-operation rejection and graceful shutdown waiting for both pending operations and ownership release. Frontend tests cover Start gating, wizard Back/masked review, failed access checks in an isolated view, escaped external text, keyset failure pages and Stop remaining usable during another operation. Parser subprocess tests still launch the desktop-linked executable's private worker entry point.

The earlier Tauri revision's BrowserOS checks exercised the synthetic frontend at 1240×820 and 760×560 sizes, including the dashboard, manual authentication steps and separate Test Access result. Its initial native fixture Common Controls error was fixed with the required Windows manifest, but the native automation pipe was unavailable that day. The Search revision adds the real native checks described below. The retired TUI's ConPTY/PTY tests are no longer part of the suite.

The production frontend contains only the application entry point and bundled assets; synthetic preview and test files are excluded. No shell, filesystem or HTTP frontend plugin is enabled. Request-header loopback tests cover every scope and approved redirects using Firefox, with independent KaijuCrawl robots evaluation.

## Search and saved-book overview

`tests/search.rs` contains eight tests covering exact decimal comparison (including exponents, negative values and zero), Unicode prefix text search and scopes, literal operator/injection input, combined OR/AND facets, missing values, flags, price/page/rating/date bounds and escaped facet prefixes. All nine sort orders traverse 123 tied/missing-value books in 50-row keyset pages without omissions or duplicates; changed filters invalidate an old cursor.

AC-45 creates 215 canonical captures, removes the derived schema to represent version 3, upgrades, and compares UUID, record bytes and timestamps. It then simulates a committed partial backfill and verifies recovery to completion. Reader tests verify uncommitted writes are invisible, rollback preserves search, and updates/deletes leave no stale FTS/facet entries. FTS integrity checks run against the external-content projection. Detail queries preserve exact prices, omit raw bodies, expose saved lifecycle/provenance and leave canonical records unchanged. Existing recovery/reprocessing tests run with the new transactional triggers enabled.

A text-plus-genre query over 20,000 synthetic books returned the first 50 rows, total and next cursor in approximately 50 ms in the Windows debug build, excluding fixture/index creation. An indexed title-plan assertion guards against a full temporary result sort. This is local synthetic evidence, not a latency guarantee for production-sized books or every filter combination.

AC-47 frontend tests cover Search during an active crawl, retained query/page after tab/detail navigation, escaped hostile text, exact money, combined filter submission, failed-draft retention, cursor paging, stale-result rejection and absence of automatic error retries on dashboard polls. BrowserOS visual inspection covered Search, expandable compact filters and saved-book details at 1240×820 and 760×560 with no horizontal overflow.

The filter update adds a disposable SQLite fixture with 88 distinct publisher values, a duplicated publisher and a case-insensitive tie across an options-page boundary. All values are reachable exactly once in bounded 40-value pages; returning to the first page is stable, literal `%`, `_` and `~` prefixes work, empty results are handled, and oversized inputs are rejected. Four picker tests cover lazy loading, retained checks across pages/prefixes/collapse, escaped labels, Escape focus, coalesced/stale requests, explicit error retry, the 20-value selection limit, missing values, single currency selection and disposal. Search flow tests verify chip removal, Reset all and retained checks through tab/detail navigation. BrowserOS inspected the new controls at both window sizes using an 87-value synthetic publisher list: the list scrolls within 220 pixels, all three pages are reachable, selection survives paging/search, and compact controls fit without horizontal overflow. No real catalogue or storefront was used for this update.

Native Windows UI automation exercised `examples/desktop_fixture.rs`, seeded with six verified synthetic captures in a disposable directory: open Search through real Tauri IPC, browse six books, search for Atlas and receive one book, inspect its saved description and exact `12.000000000000000001` price, return to results and close gracefully. The fixture had no selected target or credentials and made no HTTP requests. No Linux native mouse/keyboard run was performed; Linux compilation, linking and backend suites were executed.

### Startup progress and closing during indexing

The first index build previously showed storage failure/zero totals until all batches completed, and Close waited for the whole build. A 401-book fixture now cancels immediately after the first 200-row commit. It reports progress `[0, 200]`, returns cancellation, releases ownership and resumes as `[200, 400, 401]` with unchanged canonical records/times/identity. A controller regression closes during actual initialization, verifies real totals before completion, awaits exit within the fixture timeout and successfully opens a successor. The fixture includes successful jobs as well as records so lifetime metrics represent a valid catalogue.

Frontend tests distinguish loading, preparation and failure, display real totals and committed index progress, retain the progress panel in both workspaces, and keep Close available. BrowserOS inspection verifies the progress panel at 1240×820 and 760×560 without horizontal overflow. Cancellation waits for an executing SQLite batch or filesystem operation; the fixture timeout is not a hard deadline for blocked OS I/O.

The startup-fix release passed help/version, isolated parser and bundled-asset checks. Its attempted native relaunch was not verified because Computer Use's app-approval prompt timed out before launching it. The earlier native Search checks above and the current controller/SQLite cancellation regressions are separate evidence.

## Scheduling and persistence throughput

The previous queue refill sorted/scanned the pending backlog and repeatedly searched pending intents. It blocked the scheduler from harvesting completed jobs; dashboard aggregation also scanned wide job/product data. With 350,000 synthetic product jobs, 6,000 saved records and 14 pending intents, three native Windows debug-test measurements produced:

| Operation | Before | After |
| --- | --- | --- |
| Claim up to 16 jobs | 1,231–1,284 ms | 1.99–4.27 ms |
| Dashboard metrics | 311–323 ms | 26.9–29.7 ms |

Both measurements use the same fixture and build profile, with fresh disposable catalogues. They exclude fixture creation and initial migration time. Partial eligibility/budget/intent indexes eliminate the refill scan/sort; covering indexes and due-time seeks reduce dashboard work. Completed tasks are collected into bounded batches, and read-only ownership guards no longer acquire SQLite's writer reservation. FULL commits, archive publication and hash verification remain enabled.

A separate bounded loopback test saved 128 products in 2.14 seconds (59.8/s), using 16 download/parser slots, 25 ms synthetic HTTP latency, approximately 20 KiB response bodies, actual isolated parser processes, verified gzip archives and durable commits. It produced 13 save batches. This small warm local workload demonstrates pipeline capacity, not expected live storefront throughput; real API latency, response sizes, storage and rate limits differ. No eight-hour completion guarantee is inferred.

AC-40 checks large-backlog claims and counters, one concurrent sitemap slot ahead of old products, future due times, exhausted budgets, cooldowns, pending-intent exclusions and bounded end-to-end saves. AC-41 creates a version-2 fixture with a successful capture and unfinished intent, applies the scheduling and subsequent Search migrations and compares catalogue identity, records, metrics, archives and pending work. It also verifies that ownership reads work while a separate SQLite writer transaction is held. Existing stale-writer and process-kill tests remain enabled. The before/after throughput measurements above describe the earlier scheduling revision; the current suite also runs with search maintenance enabled.

Reproduce the local measurements (Windows app data is never used):

```sh
cargo test --locked --target-dir target/verification --test throughput -- --nocapture
cargo test --locked --target-dir target/verification bounded_local_throughput -- --nocapture
```

## Automatic runtime and product API failures

AC-38 is covered by configuration tests and the desktop's fixed controls. Both worker counts match the available logical CPU count. Synthetic old `[advanced]` settings and attempted `[runtime]` overrides cannot change workers, pacing or User-Agent; saves omit those controls while retaining target/browser choices. There is no Advanced menu or terminal shortcut handler. HTTP tests verify immediate pacing eligibility when neither robots nor server cooldowns require waiting.

AC-39 is covered at both HTTP and scheduler boundaries. Loopback product API responses exercise empty/generic 403, access-denied HTML/JSON, disabled products and age restrictions. They produce status-403 restricted failures without pausing, in-run retry or retained bodies. Explicit login/session flags, expiry messages and Cloudflare challenge headers/forms still pause, including mixed restriction/challenge evidence. Website and robots 403 retain strict checks.

The complete two-run scheduler fixture records three distinct API 403 failures, makes one attempt per product in the first run, verifies no save intents or archive directories for them, and successfully captures them on the next run when the fixture permits access. Earlier successful captures remain skipped. Reprocessing all six saved captures causes no additional HTTP traffic.

## Origin pacing and dashboard correction

Network regressions cover AC-37. Loopback HTTP serves different robots policies for the website and API: a five-second website `Crawl-delay` leaves the API immediately eligible, and an API `Request-rate: 2/10` leaves the website eligible. A concurrent request on the limited origin remains waiting without starting an attempt, while the other origin completes; cancellation releases the wait promptly. Refreshing one policy can remove its delay without changing the other. Server cooldown still applies to both origins, while another target's limiter remains independent. There is no added target request spacing.

The desktop displays effective origin delays, robots labels, automatic workers, unknown policies and cooldowns from controller snapshots. The complete scheduler scenario verifies that pacing reaches status and that reprocessing has no network pacing state. Formatting, Clippy with warnings denied and full Windows/Linux suites passed. Actual post-fix storefront throughput was not measured; no live crawl was started for this change.

```sh
npm ci
npm test
npm run build
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
npm run desktop:build
```

Windows checks use `--target-dir target/verification` to avoid overwriting a debug executable already open in the user's session. The Windows release build uses `target/release/kaijucrawl.exe`.

The initial Linux test build filled the WSL `/tmp` tmpfs with older debug artifacts. Only this project's disposable `/tmp/kaijucrawl-linux-target` cache was cleaned. Linux validation then used `CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0`, `CARGO_INCREMENTAL=0` and that same target directory. These change build artifact size, not the application feature set or assertions.

## Local storage coverage

| Area | Executed behavior |
| --- | --- |
| Fixed paths and settings | Home-directory resolution; no configurable storage/runtime fields; migration keeps target/browser choices and discards old provider/Advanced settings; new local settings take precedence and old files remain unchanged |
| Secrets | Cookies persist separately from settings/catalogue data; old cloud credentials are omitted; atomic replacement/clearing and corrupt-file errors; Unix 0700 directory/0600 file permissions |
| Startup | Automatic asynchronous initialization and totals; no storage wizard or auto-crawl; Start/duplicate checks gated while pending; failure preserves settings, permits retry, and location screen is read-only |
| SQLite | Actual bundled SQLite, versioned initialization, foreign keys, precise decimal TEXT and JSON; newer schema refusal; queue deduplication, attempt accounting, cooldowns and provenance |
| Exclusive ownership | Competing process/handle refusal, monotonically increasing generations, invalidated clones and rejected stale mutations; immediate acquisition after explicit release or forced process death |
| Archives | Local gzip roundtrip, no-clobber idempotency, decoded hash/length checks, traversal rejection and corruption refusal without overwriting the file |
| Recovery | Child processes are killed after durable intent creation, during partial staging, after archive publication and after final SQL commit; successors recover exactly once and never refetch completed captures |
| Atomic completion | Missing archive prevents success; a failed foreign key rolls back an entire save batch; only committed successes increment totals |
| Cleanup | Interrupted recognized staging files disappear on next exclusive acquisition; published archives remain available for recovery |
| Reprocessing | Failed parse preserves the prior good record; successful update preserves capture/check times and archive references; scheduler reprocesses six outdated captures without further HTTP traffic |

Linux validation exposed a transient lock-release race during parallel process creation. Explicit unlocking now precedes descriptor close. A Unix regression test keeps a duplicate descriptor open to model the brief inheritance window and verifies immediate acquisition by the next owner. Existing SQLite files are not opened/closed outside SQLite when setting file permissions, preserving SQLite's process-scoped file locks.

## Other retained coverage

- `src/scheduler/tests.rs`: complete loopback run with nested/gzip/duplicate sitemaps, storage pause before HTTP, real fixture HTTP 401 pause, cookie replacement/resume, bounded retries, next-run eligibility and no refetch of successful products.
- `tests/contracts.rs`: all metadata groups, exact decimals/raw numbers, verified identities, full-description completeness, malformed payloads, HTML gate evidence, bounded sitemap batches, per-target pacing and cookie scope.
- `tests/browser_import.rs`: Firefox WAL/container/first-party partition rules, expiry units, current session snapshots and JSON-valued cookies; inaccessible/corrupt/oversized files and unsupported Chromium encryption fallback. Linux additionally tests its Chromium cookie-decryption path.
- `src/http/tests.rs`: eligible cookies on both robots hosts, redirect isolation, unavailable/disallowed robots policies, challenge headers, bounded bodies/decompression and Retry-After.
- `src/controller.rs`, `tests/desktop_compatibility.rs`, `ui/app.test.ts`: existing-data continuity, serialized operations, credential handling, graceful drain, isolated desktop views and wizard behavior described above.
- Parser process/fault tests: real child roundtrip, malformed input, launch failure, cancellation and hanging workers killed/reaped on deadline.

All persistence and scheduler scenarios run in the normal suite. No Docker services, ignored remote-service tests or storage credentials are required. AC-27/AC-28 were removed with remote storage configuration; the new local path/migration and process-lock contracts are AC-35/AC-36.

## Earlier bounded live authentication evidence

Before this storage revision, an explicitly authorized Test Access run used the selected Windows Firefox profile (user-reported Firefox 158). Website robots, API robots and the homepage returned HTTP 200 in three requests, within the four-request cap. The imported current-session cookie supplemented database cookies; matching first-party clearance was eligible. Ordinary background challenge/CAPTCHA scripts no longer caused a false homepage denial.

Cookie values and response bodies were not printed, saved or added to fixtures. That run made no product, sitemap, non-robots API or production storage requests. Authentication remained **unverified**, and the product endpoint was **not tested**. These historical results do not establish current product API or account compatibility, complete storefront coverage, or support for every browser version.

## Later bounded product API 502 diagnosis

On 2026-10-27, the owner authorized read-only selection of a recent failed product from the local catalogue and live comparison requests after confirming that the exact API URL returned an uncached HTTP 200 in Firefox. The diagnostic used product 548229 only. Across five checks, 17 sequential requests were made: five API robots requests (all HTTP 200) and twelve requests to that product endpoint. No homepage, sitemap, catalogue mutation or archive write occurred. Response bodies were processed in memory only; no cookies or product content were printed or saved.

| Request identity | Observed product response |
| --- | --- |
| `KaijuCrawl/0.1.0`, HTTP/1.1, configured cookies or no cookies | HTTP 502 |
| `KaijuCrawl/0.1.0`, HTTP/2, configured cookies | HTTP 502 |
| Firefox 158 User-Agent, HTTP/1.1 and HTTP/2, configured cookies | HTTP 200; identity and completeness passed the existing parser |
| `Mozilla/5.0 (compatible; KaijuCrawl/0.1.0)` | HTTP 502 |
| Firefox User-Agent with KaijuCrawl appended | HTTP 502 |
| Firefox-compatible format with KaijuCrawl inside the comment | HTTP 502 |

The original header was checked before and after a compatibility variant, and Firefox controls were repeated. The observed difference is User-Agent-dependent; these results do not establish whether filtering is intentional or a gateway defect. Neither failing product response nor robots check included an explicit challenge header or Retry-After. The owner subsequently approved the exact Firefox 158 header for product API requests only. Production transport features are unchanged; the diagnostic's HTTP/2 feature is confined to an ignored helper project.

The initial AC-42 tests used Firefox on products only. The current revision extends that coverage to the same Firefox header on website, sitemap and both robots scopes, as well as approved same-product redirects. Conflicting KaijuCrawl/browser robots groups verify that permissions and the five-second pacing interval still follow KaijuCrawl, including denial before any attempt starts. Existing cookie, challenge, cooldown and rejected-redirect tests remain enabled.

After implementation, one additional bounded check used the production `Network` with its unchanged transport features and the selected browser's scoped cookies. Exactly three request starts were permitted and used: website robots, API robots and product 548229. Both policies loaded, and the product returned HTTP 200 with matching identity, title and full description according to the existing parser. This brings this incident's diagnostic total to 20 requests. The check had no catalogue/archive handles and saved no cookies or response bodies; it did not run discovery or a production crawl. This verifies one product at that time, not broader availability or account authentication.

## Limits

Forced process termination is tested; physical power-loss behavior on every filesystem/device is not. SQLite FULL synchronization and flushed archive publication rely on the operating system/filesystem honoring durability requests; directory synchronization is performed on Unix. A blocked OS filesystem operation cannot be forcibly cancelled by aborting a Tokio task.

Remembered cookies are plaintext local JSON under account permissions. Legacy settings/secret decoding is fixture-tested; the user's real configuration was not migrated by tests. Old remote catalogues/archives are not imported and no remote data is deleted. Browser-import and current storefront limitations described above remain separate from local storage correctness.
