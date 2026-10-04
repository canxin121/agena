# Tool modernization implementation and completion evidence

Objective: implement the improvements established by the tool-alternatives investigation, repair tool defects found during the audit, guide the model toward appropriate modern tools, and keep the system prompt concise and readable. This is an implementation ledger, not a replacement for that full objective.

Implementation worktree: `/Volumes/Rc20/Projects/agena-tool-modernization`; branch: `feat/tool-modernization`; source baseline: `98c1c116`. Research commit `a0e2c53e` was incorporated as `f5383449`. Do not use Agena connector tools for this work. There are no applicable AGENTS files at initialization. Work is performed without delegated agents.

The requirement table and final master-integration section record the modernization merge; subsequent product decisions are noted below. The phase narratives and prompt audit below retain intermediate measurements and failures as an audit trail; their earlier pending-work and publication statements describe those phases, not the final disposition. Research inventory: 22 plugins / 135 definitions. The corrected catalog at modernization completion was **22 plugins / 138 definitions / 131 execution tools / 7 manifest gateway handlers**, plus runtime-synthesized `tools_call` for **8 provider-facing gateway functions**. Earlier 134/4, 133/4 and 132/4 classifications in this ledger mistakenly counted the three `plugins_*` gateway handlers as execution tools; those historical statements do not mean tools were removed in the follow-up.

## 2026-10-05: retire managed workspace snapshots

Agena now records file changes and renders their diffs; Git owns file recovery,
version history, and worktree management. The `agena.snapshot` plugin, its
enter/exit/status tools, CLI command, HTTP endpoint, host callbacks, registry,
Rift/Git snapshot backends, and automatic directory pruning have been removed.
The current catalog is **21 plugins / 135 definitions / 128 execution tools /
7 manifest gateway handlers**, plus runtime-synthesized `tools_call`.

Old snapshot parts retain read-only rendering. Old static plugin settings and
profile patches are accepted and retired before host activation. Existing
workspace directories and saved session workspace paths are preserved; upgrade
does not discard files. Conversation rewind/fork and explicit Git operations
keep their separate semantics. Atomic writes and rollback within a failed file
operation remain correctness safeguards, not user-facing history restoration.

File records remain best-effort evidence from the tools that produce them,
not a universal journal of shell, external MCP, or background process writes.
The validation counts in earlier phase sections describe those historical runs.

Removal validation: 1,551 affected Rust tests pass (six optional environment or
benchmark cases remain ignored), including old-profile upgrades, legacy part
rendering, file diffs, failed-write cleanup, conversation forks and Git APIs.
The regenerated capability identity passes its drift check. Workspace all-target
`cargo check` and Clippy with `-D warnings`, formatting of changed Rust files,
Web type checking, and 37 Web diff/part presentation tests pass. The compiled
CLI help no longer lists `snapshot`.

| Requirement | Required evidence | Current status |
| --- | --- | --- |
| Modern CLI availability and routing | Runtime PATH-aware discovery, accurate identity/availability, bounded optional probes, deterministic tests; candidates accounted for | Implemented; actual PATH, aliases, cache, unset PATH, probe errors and candidate guidance tested |
| Unified command effects and exit semantics | Shared runtime/workflow classification; executable/subcommand/flag/quoting cases; no automatic rewrites | Implemented; command-shape and no-match/failure regressions pass; not a complete shell parser |
| Grep/glob completeness, output control and performance | Match/context/output/filter controls, visible truncation, bounded/cancellable work, equivalent-result benchmarks | Implemented and equivalence-tested; persistent Rayon search pool; serial glob retained after optimized profiling |
| AST and language-server improvements | Structural rules/rewrites, root-aware servers, revision checks, documented optional semantic backend | Implemented; local LSP fixtures, real plugin dispatch, stale/overlap/output-bound regressions pass |
| Search providers | Configurable structured and self-hosted providers; credentials/network effects; HTTP fixtures | Implemented; Brave/Tavily/Exa/SearXNG fixture-validated; live relevance/latency unmeasured |
| Browser modernization | Functional optional mature backend; session/permission/download ownership; actual browser tests | Implemented; native Chrome and optional Playwright; attached-target request checks have documented sandbox limits |
| Fetch/crawl/extraction | Audited third-party stack, useful extraction options, bounded output/cancellation and fixtures | Implemented; pinned bounded HTTP, managed rendered contexts, optional local Trafilatura; representative local fixtures pass |
| Local document content | Local conversion/search adapter, discovery, missing-dependency errors and format fixtures | Implemented; MarkItDown PDF/Office fixtures and plugin dispatch pass; Poppler not live-tested |
| Read/write/patch/notebook | Revision/publication contracts, batch bounds and notebook compatibility | Implemented; regular-file reads, observed-change/budget checks, staged revision validation, official nbformat schemas; external writers remain nontransactional |
| Shell/process/monitor lifecycle | Shared lifecycle and effect contracts, actual defect fixes, regression tests | Implemented; bounded shared collection, descendant cleanup, verified Darwin zombie handling and recoverable line framing; 20 parallel runtime-tools rounds pass |
| Memory, discovery, stateful and hosted tools | Inventory-wide disposition and affected correctness/contract tests | Audited; unchanged Tantivy indexes reused, committed results preserved on contention, discovery reads bounded, state/hosted contracts retained |
| Concise and formatted prompts | Reduced byte budget, usable call contracts, capability-aware workflow guidance | Audited and merged with upstream Git policy; core excluding Git 6,414 / base including Git 16,498 / all sections 21,247 UTF-8 bytes; production-schema examples, 24 capability combinations and real plan-state fixture; no sampled-model success claim |
| Optional analysis/display/log tools | Explicit candidate disposition, exact raw-output recovery, preserved status/diagnostics | Complete; curated optional CLI tiers and offline measured log experiment; no transparent output proxy |
| Generated contracts and documentation | Consistent schemas/reference/identity/configuration and usage docs | Complete; both generator examples reproduce the committed reference and identity snapshot byte for byte |
| Final validation | Targeted tests, workspace fmt/tests/clippy and invariant/capability gates | Complete on macOS for the merged source; 3,190 workspace tests pass, 12 deliberate skips; 593 web tests and production build pass with Bun 1.3.14; all-target Clippy, fmt, invariants, capabilities and generated comparisons pass |

Initial authoritative findings: core text search already uses ripgrep libraries; glob and grep use sorted serial discovery; AST uses ast-grep; search scrapes HTML; browser uses custom CDP; session.environment does not report host CLI availability. Workflow and shell analysis currently maintain separate read-only command lists. No implementation requirement is marked complete by these findings alone.

## Implemented foundation (not completion of the full objective)

