# Tool modernization implementation and completion evidence

Objective: implement the improvements established by the tool-alternatives investigation, repair tool defects found during the audit, guide the model toward appropriate modern tools, and keep the system prompt concise and readable. This is an implementation ledger, not a replacement for that full objective.

Implementation worktree: `/Volumes/Rc20/Projects/agena-tool-modernization`; branch: `feat/tool-modernization`; source baseline: `98c1c116`. Research commit `a0e2c53e` was incorporated as `f5383449`. Do not use Agena connector tools for this work. There are no applicable AGENTS files at initialization. Work is performed without delegated agents.

| Requirement | Required evidence | Current status |
| --- | --- | --- |
| Modern CLI availability and routing | Runtime PATH-aware discovery, accurate identity/availability, bounded optional probes, deterministic tests; modern CLI scenarios from every research table row accounted for | In progress |
| Unified command effects and exit semantics | Shared classification for runtime/workflow; executable/subcommand/flag/quoting cases; no automatic rewrites; meaningful regression tests | In progress |
| Grep/glob completeness, output control and performance | Literal/case/context/output/filter controls; explicit truncation; bounded and cancellable work; equivalent-result benchmarks including in-process implementations | Functional changes implemented and targeted tests pass; throughput optimization still pending |
| AST and language-server improvements | Structural rules/rewrites and language-server availability/use; revision-safe changes; integration tests and documented optional semantic backend | Pending |
| Search providers | Working configurable structured providers and self-hosted option; credentials and network effects; deterministic HTTP fixtures and error/limit tests | Implemented and fixture-validated; live relevance/latency not measured |
| Browser modernization | Functional mature optional backend with session/permission/download ownership; real browser behavior and lifecycle checks | Pending |
| Fetch/crawl/extraction | Existing third-party stack audited; working optional quality backends where useful; bounded output, cancellation and representative fixtures | Pending |
| Local document content | Functional local format conversion/search adapter, capability discovery, clear missing dependencies, fixture-based validation | Pending |
| Read/write/patch/notebook | Existing revision/publication contracts preserved, discovered defects fixed, batch and notebook compatibility validated | Publication checks and notebook schema compatibility implemented; remaining read/write audit in progress |
| Shell/process/monitor/snapshot lifecycle | Audit and fix actual defects; modern tools remain within existing lifecycle/effect contracts; lifecycle regressions | Pending |
| Memory, discovery, stateful and hosted tools | Inventory-wide audit, retained engines justified by evidence, affected correctness/contract tests passing | Pending |
| Concise, formatted and effective prompts | Reduced assembled prompt budget while retaining actual behavioral requirements, modern-tool guidance, clear headings, no redundant tool schemas; behavior/format/budget checks | Implemented; deterministic obligations/format/byte-budget tests pass; no model task-success evaluation yet |
| Optional analysis/display/log tools | Each investigated candidate has a deliberate integration/routing disposition; lossless raw-output recovery and accurate diagnostics; measured output reduction if enabled | Pending |
| Generated contracts and user documentation | Regenerated tool schemas/reference/identity artifacts, configuration and usage docs, consistent public surfaces | Pending |
| Final validation | Appropriate targeted tests, workspace fmt/tests/clippy and repository invariant gates, frontend gates if affected; final evidence audit covers every row | Pending |

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

## Next implementation and audit work

The earlier table's pending areas remain mandatory; none is satisfied merely by the CLI catalog. In particular, finish validation of structured search providers and implement a functional optional mature browser backend, local document adapters, AST rule/rewrite extensions and fetch/extraction options. Account for optional analysis/display tools deliberately, retaining raw logs and exit codes for any compression experiment.

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

The optimized in-process grep equivalence benchmark is currently compiling/running. The only recorded completed measurement remains the slower debug experiment above. The AST audit also found that reaching its match limit currently stops without marking incomplete results; fix that alongside rule/rewrite extensions.