- Shared shell/workflow classification recognizes rg, fd/fdfind, jq and task-specific modern CLIs. It distinguishes a simple command's no-match exit from compound/pipeline failures. Tests cover quoted separators, empty arguments, wrappers, Git global options, command execution flags and mutation flags. Further audit fixed find predicates after a leading `--`, multi-script sed, tree output, date setting, and Git pager/config forms. This is static command-shape classification, not a sandbox or a full shell parser.
- `session.executables` discovers curated candidates from the actual runtime PATH without executing them by default. Aliases, executable permissions, refresh, cache invalidation and bounded explicit version probes are implemented. An unset PATH reports no candidates rather than silently searching the workspace; platform/shell fallback paths are not guessed. Named version probes require 1–8 candidates; calls can query up to 32 names.
- `session.environment` includes compact CLI availability and reads Git facts with one porcelain-v2 NUL-separated snapshot. Unborn/detached repositories, rename source records and unknown facts have tests. Git failures no longer hide the rest of the environment.
- System prompt byte counts changed from 9,738 to 4,557 for the base and 16,811 to 7,392 with all workflow sections. These are UTF-8 byte counts, not tokenizer counts. Detailed CLI and grep controls live in tool help. See `tool-modernization-prompt-size.json`.
- `fs.grep` retains the embedded ripgrep libraries and adds literal/case modes, multi-include/exclude globs, context and content/files/count output. Counts refer to matching lines per file and are never silently capped at 500. Structured records separate path, line and content; old result payloads still decode/render. Exact result limits are complete unless another match is observed. Scan completeness and shortened text are separate facts. Per-read cancellation/deadline checks, a 256 KiB serialized-record budget, 4 KiB displayed-line limit, growth checks and regular-file/nonblocking opens bound work. Searcher buffers are reused across files.
- `fs.glob` adds kind/exclusion filters, cancellation and a result-byte bound while retaining deterministic pagination on an unchanged tree. The traversal remains serial. Deadline checks cannot interrupt an operating-system filesystem call already blocked in the kernel.
- Shared regular-file opening now protects read previews, attachments, patch input, instruction files, code search, filesystem plugin operations and notebooks from waiting on ordinary FIFO paths. Unix opens use O_NONBLOCK and recheck the opened descriptor; existing regular symlink reads remain supported. LSP synchronization has equivalent regular-file checks. Revision hashing is limited to 64 MiB and detects observed growth/changes. These changes do not establish race-free path authorization or external-writer transactions.
- File write/replace, notebook and patch replacements now revalidate the captured revision after staging and immediately before publication. Patch deletes and moves recheck contents before removing the source, and rollback refuses to overwrite a subsequently edited result. Conflict/rollback failures remain visible. Deterministic tests exercise external edits during preparation, after staging, and before rollback. The comparison and rename/unlink are separate system calls: noncooperating writers can still race that interval.
- Host executable results now have a dedicated table and usage details, including missing candidates and version-probe errors. Environment results render compact PATH availability. Jq exit-status interpretation skips option values rather than mistaking `--arg name -example` for `-e`.

## Measured search evidence

`tool-modernization-search-benchmark.json` records 21 alternating trials after two warmups over a deterministic 2,048-file / 18,063,664-byte corpus. The legacy serial loop and new collector run in the same debug test binary, and complete rows are compared every iteration. The benchmark is an ignored Rust test in `crates/agena-runtime-tools/src/tool/grep/benchmark.rs`; it starts no Agena service.

| Workload | Equal records | Legacy median | Enhanced median | Interpretation |
| --- | ---: | ---: | ---: | --- |
| Sparse literal-compatible regex | 16 | 321.38 ms | 346.86 ms | About 7.9% slower in this debug experiment |
| No match | 0 | 288.79 ms | 295.49 ms | About 2.3% slower in this debug experiment |

The new checks provide functional and resource-control benefits, but this experiment does **not** establish a speed improvement. Optimized builds, real repository corpora, glob costs and deterministic bounded parallelism remain required performance work. The baseline loop isolates traversal/search rather than the entire previous product pipeline. OS caches and host load were not isolated.

Reproduce from the worktree root:

```sh
AGENA_SEARCH_BENCH_OUTPUT=docs/research/tool-modernization-search-benchmark.json cargo test --locked -p agena-runtime-tools embedded_search_equivalence_benchmark -- --ignored --nocapture
```

## Verification log

- Existing foundation checks: shell classification (8 tests), CLI discovery (initial 4), identity prompt (3), dynamic prompt (3); session tests expanded to 5 and all passed.
- Search changes: initial 10 grep tests passed; subsequent runtime-tools suite passed all 156 tests, covering lifecycle, sandbox, file mutation and search behavior.
- Shared file opener and additional classification: all 72 agena-tool tests passed.
- A later runtime-tools suite encountered the existing monitor test's outer timeout equal to its two-second process timeout. Its isolated rerun passed. The test waiter now permits five seconds for process cleanup and queued reads; the product timeout is unchanged. The complete suite must be rerun on the final patch.
- Python 3.13: five refactor-tool unit tests and all 19 failure-semantics invariants passed. Runtime crypto/auth capability verification passed, including both real client-construction tests.
- The initial workspace test run completed with three failing targets, all caused by the added tool's missing human projection or hardcoded 135/131 totals. Those defects were fixed. All other workspace targets passed on that run; this is not a final workspace-wide pass.
- Foundation rerun: `cargo test --locked -p agena-tool -p agena-runtime-tools -p agena-bundled-plugins --no-fail-fast` passed 444 tests, with the benchmark intentionally ignored. This includes 160 runtime-tools tests, 72 tool tests and 212 bundled unit/integration tests. Formatter, diff-whitespace and Clippy (`--all-targets -- -D warnings`) checks passed for those three crates.
- Generated reference and identity snapshots now record 22 plugins, 136 definitions, 132 execution tools and 4 gateways. The original research's 135 remains its baseline count.

## Historical next-work checkpoint after the foundation

At the foundation checkpoint, the remaining areas were not satisfied merely by the CLI catalog. In particular, finish validation of structured search providers and implement a functional optional mature browser backend, local document adapters, AST rule/rewrite extensions and fetch/extraction options. Account for optional analysis/display tools deliberately, retaining raw logs and exit codes for any compression experiment.

Publication-time revision checks and nbformat reference validation have been added; the remaining file audit is still required. Even a final comparison cannot make arbitrary external writers participate in the sidecar-lock protocol; state that boundary honestly.

## Structured search providers

Foundation implementation is committed as `dea9c58f`. The next changes add configurable Brave, Tavily, Exa and SearXNG adapters to `web.search`, with the existing HTML route as the default. Configuration and usage are documented in `docs/web-settings-workbench.md`.

- Official request schemas were checked against Brave web-search docs, Tavily's search OpenAPI, Exa's search OpenAPI and SearXNG's `docs/dev/search_api.rst` on 2026-10-04. No paid provider was called.
- API credentials remain environment variables and sensitive headers. HTTPS is required for authenticated APIs. Private SearXNG needs an explicit endpoint, its private-network setting and a runtime network-policy allowance. Search calls now honor the host's network permission before performing network I/O; endpoint connections pin the validated DNS addresses, disable environment proxies and do not follow redirects.
- API errors do not cause automatic retries or cross-provider fallback. Errors retain status and numeric retry delay without echoing response bodies or credentials. Bounded streaming works with or without Content-Length.
- Result normalization covers provider attribution, domain allow/block filters, duplicate URLs, malformed rows, SearXNG partial failures, text clipping and provider-reported usage. Four request/response shapes, status/JSON errors, budgets, timeouts, DNS pinning and the actual SearXNG plugin dispatch are covered by local fixtures. Live relevance, paid-service availability and comparative latency remain unmeasured.
- Verification: 237 tests passed across all `agena-web` and `agena-bundled-plugins` unit/integration/doc targets. Clippy passed for those crates plus `agena-runtime-tools`, including all targets and `-D warnings`. Generated references and identities were refreshed; the only identity change is `web.search`'s documented input bounds/help/network tag. Source retrieval hashes are in `tool-modernization-provider-sources.json`.

## Notebook reference implementation and compatibility

Structured search was committed as `fb84eb81`. The notebook audit then found that hand-written validation rejected/deleted valid raw-cell attachments and that inserted cells always received an `id`, even in older nbformat 4.0–4.4 files where it is an unknown field.

- The editor now preserves attachments across raw/markdown edits and conversions, removes them when converting to code, and adds IDs only for notebook minor versions 5 and newer.
- Six unmodified official schemas from nbformat 5.11.1 replace the incomplete hand-written field checks. Their BSD-3-Clause license, source/version and SHA-256 hashes are included alongside the schemas. Runtime validation uses the workspace's existing Rust `jsonschema` implementation; Python is only a test reference dependency.
- Future minor versions follow nbformat's additional-property/unknown-cell/unknown-output relaxation, preserving fields the current implementation does not understand. Editing an unknown target cell type requires an explicit supported type. Duplicate IDs remain an explicit conflict instead of accepting nbformat's warning-and-repair behavior.
- Twenty-four positive/negative reference cases cover all six current minor versions, attachment/MIME rules, notebook/cell metadata, unknown fields, output shape, duplicate IDs and future minor versions. Twelve real edited files (replace/insert for all six minor versions) passed Python nbformat validation. An invalid unedited MIME output prevents publication.
- All 149 bundled unit tests passed. The full bundled run had one integration failure because its old assertion required raw attachments to be deleted; that expectation was corrected, and all three notebook correctness integration tests passed afterward. All other integration/doc targets passed in the full run. Clippy passed with all targets and `-D warnings` after the correction. No final workspace pass is claimed yet.

Reproduce the independent reference check (with Python 3.13 available):

```sh
AGENA_NOTEBOOK_REFERENCE_OUTPUT=/tmp/agena-notebook-edits.json cargo test --locked -p agena-bundled-plugins --lib raw_attachments_and_legacy_ids_survive_cell_edits
uv run --no-project --python 3.13 --with nbformat==5.11.1 python tools/verify_notebook_reference.py --outputs /tmp/agena-notebook-edits.json
```

## Optimized search measurements and parallel implementation

The serial optimized benchmark completed (`tool-modernization-search-benchmark-release.json`): sparse-match medians were 84.306 ms legacy / 84.481 ms enhanced (+0.2%), and no-match medians were 85.546 / 87.055 ms (+1.8%). These show near-parity for this corpus, not a speedup.

The first parallel prototype reserved byte budgets before dispatch, used at most sixteen jobs per batch and four workers, merged in sorted discovery order, and counted speculative reads. Tests cover context ordering, exact limits across batch boundaries, disjoint byte budgets and changed-file reservations. All 163 runtime-tools tests passed (two opt-in benchmarks ignored), and Clippy passed with `-D warnings`.

That prototype recreated OS threads for each batch. Its optimized synthetic sparse-match median regressed (81.608 ms legacy / 94.133 ms enhanced), while no-match improved (76.611 / 68.021 ms). The actual repository experiment improved both workloads (61.180 / 53.648 ms and 86.810 / 76.407 ms). Complete rows were equal every iteration, but the samples show substantial host-load variance. Raw measurements are retained in `tool-modernization-search-benchmark-parallel-release.json` and `tool-modernization-search-benchmark-repository.json`; the earlier debug gains are not a product performance claim.

The implementation now uses a dedicated persistent Rayon pool, limited to four threads process-wide, instead of per-batch thread creation. It preserves the same budget and ordering contracts. Fourteen grep regression tests passed, followed by all 163 runtime-tools tests (two opt-in benchmarks ignored) and Clippy with `-D warnings`.

| Persistent-pool optimized workload | Legacy median | Enhanced median | Legacy / enhanced |
| --- | ---: | ---: | ---: |
| Synthetic sparse match, 2,048 files | 36.211 ms | 34.674 ms | 1.04× |
| Synthetic no match, 2,048 files | 41.898 ms | 43.427 ms | 0.96× |
| Repository sparse match, 1,234 Rust files | 33.674 ms | 32.234 ms | 1.04× |
| Repository no match, 1,234 Rust files | 56.733 ms | 42.829 ms | 1.32× |

`tool-modernization-search-benchmark-pool-release.json` and `tool-modernization-search-benchmark-pool-repository.json` retain all 21 samples, corpus/result hashes and equality checks. Results are mixed: no uniform speedup claim is supported. Do not compare absolute timings across separate runs as controlled measurements. The dedicated pool avoids the thread churn of the prototype and caps aggregate worker threads across concurrent searches; sorted discovery and bounded merge remain deliberate constraints.

The parallel search phase was committed as `c3550adf`. The subsequently discovered AST truncation and LSP routing defects have been repaired in the semantic-tools phase below.


## Glob profiling and deliberate serial traversal

The optimized ignored benchmark `tool::glob::benchmark::repository_glob_equivalence_benchmark` compares the legacy walk/filter loop with the bounded implementation on the current worktree's `crates/`, using normal ignore rules. It verifies every returned path and page-completeness flag on every iteration, with two warmups and 21 alternating rounds.

| Workload | Records | Legacy median | Enhanced median |
| --- | ---: | ---: | ---: |
| Rust paths, offset 0 / limit 200 | 200 | 2.654 ms | 2.538 ms |
| Rust paths, offset 1,000 / limit 1,000 | 240 | 14.203 ms | 13.961 ms |
| No matching path | 0 | 15.317 ms | 15.683 ms |

`tool-modernization-glob-benchmark.json` retains the samples and limitations. These small mixed differences do not establish a speedup or regression. Keep sorted serial discovery for stable bounded pagination: parallel traversal would need to collect/sort more of the tree before safely returning the first page. Filesystem/load/cache isolation and cross-platform measurements were not performed.

## AST rules, rewrite planning and language-server correctness

See `../structural-code-tools.md` for invocation examples and operational boundaries. The official ast-grep 0.44.1 rule/fixer implementation is now embedded alongside the existing core/language crates. Retrieved source hashes are in `tool-modernization-semantic-sources.json`.

- `code.search_ast` accepts exactly one pattern or structured rule, with bounded rule size/depth/node count. Broad simple patterns remain compatible. Composite/relational rules use the upstream parser/matcher. Search discovers one additional result before claiming truncation, marks shortened text, and sorts file discovery before its limits. Failed I/O/UTF-8 reads consume the global content budget; per-file reservation and limited reads enforce the remaining budget (plus a boundary-probe byte). Observed changes while reading are rejected.
- `code.syntax_tree` adds a 512-node total cap, retains the 50-child and depth limits, and explicitly marks omitted descendants. Oversized files are rejected before content allocation.
- `code.rewrite_ast` defaults to a no-write preview containing an exact-source diff and revision hash. Applying requires that hash, uses the shared file lock/staged publication/check, and reports file-change effects only if a write occurred. It rejects overlapping edits, undefined replacement variables, parse errors, stale revisions, more than 100 matches, and output over 8 MiB. It intentionally accepts only one file. As with other publication helpers, a noncooperating external writer can race the final comparison/rename interval.
- The real plugin route exposed an optional-field validator defect: a rule-only request was wrongly required to provide a nonempty pattern. The input contract now requires nonemptiness only when the optional field is present; the regression passes through the actual PluginHost/ToolExecutor JSON boundary.
- LSP routing gives explicit extensions priority over catch-all servers and resolves ties by server name. Client/startup caches use server plus canonical root; a changed specification drains its old instances. Existing startup locks and shutdown gates remain in force.
- `lsp.servers` inspects configured PATH commands using the existing mature `which` dependency, without executing them, and reports running roots. Discovery runs on a blocking worker and is explicitly a file-presence check, not initialization/semantic readiness. Windows per-server PATHEXT overrides report unknown rather than guessing.
- LSP position help now states zero-based UTF-16 input semantics; local URI display correctly decodes spaces, Unicode and literal hashes. AST human rendering uses the actual start-line/start-column fields; rewrite results render a preview/application state and diff.
- Ten initial AST tests, one rewrite publication test, and the real plugin dispatch test passed. A later additional test covers invalid UTF-8 charging the shared read budget. All eight LSP tests passed, including an actual Python stdio peer proving per-root initialization, canonical aliases, twelve-way startup deduplication, configuration replacement and child cleanup while client handles remain retained. No external LSP service or Agena connector was called.
- The first combined run passed 471 tests across bundled/tool/runtime-tools/LSP (three benchmarks intentionally ignored). After the read-budget fix, all 464 bundled/tool/runtime-tools tests passed (three benchmarks ignored); the unchanged LSP suite had already passed all eight tests, for 472 passing tests across this phase. Clippy passed for LSP, tool, runtime-tools, bundled-plugins and runtime with all targets and `-D warnings`. A Clippy module-placement finding in a test was fixed. Generated references/identity snapshots now contain 22 plugins, 137 definitions, 133 execution tools and 4 gateways. No final workspace-wide pass is claimed.

At the semantic phase checkpoint, browser, document conversion, fetch/extraction, the remaining file/process/stateful audits, optional analysis/log-tool disposition, prompt review and final validation were still outstanding. Their later disposition is recorded below.


## Local PDF/Office extraction and search

The semantic-tools phase was committed as `98dd42b8`. `fs.document` now supplies a functional local adapter for PDF, DOCX, PPTX and XLSX, with optional line search and bounded previews. It prefers Poppler's `pdftotext` for PDFs when available, otherwise invoking selected MarkItDown converter classes through an isolated Python process. Setup and limitations are documented in `../local-document-tools.md`; official source observations are hashed in `tool-modernization-document-sources.json`.

- Runtime dependency installation is never automatic. `AGENA_DOCUMENT_PYTHON` selects a configured Python environment; missing interpreters or format extras yield actionable errors. No MarkItDown plugin, URL auto-routing, audio/image transcription, LLM client or hosted conversion configuration is enabled.
- Source files are opened as regular files, limited to 32 MiB, checked for observed changes, hashed and copied into a private temporary directory. This phase also fixes the shared mutating/read helper to reject oversized metadata before allocating/reading and to check observed changes afterward.
- At most two converters run concurrently. Existing managed process-tree execution supplies timeout/cancellation behavior, a 30-second process deadline and 2 MiB per stream cap. File discovery/copy and executable resolution run on blocking workers. Expanded Office archives are limited to 4,096 entries / 64 MiB declared expanded bytes. These are resource bounds, not an arbitrary-Python sandbox.
- Literal/regex modes, case handling, line offsets, line counts, exact limit semantics, UTF-8 clipping and a 128 KiB record budget are implemented. Converter warnings remain visible. Conversion success is distinct from extraction quality or full layout/OCR understanding.
- MarkItDown 0.1.8 was installed only in `/tmp/agena-document-adapter-env` for verification. The ignored real-plugin test passed for all four formats, including Unicode in DOCX/PPTX/XLSX. `tools/document_adapter_fixtures.py` reproducibly creates the inputs; fixture outputs are in `/tmp/agena-document-adapter-fixtures`. No Agena service, connector or hosted conversion was called. Poppler was absent, so its branch has not had a live conversion test.
- Three dedicated document tests passed. The first broad run exposed a stale filesystem-manifest name expectation and a missing completed-title result fact for the new tool; both were fixed. All 153 bundled unit tests and all 11 human rendering tests then passed. Every other target in the broad bundled/runtime-tools/tool run passed, including 164 runtime-tools tests and 78 tool tests. This totals 467 passing tests across those targets after the corrections, plus the opt-in real-converter test. Clippy passed for all three crates with all targets and `-D warnings`. Generated references and identities now have 22 plugins, 138 definitions, 134 execution tools and four gateways.

At the document phase checkpoint, browser, fetch/crawl/extraction, the remaining lifecycle/state audits, optional tooling, prompt review and final validation were still outstanding. Later sections record their implementation.


## Optional Playwright interactions and browser/process lifecycle

The document phase was committed as `0a0dc9fa`. The browser now has an explicit `browser.interaction_backend=playwright` option (default: `native`). It uses the installed Python Playwright API over CDP for click/fill/wait, retaining the native context owner, page gates, navigation interceptor, redacted snapshot format, artifact/download ownership and shutdown. Configuration is documented in `../web-settings-workbench.md`; official source observations are in `tool-modernization-browser-sources.json`.

- Playwright supplies strict CSS locators, actionability and visible-selector waiting. Snapshot refs still validate the original main-document identity/signature. Optional `frame_selector` addresses one iframe by CSS; it cannot be mixed with snapshot refs and requires Playwright. Native action semantics otherwise remain available. A failed action never silently retries through another backend.
- `AGENA_BROWSER_PYTHON` selects a prepared Python environment. No runtime dependency/browser installation occurs. Each action has one temporary bridge connection to the already owned target/context; the native connection remains alive. Bridges serialize admission and avoid native download leases. Requests are limited to 128 KiB and use stdin, not argv or temporary files. Timeout/startup admission and 16 KiB output streams are bounded. Errors return a small fixed vocabulary instead of exception/call-log text; DEBUG, DEBUG_FILE, PWDEBUG and Node injection settings are removed.
- Shared `agena-process::output_with_input` drains input/output concurrently and kills descendants as soon as the direct child exits, even when they retain stdin. Reader futures are scoped instead of detached. Large Unicode echo, early failure diagnostics/status, blocked stdin, timeout and cancellation cleanup all pass.
- The synchronous browser launcher now uses the same process-wrap Unix session / Windows Job Object policy, with bounded termination and natural-leader-exit cleanup. Discovery uses `which` without running probe programs. Enabling idle expiry on a reused browser starts its janitor; an action lease prevents idle closure during work; expiry check/removal is atomic under the registry lock. Windows behavior is compiled behind cfg but was not run on this macOS host. A vendored process-wrap import used only on Linux was qualified to keep the newly enabled std feature warning-free on macOS.
- Browser snapshots and activity-panel stop use the page gate. Native downloads retain the gate through their cleanup worker even if the caller cancels. Failed page readiness/final-URL checks close the incomplete target. Closed targets release retained gate/connection entries.
- Playwright 1.58.0 was installed only in the temporary validation environment. Real Chrome fixtures passed covered-click rejection, Unicode/password fill and omission, valid/stale snapshot refs, delayed visibility, iframe input/strict selection, context retention/scope, and continued native navigation interception. A separate real native test printed evidence for screenshots, independent caller cookies, GUID-based download completion, in-flight download limits and dropped-caller cleanup. No user browser session, Agena connector or hosted browser service was used. The bridge preserves existing document-request interception; this is not a new browser sandbox or complete interception of all subresources.
- Validation: 254 tests passed across process/web/bundled unit, integration and doc targets (two optional dependency tests ignored in that default run). The real browser fixtures ran explicitly as described above; after adding iframe inputs, the expanded Playwright fixture and three regenerated-contract tests passed. Clippy passed for all three crates, all targets, with `-D warnings`. The catalog remains 22 plugins / 138 definitions / 134 execution tools / 4 gateways. No final workspace-wide pass is claimed.

At the browser phase checkpoint, the fetch audit identified two concrete next fixes: `Website::crawl()` selects a Chrome path under the compiled Spider feature even when the caller did not request JS, and the current Spider body limit is applied after collection. Ordinary HTTP transport needs explicit bounded streaming/redirect authorization, and crawler attempts/discovery need budgets independent of successful document counts. Optional Trafilatura extraction, remaining file/process/stateful audits, CLI/log disposition, prompt behavior evaluation and final validation remain required. Later sections record the resulting changes.

## Bounded HTTP, extraction alternatives and crawl caches

Ordinary product fetches now use checked reqwest streaming; each initial/redirect/robots request passes host permission and public-DNS validation, pins approved addresses, and disables environment proxies. One deadline includes DNS, pacing, robots, redirect handling and body streaming. This fixes the compiled Spider `crawl()` route selecting Chrome even without a rendering request. Spider's library-only plain route explicitly uses `crawl_raw()`, bounded collection, upstream byte limits and a scoped collector.

- Requests preserve HTTP/query semantics; URL credentials are rejected. Actual final URL is separate from a canonical HTML hint, and relative links use the final URL/HTML base. `encoding_rs` decodes declared character sets and reports replacement errors. The robotstxt 0.3.0 matcher receives the product token, with a regression for specific bot groups.
- Trafilatura is an explicit optional local HTML adapter; readability stays default. No runtime installation or hosted transfer. Sources are registered in `tool-modernization-extraction-sources.json`; three authored fixtures and all six outputs are inspectable in `tool-modernization-extraction-fixtures.json`. Trafilatura's forum omission/duplication precludes claiming universally better extraction; single observed timings are not a performance benchmark.
- Extracted text and standalone previews have separate output budgets and visible truncation. The alternate per-call extractor bypasses the configured cache. Errors/truncated bodies are not reused as successful pages/documents; crawl attempts and discovery are bounded even when every child request fails.
- Stored document cache hits now require current network permission, including the actual final destination; old entries without that destination are refetched. Database handles use canonical paths and weak registry entries with per-store retention, releasing locks when the last store disappears. Moka's weighted 64 MiB eviction accounting and periodically pruned host rate-limit keys bound retained coordination state, without claiming a strict heap limit.
- Initial extended suites passed 159 bundled unit tests and 28 web unit tests, with optional fixtures ignored; the real Trafilatura comparison passed separately. The final phase run passed 259 tests across both crates and their integration/doc targets (three optional tests ignored). Clippy passed for both crates with all targets and `-D warnings`; generated contracts were refreshed without changing the 138-definition count. Rendered transport validation was still outstanding at that checkpoint and is completed in the later sections; HTTP connection pinning must not be generalized to Chromium.

## Rendered fetch and remaining lifecycle audit

Ordinary HTTP/extraction was committed as `6844a297`. The rendered product path now uses a disposable managed CDP context instead of Spider's separate navigation path. The attached page intercepts document and HTTP subresource requests before continuation, performs host permission/public-DNS checks, checks robots for documents, and disables service workers and downloads in that context. A dedicated root connection owns `disposeOnDetach`, including cancellation during target creation. Two renders may run concurrently, with an overall deadline, bounded DOM capture, explicit HTTP status, and observed resource-data limits. Spider remains available as a library adapter; the product no longer mistakes its compiled Chrome feature for a request to render.

A real local Chrome fixture validates JS-produced content, redirect and subresource denial before the denied endpoint is contacted, real 404 status, Unicode byte clipping, timeout and cancellation cleanup. CDP policy workers now explicitly echo the foreground call's host callback context, since Tokio does not inherit task-local authority; interactive native/Playwright commands refresh it. The host remains responsible for rejecting expired authority. Native HEAD preflight also pins approved DNS addresses, disables environment proxies and shares one total deadline. IPv4-mapped private IPv6 addresses and multicast IPv6 are rejected.

These are attached-target request checks, not a complete browser network sandbox: Chromium resolves hostnames independently, and arbitrary workers, WebSockets, WebRTC and browser-internal traffic are outside the stated guarantee. Resource-data events and captured DOM budgets do not impose a strict browser heap/network ceiling. The ordinary HTTP connection-pinning guarantee must not be generalized to Chromium.

- Snapshot Git/Rift commands and session Git inspection now reuse `agena-process` bounded output and process-tree termination. The synchronous bridge runs the async collector on a dedicated thread, avoiding nested Tokio runtimes and detached pipe readers. Nonzero backend probes are unavailable, and oversized machine output is rejected rather than interpreted as a complete inventory. `process_control` is no longer a runtime dependency.
- Monitor readers have an abort-on-drop guard tied to the runner. Existing terminal tests cover owned sessions, launch cancellation, partial writes, bounded output, sandbox wrapping and dropped callers. Existing monitor tests cover reserved identity/replay, success/failure patterns, quiet periods, registry drop and descendant cleanup. The first broad audit run found one intermittent quiet-period failure; its diagnostics and failed reproduction runs were retained. The corrections and final green run are recorded below.
- `fs.read_many` now rechecks observed file size/mtime after reading and uses a boundary-probe byte. Failed decoding/change checks consume their source reservation; they cannot repeatedly spend the same batch budget or return a complete revision after observed growth. Command/skill resource reading uses the shared regular-file opener and checks observed changes. Existing staged-write, patch rollback, notebook schema and revision tests remain applicable; external writers still cannot be made transactional by a comparison plus rename.
- Memory keeps Tantivy. Repeated queries still inspect source documents, but unchanged content digests reuse the committed index. Rebuilds use Tantivy's writer transaction and commit the digest with its segment metadata, preserving the previous searchable index if an update cannot obtain the writer lock. A regression validates unchanged metadata, writer contention, replacement, deletion, Chinese recall and zero-result limits. The portable JSON backend publishes atomically and skips identical bytes; big-endian execution has not been performed.

## Candidate disposition: implementation versus optional task tooling

The initial `tool-alternatives.md` is a historical source/benchmark snapshot. This table records what was actually chosen; listing a CLI is not a claim that it is installed in the Agena service's PATH, that it is universally faster, or that its use overrides permissions.

| Candidate or family | Final integration/disposition | Reason and operational boundary |
| --- | --- | --- |
| ripgrep / `rg` | Embedded libraries retained and enhanced; preferred shell guidance and PATH discovery | Literal/case/context/files/count controls, deterministic bounded Rayon search; measured gains remain workload-dependent |
| `fd` / `fdfind`, `rg --files`, Git tracked-file search | Preferred task-specific CLI routing; embedded glob retained | Git scope excludes untracked files; serial glob retained after equivalent-result optimized profiling |
| ast-grep, Tree-sitter, LSP | Embedded structured rules/rewrite plus root-aware external LSP routing | Preview/revision-required publication and bounded AST output; LSP symbol meaning is distinct from text matching |
| Serena | Existing optional MCP integration, documented in `../structural-code-tools.md`; no competing default LSP owner | Symbol editing/navigation may justify an explicit external service; current root-aware LSP and AST rewriting cover the implemented requirements |
| Semgrep / Comby | Semgrep discoverable as an optional rule tool; no second default rewrite engine | ast-grep covers the implemented structural-rewrite requirement; add specialized rules only for a concrete task, with their runtime/effect costs |
| `jq`, Mike Farah `yq`, `xan`, `qsv`, DuckDB | Curated discovery and usage guidance | Structural JSON/YAML/CSV/SQL operations; verify incompatible yq identity, command effects and project requirements |
| `sd` | Optional discovered CLI; writes retain normal effects | Simple replacement convenience; built-in revision checks and AST rewrites remain the default publication routes |
| `uv`, `just`, Hyperfine | Optional task/development tools with explicit guidance | Respect lockfiles and existing recipes; uv may install/download, just executes commands, benchmarks require equivalent results |
| `xh`, HTTPie, `gh` | Optional HTTP/platform workflows | Better task interfaces, not proof of lower network latency; preserve authentication and operation authorization |
| tokei / scc, dust / duf, procs | Optional analysis/inspection CLI catalog | Machine output, explicit scope and platform support; no unsupported universal performance claim |
| bat, eza, delta, Difftastic | Display/review tier, not automatic model-output replacements | Disable decoration/pagers; Difftastic output is not an applicable patch; plain bounded output remains preferred |
| fzf / zoxide | Explicit interactive tier | Model tasks use known cwd/paths; history and interactive selection are not implicit dependencies |
| Watchexec | Optional CLI inside the existing shell/monitor lifecycle | File-change events differ from periodic monitoring; retain Agena ownership, notifications and stop semantics |
| RTK / targeted log filtering | Experimental catalog tier; reproducible offline filtering experiment only | Exact raw streams and actual exit status retained; no transparent command wrapper in the product |
| Brave / Tavily / Exa / SearXNG | Functional configurable structured search adapters | Explicit provider, bounded results, credentials in environment; deterministic HTTP fixtures; paid relevance/latency unmeasured |
| Playwright | Functional optional interaction backend over owned CDP contexts | Actionability/strict locators/iframe support; preserves native ownership and snapshot/download contracts |
| agent-browser / playwright-cli / Playwright MCP | Optional discovered CLI or existing MCP boundary | Avoid a competing default browser session owner; use explicit isolated sessions when that workflow is selected |
| Chrome DevTools MCP | Existing optional MCP boundary for specialized tracing/network diagnostics | Use an explicitly configured debugging session; no second default browser owner or implicitly launched Node service |
| Spider / readability | Library adapter retained; checked HTTP and owned CDP are product transports; readability remains default | Audit uncovered transport and cancellation/policy reasons to change orchestration rather than replace every extraction engine |
| Trafilatura | Functional explicit local extractor | Three inspectable article/documentation/forum fixtures; no general-quality claim or silent fallback |
| Crawl4AI / Firecrawl | Explicit external MCP/service option; no new default dependency/service | Current adapters cover the verified fetch/render/extraction requirements; additional deployment, licensing and hosted-data behavior need a concrete task benefit |
| MarkItDown / Poppler | Functional local `fs.document` conversion/search adapter | Real PDF/Office MarkItDown fixtures passed; Poppler path exists but was not live-run on this host |
| Docling / ripgrep-all | Optional discovered document tools | OCR/layout and multi-format archive search are distinct tasks; heavyweight models/converters are not installed implicitly |
| nbformat | Official schemas embedded; Python reference validation in development | Preserve notebook revisions and attachments without starting Python for every cell edit |
| Tantivy | Retained with content-aware transactional reuse | Avoid rebuilding unchanged memory indexes; no speculative vector-database migration |
| Git worktree / Rift | Managed snapshot layer retired on 2026-10-05; use Git directly | Preserve existing directories and change records; no Agena-owned workspace restore |
| plan / tasks / cron / settings / interaction / report | Retained Agena state protocols; audited against existing failure/concurrency tests | Delayed plan approvals bind revisions; task admission/cancel failures retain state; scheduling is session-aware; saved settings survive reload failure |
| commands / tools discovery / MCP | Retained contract/resource/connection protocols | Bounded discovery, regular-file reads, exact schemas, reconnect cleanup and structured content are not equivalent to shell aliases |
| chatgpt / claude / gemini hosted tools | Existing 32 wrappers retained and fixture-validated | Vendor-hosted files/containers remain separate from local workspace; returned client actions are not executed locally |

## Prompt behavior review and measured log experiment

At modernization completion, before the follow-up usability audit, the base prompt was 4,658 UTF-8 bytes versus 9,738; all workflow sections totaled 7,493 versus 16,811. The follow-up below restores necessary guidance, and `tool-modernization-prompt-size.json` now preserves all three versions. No tokenizer-count or model-task-success claim follows from these byte counts. The modernization addition names local `fs.document` extraction and revision-bound AST rewrite preview; detailed flags remain in live help and the CLI catalog.

The following scenario review goes beyond formatting/obligation checks by mapping intended decisions to the exercised execution contracts. It is a source-backed scenario review with deterministic runtime tests, not sampled model responses or a paid model benchmark.

| Situation | Expected model decision | Runtime evidence/limit |
| --- | --- | --- |
| Search a literal containing regex punctuation | `fs.grep` fixed-string mode or `rg -F`; narrow paths | Grep equivalence/control fixtures and command parsing |
| Search produces no matches | Treat simple rg exit 1 as an empty result | Exit interpretation tests; pipeline/compound failures remain errors |
| Need only matching filenames or counts | Use files/count mode without fetching every matching line | Structured grep tests preserve complete counts and visible truncation |
| Preferred CLI absent or named `fdfind` | Discover actual runtime PATH, resolve alias or use fallback | PATH/alias/cache/unset-PATH tests; no implicit executable probe |
| User supplies an existing script/toolchain | Preserve command and project semantics | Prompt explicitly disallows modernization-only rewriting/installation |
| Need YAML or CSV fields | Choose identified structural parser; avoid regex/comma splitting | Catalog identifies yq variants and CSV parsers; availability/effects stay explicit |
| Search symbol references or rewrite code structure | Choose LSP or AST; preview then revision-bound apply | Root-routing, rule-only dispatch, stale-revision and overlap tests |
| PDF/Office source | Local document adapter if text is sufficient; inspect layout separately | Four real formats; extraction does not imply OCR/visual correctness |
| `fd -x`, `rg --pre`, yq `-i`, AST `-U` | Do not assume read-only from executable name | Shared command-effects regression cases |
| Changed file after preview/read | Refresh and replan publication | Batch growth, stale-revision, staged-publication and rollback-conflict tests |
| Known versus unknown tool | Read known live help; focused search for unknown owner/capability | Real gateway discovery/dispatch fixtures; no catalog enumeration required |
| JavaScript-heavy page or covered button | Render explicitly / choose optional Playwright | Real Chrome render and actionability fixtures; timeout/denial remains visible |
| Cached URL after policy change | Recheck permission before exposing cached content | Cached-document authorization regression |
| High-volume logs | Bound preview and preserve original diagnostics/exit status | Explicit filtering experiment; no transparent RTK proxy |
| Background task still running | Use notifications and retained handle/cursor; silence is not exit | Terminal/monitor lifecycle and dynamic-prompt contracts |
| Hosted tool returns a local/client action | Respect vendor boundary; do not execute that action | Synthetic provider fixtures for all hosted operation families |

`tools/experiment_log_summary.py` ran an actual `cargo test --locked -p agena-tool --lib` and retained raw stdout/stderr, preview files, hashes, argv and real return code under `tool-modernization-log-fixtures/tool-tests/`. The 78-success-test sample went from 6,606 to 192 stdout bytes; combined streams went from 6,848 to 411 bytes (about 94.0% shorter). A separate intentionally failing subprocess retained code 7 and byte-identical warning/error stderr. The tiny failure stdout became larger because the folding notice costs bytes. These samples are deliberately not an RTK product benchmark, general log-quality guarantee, tokenizer measurement or cost estimate. All unknown lines remain visible, and raw output remains the authority for diagnostics.


## Final lifecycle corrections and reproducible regressions

The broad concurrent audit identified two independent failures; isolated green reruns were insufficient:

1. Darwin can return `EPERM` when a process group contains only zombies. The shared process layer now retains the original group identity and reuses bounded process metadata inspection from the PTY implementation. An error is accepted only after proving no live group members remain. Live-member and unknown-group permission failures remain errors. Both synchronous and asynchronous cleanup share the check; successful kill is idempotent, force-kill reaping is bounded, and vendored group wait loops retry `EINTR`.
2. A `LinesCodec` length error put `FramedRead` in a paused state. Even polling past its transitional `None` could wait for another pipe read while a complete `READY` record was already buffered. The monitor now represents malformed/oversized lines as recoverable records, retaining the mature codec's discard behavior without entering the transport-error state. Actual pipe errors are reported separately. A deterministic open-and-silent duplex fixture verifies recovery of buffered valid lines after both oversized and invalid-UTF-8 lines without waiting for EOF or more bytes.

After both fixes, 20 consecutive full parallel runtime-tools rounds passed: 167 tests per round, with three deliberate benchmark skips. The process suite has nine passing tests, including real all-zombie cleanup and preservation of genuine permission errors. Earlier failing stress logs were retained alongside the corrected evidence; the added ten-second oversized-line fixture deadline is not the fix for the decoder stall.

Crawl metadata updates now remove secondary URL/content-hash entries owned by the replaced document. Deleting a duplicate does not remove another document's mapping, and every secondary-index hit is checked against the loaded document, including interrupted-publication stale entries. The refreshed-hash/duplicate-ownership regression passes.

The final Playwright and rendered Chrome fixtures pass on the corrected process implementation. They validate actionability, Unicode/redaction, snapshot refs, iframe selection, context ownership, interception, rendered HTTP status, output bounds and timeout/cancellation cleanup. These local tests start isolated Chrome and fixture HTTP servers, never an Agena service or Agena connector.


## Modernization validation before the prompt follow-up

Source validation covers commit `53b5c95ad2962f74107650a2defe9f9c4e69c4d1` (later changes in the completion commit are documentation and the already exercised offline experiment). Machine-readable commands, log paths, hashes, results and limitations are retained in `tool-modernization-validation.json`.

| Final check | Result |
| --- | --- |
| `cargo test --locked --workspace --no-fail-fast` | 3,167 passed, 0 failed, 12 ignored across 154 reported unit/integration/doc-test targets |
| `cargo clippy --locked --workspace --all-targets -- -D warnings` | Passed |
| `cargo fmt --all -- --check` and diff whitespace check | Passed |
| Concurrent process/monitor regression reproduction | 20 complete runtime-tools rounds; 167 passed and 3 opt-in benchmark skips in every round |
| Real optional Playwright and rendered Chrome fixtures | Both passed, with printed behavior evidence |
| Real native Chrome snapshot/screenshot/context/download fixture | Passed, with secret omission, stale refs, separate cookies, GUID completion, in-flight limits and cancellation evidence |
| Python refactor tools / failure-semantics invariants | 5 tests and 19 invariant checks passed |
| Universal target manifest consistency | Passed; this is manifest verification, not cross-platform execution |
| Runtime crypto/auth capability gate | Passed, including both real HTTPS client-construction tests |
| Reference / capability identity generator examples | Both outputs are byte-identical to committed artifacts; 22 plugins / 138 definitions |
| Offline log experiment artifact integrity | Every raw/preview byte count and SHA-256 matches; success code 0 and intentional failure code 7 retained |

The twelve default-suite skips comprise two optional browser fixtures, the optional MarkItDown fixture, three search/glob benchmarks, the optional Trafilatura comparison and five documentation examples. Browser fixtures were executed explicitly as recorded above; document, extraction and benchmark evidence is retained in the earlier phase artifacts. No frontend source or build configuration changed, so frontend-specific suites were not rerun. The workspace test linker emitted the existing macOS compact-unwind-size warning for the large `e2e_probe` debug example; Clippy with warnings denied passed. It was not suppressed by disabling unwind support.

All requirements in the current table have an implementation, deliberate retention/optional-integration decision, and recorded evidence. Work remains on the independent branch/worktree; no merge or push was performed. This completion does not claim universal speed gains, production service quality, model task-success improvements, a complete browser sandbox, tested Windows/big-endian execution, live Poppler validation or atomic cooperation from arbitrary external file writers.

## Follow-up: 系统提示词可用性审计（2026-10-04）

用户要求详细检查提示词是否精简过头。本轮对照 `98c1c116` 的原文、`fe79003c` 的压缩版本，以及实际网关 schema、计划状态机、后台通知、项目指引注入和输出截断路径。修复源码提交为 `3fc869a83b948759d843617bc6224b0edd6c6f9a`，仍在本 worktree / branch 内；没有调用 Agena 连接器、启动 Agena 服务或发起真实模型请求。

结论：压缩确实删弱了几条必要指引，也引入了一个计划审批语义错误；此外还发现了此前就存在的 help 矛盾和目录分类问题。已补齐调用、恢复和工作流决策信息，详细参数继续由实时 help 提供。

| 检查项 | 发现及来源 | 修复后的行为 | 证据 |
| --- | --- | --- | --- |
| 普通工具与直接函数的区别 | 压缩后只说“通过网关执行”，未再明确普通工具名只能作为参数值 | 明确 `fs.read` 等不是直接函数；网关不得嵌入 `tools_call` | 基础 prompt；真实 manifest 中的工具名检查 |
| 最小调用结构 | 压缩删除了原先 `{tool, input}` 的明确结构，缺少可直接理解的完整示例 | 加入先读 help、再传 `{"tool":"fs.read","input":{"file_path":"src/main.rs"}}` 的示例；说明不能传 JSON 字符串或额外包裹层 | 测试从实际 prompt 提取 JSON，验证网关 schema 和 `fs.read` schema；四种错误结构被拒绝 |
| 已知工具是否仍要搜索 | 既有矛盾：主提示词允许直接 help，但网关描述和 help 字段要求名字来自 list/search | 已知精确名字可以直接 help 验证当前可用性；未知或不可用才返回 discovery；复用完整且仍在上下文中的 contract | 网关描述、输入说明和生成文档一致 |
| 用户要求列举能力 | “默认不枚举全目录”缺少明确例外，可能误伤工具清单问题 | 用户要 inventory 时允许 `tools_list`，日常任务仍按需搜索 | 基础 prompt 与网关 inventory 说明对齐 |
| 修改计划后的审批 | 压缩版本把“审批期间发生并发变更要重新读/审”写成“已批准计划修改后再审批” | 普通步骤进度更新和已批准计划的完成无需再审批；替换计划重新进入 planning；审批期间的旧版本不能覆盖新版本 | 真实内存 host fixture 覆盖审批→进度→完成→替换；旧审批冲突测试继续通过 |
| 计划 help 与状态机 | 既有文案漏掉可信配置条件，还称只有 `plan.review` 能等待审批；`plan.edit` 也被误描述为通用内容改写入口 | `request_approval: false` 同时要求用户授权与可信配置；`plan.phase` 也可触发所需审批；正文/步骤结构用 `plan.set`，状态/备注用 `plan.edit` | 与实际 `invoke_plan_*`、revision 检查和 schema 对照；生成参考同步 |
| 只有部分计划工具 | 既有逻辑只看 `plan.set` 是否存在，就推荐可能不存在的 review/edit | 可用时优先 `plan.review`，仅有 `plan.phase` 时用其激活审批；两种审批入口都缺失时指导用文字计划，保留已有 planning 限制 | `plan.set` 单独、set+phase、set+review、edit/phase/review 单独等组合测试 |
| 等待后台通知 | 压缩删掉了“没有其他工作时结束当前轮次等待通知”，可能诱发轮询或误报完成 | 明确当前轮次可以结束但工作仍 pending；通知到达后继续，读取结果并验证；允许有具体诊断目的的有限日志读取 | 动态 prompt 回归与既有通知/生命周期测试 |
| 部分后台能力 | 既有实现只要启用任一后台入口，就同时推荐 shell、tasks、monitor、cron | 分能力拼装段落，仅命名实际可用的工作流工具；monitor 可因退出/超时等结束 | 24 组能力组合检查；manifest/运行时分类交叉验证 |
| 交互终端 | 压缩后的“Never poll”和交互读输出之间缺少足够区分；生命周期工具也可能未启用 | 只有 run+write 同时可用才指导 PTY；保留精确按键、空 `chars` 读取、`since_seq`、静默不等于退出、部分写入不可整体重发 | 能力组合及终端义务回归；原有终端运行时测试 |
| 失败、并发与输出不完整 | 原提示词多为“读错误后重试”，不足以区分恢复动作 | 未知工具重新发现；错误参数按嵌入 help 修正；写操作超时先核对结果；revision 冲突重读；空结果与失败分开，截断后按游标或缩小范围继续 | 基础 prompt，与既有网关/文件/输出边界测试共同验证 |
| 任务范围、项目指引与授权 | 压缩省掉了问答/审阅不随意改代码、运行时已注入根指引等说明 | 补回任务范围和完成验证；解释根/嵌套指引来源与截断提醒；已有范围内的授权无需重复索取 | 基础 prompt 与项目指引注入源码对照 |
| 网关目录分类 | 既有静态目录漏算三个 `plugins_*`；部分注释和真实模型 E2E 期望仍硬编码五个入口 | 静态目录和参考文档复用运行时分类；实际 131 个执行工具、7 个 manifest 网关、包含合成 call 后 8 个协议函数 | 枚举真实 manifest 与 `ToolApiFunction::ALL` 比较；工具总数仍为 138 |
| 现代工具选择 | `rg/fd/jq/ast-grep`、本地文档、AST 预览及 revision 检查在压缩后仍存在 | 保留可用性检查、缺失时回退、遵循用户命令/项目工具链、不为升级工具隐式安装；补明 shell 的 reads/writes/network 效果字段 | 核心工具名与真实 manifest 交叉验证；既有选择/效果测试 |

审计过程中补充的第一版示例误用了 `path`，生产 schema 测试因缺少必填 `file_path` 真实失败。该草稿错误在成功的全工作区验证前已修正；这不是原版本中已存在的示例错误。用真实 schema 校验示例比仅断言 prompt 包含某个工具名更能发现这类问题。

| 计量范围 | 精简前 `98c1c116` | 压缩版 `fe79003c` | 本轮修复 | 相对精简前 |
| --- | ---: | ---: | ---: | ---: |
| 基础系统提示词 | 9,738 | 4,658 | 6,288 | 减少 35.4% |
| 基础 + 全部工作流段落 | 16,811 | 7,493 | 10,537 | 减少 37.3% |

单位为 UTF-8 字节。只计身份提示词与工作流段落，不含 provider 函数定义、项目指引、用户追加 system 或对话历史；不能据此推算实际输入 token 或模型成功率。字节上限调整为基础 7,500 / 全工作流 12,000，留出必要解释空间。运行时仍按实际工具集合拼装工作流段落。

| 本轮验证 | 结果 |
| --- | --- |
| 工作区 `cargo test --locked --workspace --no-fail-fast` | 3,172 通过，0 失败，12 项按原配置跳过；154 个报告目标 |
| `cargo clippy --locked --workspace --all-targets -- -D warnings` | 通过 |
| fmt 与 diff 空白检查 | 通过 |
| 新增/修订的重点验证 | 实际 prompt JSON/schema；四种错误调用结构；真实工具名；24 组能力组合；计划审批/进度/完成/替换；全部网关的 manifest/文档/运行时分类一致性 |
| 生成参考与 identity 快照 | 两个 Cargo generator 产物逐字节一致；未使用 `agena inspect` |
| Python 3.13 辅助门禁 | 5 项 refactor 工具测试、19 项不变量检查、目标清单检查均通过 |
| Runtime crypto/auth 门禁 | 通过，包含两项 HTTPS client construction 测试 |

机器可读证据、源码提交、日志摘要和 SHA-256 见 [tool-prompt-audit-validation.json](tool-prompt-audit-validation.json)；三个版本的长度与计量范围见 [tool-modernization-prompt-size.json](tool-modernization-prompt-size.json)。先前完整实施验证保留在原 JSON 中，并明确标注目录计数修正。本轮默认系统 Python 因不支持 `zip(strict=True)` 造成的脚本失败日志也被保留；改用本机已缓存的 CI Python 3.13 后通过，没有修改脚本来掩盖问题。

这些证据能确认提示词示例符合当前接口、条件拼装不会推荐未启用的工作流工具、审批指引符合实际状态机。尚未做真实模型 A/B 任务成功率比较；也没有为了本轮 prompt 修改重跑选装浏览器/文档工具实机测试。真实 provider E2E 中的网关预期已更新并通过编译，其发起模型请求的 main 没有运行。macOS linker 的既有 compact-unwind-size 提醒仍保留，未通过关闭 unwind 隐藏。

## Master integration validation（2026-10-04）

用户授权提交并推送到主分支，要求保留主分支新增修改。本次在独立 worktree `/Volumes/Rc20/Projects/agena-tool-modernization-master`、分支 `integrate/tool-modernization-master` 集成 `master` 的 `68d3b9b52705ec9fa963a3a124207ca13eb9c7ea` 与 feature 的 `cf4984c1b96d3779c7e0e0b395d7d06b84368f1b`。两者自共同基线 `98c1c116` 分别新增 10 和 15 个提交；合并保留双方提交历史，发布使用普通 push。

两个提示词冲突按实际功能合并：保留 capability-aware 工作流、工具调用与恢复指引，同时保留上游 Git 恢复策略、授权复用、取消/超时不等于批准，以及同步和异步路径中的对话范围指引注入。上游 `identity/git_workflow.md` 逐字节保留，并验证无工作流工具时仍插入、各模式均只出现一次。主分支移除的持久化 inverse-patch/undo 机制没有恢复。

上游独有的 124 个变更路径中，121 个与上游逐字节一致。其余三个 TUI session-work 文件仅修复合并后全工作区 Clippy 发现的四项问题：控制结果装箱，三个嵌套条件改为等价 let-chain；保留原有刷新与状态更新语义。另有 17 个双方改动的路径完成合并核对。新加入的 side/BTW 对话、provider retry 状态、Git 有界预览与 rename diff、Web/TUI 内联计划与工作面板均保留。

| 合并后的提示词范围 | UTF-8 字节 |
| --- | ---: |
| 核心基础提示词，不含独立 Git 段落 | 6,414 |
| 上游 Git 段落及分隔符 | 10,084 |
| 基础提示词，包含 Git | 16,498 |
| 基础及全部工作流段落，包含 Git | 21,247 |

原审计的 6,288 / 10,537 字节保留为 feature 历史数据。Git 段落单独设置 12,000 字节上限；原有核心基础 7,500 / 核心及全部工作流 12,000 字节预算继续生效，不为适配旧总长度裁剪上游策略。计量范围仍不含 provider schema、项目指引、用户 system 追加和历史消息。

| 合并源码验证 | 结果 |
| --- | --- |
| `cargo test --locked --workspace --no-fail-fast` | 3,190 通过，0 失败，12 项按原配置跳过；154 个报告目标 |
| `cargo clippy --locked --workspace --all-targets -- -D warnings` | TUI lint 修复后通过，并再次运行上述完整测试 |
| `cargo fmt --all -- --check` | 通过 |
| Web 测试，仓库指定 Bun 1.3.14 | 593 通过，0 失败，192 个文件 |
| Web import/i18n/typecheck 和生产构建 | 全部通过 |
| Python 3.13 辅助门禁 | 5 项 refactor 测试、19 项不变量、目标清单检查通过 |
| Runtime crypto/auth 门禁 | 通过，包含两项 HTTPS client construction 测试 |
| 工具参考与 capability identity 生成物 | 与本轮 generator 输出逐字节一致；目录计数不变 |

独立证据见 [tool-modernization-integration-validation.json](tool-modernization-integration-validation.json)，其中记录验证过的源码 Git tree/blob ID、日志摘要与 SHA-256、父分支版本和保留检查。早期两份验证 JSON 只证明各自记录的 feature 源码，不替代本轮合并验证。没有调用 Agena 连接器、启动 Agena 应用服务或发起真实模型请求；未为本次合并重跑选装浏览器/文档实机测试，也未执行跨平台测试。既有 macOS 大型 debug 二进制 compact-unwind 提醒未被抑制。
