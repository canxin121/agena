# Agena Architecture V2 Refactor Plan

**Status:** complete for the V2 functional architecture scope — the
source/manifest migration, concrete Runtime composition cutover,
normal-consumer cutover, legacy-monolith deletion, and the Phase 6/8/10 scope
are implemented and functionally verified. Build-graph and timing work is a
separate plan: [`architecture-v2-phase9-performance-plan.md`](architecture-v2-phase9-performance-plan.md).

**Execution mode:** work proceeds in long, ownership-oriented source/manifest/
documentation batches followed by one consolidated *functional* stabilization
phase. Do not run Cargo, architecture, E2E, dependency-analysis, or timing
gates after each small move. During an active batch, use only source
inspection, `rg`, and diff hygiene to keep moving; first delete the old owner
or concrete edge, then continue directly to the next adjacent slice. Run the
locked functional pipeline only once the batch has no remaining
source/manifest/docs work. Build-graph and timing commands are not part of
that pipeline; they belong to the separate performance plan. The
2026-07-23 Plugin Workbench/Usage batch is historical evidence only; the
current batch started with the remaining Runtime repository-composition cutover.
Its consolidated functional verification is recorded below.

**Acceleration protocol:** Phase 6 concrete-edge removals and Phase 8
authoritative vertical slices take precedence over build-speed investigation.
Performance acceptance evidence is owned by the separate performance plan and
is not a blocker for source migration. Do not add
facades, aliases, re-exports, or wrapper crates merely to keep an intermediate
batch compiling; make the actual owner change and repair every consumer in the
same continuous batch.

**Current source-train contract (2026-07-24):** optimize this refactor for
continuous ownership work, not for repeated partial validation. Keep a single
ordered source train active: (1) exhaust real Phase 6 public-concrete-edge
removals, (2) exhaust only complete Phase 8 State/Action/Effect/View slices
that delete their App owner, and (3) perform the associated Phase 10
manifest/export/dead-artifact/documentation cleanup in the same patches. A
slice is not done because a call site moved: its former owner, duplicate
reducer, concrete public edge, or obsolete artifact must be gone. During this
train, do not run formatting, Cargo check/build/Clippy/test, the architecture
executable, E2E, dependency analyzers, or timing probes. Use only source and
manifest inspection, targeted `rg`, `git diff --check`, and untracked-file
diff hygiene to detect mechanical mistakes while continuing to the next
adjacent deletion.

The source train ends only when every non-deferred source, manifest, guard,
and documentation requirement in Phases 6, 8, and 10 has been implemented or
honestly classified as intentionally retained by its proper owner. At that
point—and only after the write queue is explicitly declared closed—run one
consolidated functional stabilization pipeline and repair its complete failure
set together. Do not use a preliminary check or test run to discover the next
source candidate. Normal development and test profiles remain incremental
throughout; performance sampling policy is defined only in the separate
performance plan.

**Immediate replanning directive (2026-07-24):** finish every remaining code,
manifest, architecture-guard, cleanup, and truth-maintenance edit before
spending another turn on execution feedback. Do not split this train into
provisional check/build passes, and do not retune, waive, or measure a speed
budget while it is parked. The eventual functional stabilization gate proves
the completed ownership graph. Performance follow-up is governed by its own
plan and approval decision.

**Write-forward priority decision (2026-07-24):** functional-gate work is
paused while any non-deferred content candidate remains: source, manifest,
architecture guard, dead-artifact deletion, test-fixture repair, or
current-state documentation. A failure already observed in an earlier gate is
input to that same repair batch, not a reason to keep invoking gates between
patches. Complete the entire repair/content queue first, including every
adjacent ownership deletion it exposes; then run format, architecture,
compile, lint, test, E2E, and dependency gates as one ordered stabilization
pass. This explicitly prioritizes finishing the implementation over obtaining
incremental green signals. Build-speed, cold-build, target-graph, rebuild
attribution, and threshold work stays outside this pass in the separate
performance plan.

**Acceleration decision (2026-07-24):** a static source-audit closure is a
queue-management marker, not permission to start validation early. If a
remaining content edit is identified—code, manifest, guard, cleanup, or this
plan—make it in the same write-forward train, update its ownership record, and
continue the source queue. Enter the final functional gate only after there is
no such non-deferred edit left. Any later performance work follows the
separate performance plan rather than reopening this architecture plan.

**Source-train operating protocol (2026-07-24):** treat the current plan as a
write-forward ownership queue, not as a request to repeatedly re-audit its
historical checkpoints. For every candidate, make a short static decision and
record it in the current-state sections only:

1. **Migrate now** only when the patch can delete a real old owner, concrete
   public edge, duplicate reducer/renderer, obsolete export, or dead artifact.
   Trace definitions and consumers with source inspection, change every
   consumer in the same train, and add or update the static architecture guard
   and current-state documentation before selecting the next candidate.
2. **Retain deliberately** when the value still carries a concrete
   Runtime/Application effect (persistence, filesystem, process, schema
   validation, provider/session composition, or a Domain request). State the
   actual owner and reason once; do not revisit it merely to move an enum,
   wrapper, alias, or call site.
3. **Never create a compile bridge.** A temporarily broken intermediate tree
   is cheaper than a compatibility facade. Repair consumers by using the
   proper service/contract boundary or by completing the vertical slice, then
   delete the former path.

The only progress feedback permitted while this protocol is active is static:
read source/manifests/docs, use targeted `rg`, and check patch hygiene with
`git diff --check` (plus `git diff --no-index --check` for a relevant
untracked file). A source candidate does not earn a Cargo, test, architecture,
dependency, or timing command; those commands remain a single end-of-train
stabilization activity. Historical phase text, old green commands, and old
open checkboxes describe context or deferred work, not a competing active
queue.

**Last evidence audit:** 2026-07-23 — Runtime owns concrete database,
configuration, provider, MCP, LSP, PluginHost, scheduler, session, tool,
snapshot, event-bridge, host-client, reload/watch, and application-service
composition. It also owns the public `bootstrap_application_services` factory,
schema-neutral configuration contracts, request normalization, Runtime session
policy, cache maintenance, prompt/compaction bounds, and typed live/query
projections. Provider and Domain retain their stable contract/value ownership;
Application and API retain their service and wire projections. App, CLI,
Studio, E2E, API-server fixtures, and Application fixtures resolve these owners
directly. Locked metadata contains no `agena-core` package, and the old
workspace member, root alias, manifest, source tree, test fixture, compatibility
facades, and all normal consumer edges are deleted. The consolidated format,
architecture, workspace check, strict Clippy, workspace test, E2E, machete,
deny, diff, metadata, and incremental-profile audits passed on the active
2026-07-23 worktree. The timing audit retained normal dev/test incremental
compilation, uses temporary `CARGO_INCREMENTAL=0` targets only for its isolated
cold samples, and kept the retained target below its configured 40 GiB ceiling.
The newest timing command also completed its isolated cold samples and retained
the normal incremental profiles. Its direct samples were 0.70s TUI no-change,
1.13s root no-change, 0.95s TUI leaf, 1.71s CLI leaf, and 7.97s final-app
leaf, all within their declared budgets. This validates the current build graph
and timing probe, not the still-open Phase 6/8/10 ownership exit criteria.

**Scope:** a deliberate, source-breaking refactor of the Rust workspace. The
goal is a maintainable, testable, and fast incremental-build architecture, not
an incremental cleanup of the current `agena-cli`/`agena` layout.

## Current execution ledger

This section records the repository state, not an aspirational checkpoint.
Do not mark a row complete based only on a crate existing: completion requires
the listed evidence and removal of the replaced slice.

### Current status snapshot

This is the fast handoff view. Read it and the execution board before the
historical phase notes below.

| Status | Current fact | Immediate handling |
| --- | --- | --- |
| **Verified** | Runtime database/MCP/LSP/PluginHost/provider-version composition; Runtime-owned schema-neutral configuration errors, JSON-file/numeric/boolean parsing, settings-error classification, optional model-catalog persistence composition, provider model-catalog priority policy, bundled-plugin merge precedence, and bootstrap-request-to-composition-config normalization; Domain-owned schema-neutral dotted JSON-path values; CLI-owned error boundary; Runtime typed live presentation subscription and separate historical-timeline projection; terminal backend/transcript/timeline typed consumers; terminal draft-auth consumer now crosses a Runtime port; all OpenAI/Copilot device and OpenAI/GitLab browser-PKCE draft OAuth transports, Codex OAuth request-user-agent construction, terminal plugin settings path parsing, and the complete provider-owned draft patch wire contract (including adapter policy enums, `ProviderAdapterOverlay`, and outer `ProviderOverlay`). | Keep the executable architecture guards green. |
| **Verified** | The concrete configuration/provider/session/tool/snapshot implementation closure and bootstrap factory are physically owned by `agena-runtime`; the public bootstrap is `agena_runtime::bootstrap_application_services`. App, CLI, Studio, E2E, API-server fixtures, and Application fixtures no longer import or depend on `agena-core`. The old workspace member, alias, crate manifest, source tree, and Core integration fixture are deleted. | Preserve the direct Runtime factory boundary; do not restore an `agena` facade, compatibility alias, or wrapper crate. |
| **Complete for the Core-cutover slice** | Architecture guards, documentation, lockfile, dependency cleanup, and the final graph audit after monolith deletion. | Preserve the completed Runtime/no-Core boundary while the remaining plan phases advance. |
| **Historical full gate; active ownership batch** | The locked format, architecture, workspace check, strict Clippy, workspace tests, E2E, dependency analyzers, timing probe, and static no-Core/incremental-profile scans passed on 2026-07-23 after the Plugin Workbench list and Usage dashboard display slices. The current Runtime repository-composition and remaining Phase 6/8 source batch has not been validated yet. | Keep moving through adjacent source/manifest/docs ownership work; run the complete pipeline only after this batch ends. |
| **Active, no interim gates** | Runtime now composes contract-typed application repositories; App, CLI, Studio, and normal API-server routing no longer compose SQLite repositories or receive a database connection. Default App and CLI memory commands now cross Application use cases instead of rebuilding `MemoryStore`. | Continue deleting adjacent concrete-owner and presentation-owner paths. Do not run Cargo/architecture/E2E/dependency/timing gates until the source batch has no remaining work. |

### Core-cutover verification evidence (2026-07-23)

The 2026-07-23 worktree passed the consolidated local pipeline after the
Plugin Workbench list and Usage dashboard source/docs batch:

- `cargo fmt --all --check`
- `cargo run -p architecture-check --locked --quiet`
- `scripts/cargo-bounded.sh check --workspace --locked`
- `scripts/cargo-bounded.sh clippy --workspace --all-targets --locked -- -D warnings`
- `scripts/cargo-bounded.sh test --workspace --locked`
- `scripts/cargo-bounded.sh test -p agena-e2e --locked`
- the CI feature matrix for marketplace server, PluginHost, Runtime, API server,
  and CLI
- `cargo machete`
- `cargo deny check`
- `git diff --check`

Locked metadata, manifests, the lockfile, and the deleted source path contain no
`agena-core` package, root alias, consumer dependency, or legacy Core source.
Normal development and test profiles retain `incremental = true`; only the
bounded wide-gate runner temporarily exports `CARGO_INCREMENTAL=0`.

| Workstream | State | Evidence already in the worktree | Required next outcome |
| --- | --- | --- | --- |
| Single terminal binary and final app package | **Complete for the entrypoint slice** | `apps/agena` owns the only `agena` bin; the executable architecture checker rejects an `agena-tui` bin and verifies that the entrypoint delegates to the single `agena-cli::AgenaCli` parser. Format and locked architecture gates passed at the historical checkpoint. | Retain the single-entrypoint and no-legacy-app guards; rerun them after the active batch. |
| CLI presentation extraction | **Complete for the presentation slice** | CLI code lives in `crates/agena-cli`; the deleted monolith exposes no CLI module or facade, workspace search finds no `agena::cli` consumers, and the executable architecture checker verifies the single parser/startup path plus CLI boundary consumers. Capability consumers use Runtime/application/plugin-host contracts directly. | Keep the parser/startup and ownership guards green. |
| Application-service extraction | **Complete for the extracted-service slice** | `agena-application` owns the moved transport-neutral services; API server and TUI use application-facing services, and the application boundary is covered by its locked consumer tests. | Split remaining oversized services by capability and remove any remaining server-local business code; these are capability-granularity follow-ups, not a return to transport-owned use cases. |
| TUI server decoupling | **Complete for the server-boundary slice** | `agena-tui` has no `agena-api-server`, SeaORM, or Clap dependency; manifest and source architecture checks enforce this. Terminal environment/profile/transport/identity detection, environment overrides, canonical capability evidence/path/provider-readiness values and lifecycle projection, input normalization, lifecycle and typed response-transaction state machines, validated ordered protocol-frame output, terminal-color values/query transactions, graphics policy/protocol-hint decisions, and in-memory TUI presentation preferences now live in `agena-tui`; the app retains process runtime orchestration, helper evidence collection, capability diagnostics, and one-way mapping from persisted configuration. | Move remaining app-resident TUI feature slices and remove old runtime-type leakage; these are presentation/composition follow-ups, not a return to TUI → API-server coupling. |
| API contract decoupling | **Complete for the wire-boundary slice** | API-owned provider, permission, model-catalog, plugin UI, and message-part DTOs exist; `agena-application` explicitly projects every message-part variant; all application, transport, and TUI transcript consumers use the typed resource; `agena-api` has no legacy-core dependency, and the architecture checker forbids the canonical `agena-core` package. Real REST/WS fixtures and router/client contract tests exercise the shared wire resources. | Extend command/event-kind and resume/replay coverage while keeping the API crate runtime-free; additional protocol coverage is not a return to legacy contract ownership. |
| Remote client decoupling | **Complete for the transport-boundary slice** | `agena-client` has no legacy-core, domain, runtime, application, database, UI, or CLI dependency; its public surface is limited to HTTP/WebSocket transport clients over `agena-api`, and the architecture checker scans both its manifest and Rust sources for forbidden edges. Checked-in HTTP and WebSocket fixtures decode through shared API DTOs; real REST routes cover health/runtime/workspace/error responses; a real Axum listener plus tungstenite client covers WebSocket upgrade, hello, ping/pong, query dispatch, workspace commands, subscription acks, and event notifications. | Keep adding command/event-kind coverage and resume/replay tests as protocol features grow; these are protocol breadth improvements, not a return to legacy transport ownership. |
| Stable domain extraction | **Complete for the V2 stable-value boundary** | `agena-domain` owns role; message/part/execution values, including message text, reasoning, error, file-change, and message-level web-search payloads, operation lifecycle time ranges, the dynamic `ToolInvocation` value and its structured payload, stable structured `ToolOutput` and managed-output values, the fixed provider-facing Tool API function identity, stable message-part delta-field discriminants and message-part delta events, stream-error payloads, execution/subtask lifecycle payloads, command-stream context/begin/delta/end payloads, and permission request/reply/rule-change event payloads; message, part, execution, run, and tool-call IDs; the complete provider-model catalog value (`Model`), identifiers, capabilities, metadata, token limits, pricing tiers, input modality, lifecycle values, thinking/speed modes, provider-neutral thinking requests/reasoning effort/display values, and speed-mode request override/JSON-patch preferences; canonical permission mode/reply/scope values plus read/write access kinds/selectors, parsed network targets, interactive permission request/reply/pending payloads, permission-policy decisions and explainable resolution values, permission-request actions, risk levels, policy-source kinds, and decision-trace values; user-input reply outcome and pending-interactive-request kind; prompt-compaction strategy and trigger preferences; normalized run finish/abort reasons; assistant reasoning-field semantics; event envelope metadata, filtering, scope, and kind-matching contracts; file-change/todo status/priority values; and session lifecycle/lineage/subtask/workflow states. The old Core definitions and facades are deleted; all former consumers use Domain directly. API wire DTOs remain explicitly mapped at the Application boundary, Provider owns provider-specific values, and persistence/lifecycle behavior remains with Runtime/storage rather than being misclassified as pure Domain data. | Preserve the no-I/O/no-UI/no-transport Domain dependency guard and avoid duplicate value mirrors. |
| Ports, concrete adapters, runtime composition | **Complete and verified** | Runtime now owns the actual loader, provider/session/tool builders, snapshot swapping, host-client/event bridge adapters, lifecycle and application-service assembly—not a callback facade over Core. The former Core modules were moved into their owning concrete composition crate and the old definitions were removed in the same source batch. All process consumers call the Runtime-owned free bootstrap factory. | Preserve the combined architecture, Clippy, workspace-test, feature, E2E, machete, and deny gates. |
| Old `crates/agena` monolith deletion | **Complete and verified** | `crates/agena/Cargo.toml` and its source/test files are gone; the root workspace has no `agena-core` alias or member; normal and dev consumers have no `agena = { workspace = true }` edge. Locked metadata and the architecture checker confirm deletion and reject restoration of the package, path alias, or facade. | Keep the no-facade and no-Core dependency scans executable. |

`ExecutionSelection` is now domain-owned: it carries provider/adapter/model-mode,
agent, and canonical permission override declarations used by configuration,
provider resolution, session construction, and persisted session state. Its
unused Core SeaORM JSON derive was deleted rather than pulled into Domain;
persistence adapters own such derives at their boundary. The architecture
checker rejects restoration of `crates/agena/src/execution_prefs.rs`.

Core's former `project_paths` shim is deleted too. Runtime now owns the shared
project-state root, snapshot/Rift paths, and the generated-image artifact path
with its sanitization behavior; session, tool, installation-id, and workflow
callers use `agena-runtime` directly. This removes a second Core top-level
module without changing an application-facing contract.

The installation UUID store now belongs to Runtime as process-managed state.
It preserves the existing create/reuse/invalid-file replacement behavior and
Runtime session replies call the Runtime-owned helper directly; its persistence
test passes in the final workspace run.

The shared Tantivy/fallback tool-discovery algorithm now belongs to
`agena-tool`. Its public document/result contract is consumed directly by the
built-in tool and workflow plugin, while Core's private `search` module is
deleted. This keeps tool discovery provider-independent and removes another
Core top-level implementation module.

`PermissionMode` to `PermissionDecision` materialization is now Domain-owned
as well. Runtime keeps the concrete filesystem/network/tool policy evaluator
and persisted-rule adapter; the former Core pure mapping is deleted, and Domain
tests cover all three decision modes.

### Current critical path and remaining work

#### Fast-batch execution board

| Order | Slice | Current source state | Continue directly with | Deferred evidence |
| --- | --- | --- | --- | --- |
| 1 | Phase 6 concrete-owner closure | **Functionally verified on the current worktree.** Runtime owns repository composition and its root hides implementation trees, including `db`; App/CLI/Studio/API routing no longer compose SQLite repositories or default `MemoryStore` instances. The train closed the public snapshot-registry return and backend-process probe, OAuth browser-listener helper, provider-version HTTP helper, process-metrics helper, detailed tool-executor result edges, API model-catalog Runtime-service escape, API/CLI authentication lifecycle escape, Studio Runtime service-bundle retention, and CLI status projection escape. The remaining Application getters are explicitly audited stable capability ports, not concrete owner leaks. | Preserve the direct capability boundaries. Reopen only for a newly introduced substantive concrete edge; do not create a facade. | Current format/architecture/compile/lint/test/E2E/dependency evidence |
| 2 | Phase 8 authoritative TUI slices | **Functionally verified on the current worktree.** Completed State/Action/Effect/View slices include Choice, file attach, path browser, session navigation/search/model chooser, generic selection, command palette, prompt history, slash commands, file mentions, Timeline, Permission Prompt, and User Input. The remaining App aggregates have explicit concrete-effect retention notes. | Preserve full-slice ownership; do not move a focus enum or renderer shell merely to claim another slice. | Current TUI/application and full-pipeline evidence |
| 3 | Phase 10 cleanup in the same train | **Functionally verified on the current worktree.** Obsolete owners, renderers, exports, stale architecture assumptions, compatibility paths, normal `agena-core` edges, and production `allow(dead_code)` allowances were rescanned after the final repair patches. | Preserve the deleted paths and current guard truth; no compatibility bridge is permitted. | Current source/metadata/dependency audit |
| 4 | Functional stabilization | **Passed on the current worktree.** The consolidated locked format, architecture, compile, lint, workspace-test, E2E, dependency, and diff gates pass after the complete repair batch. | Treat a new functional failure as a new repair batch. | Recorded current functional evidence |

This board is a single acceleration queue, not a set of stop points: finish
all adjacent source, manifest, guard, and documentation moves in rows 1–3
continuously, then enter row 4 once only after the content queue is explicitly
closed. A row may contain many edits and may remain temporarily uncompilable.
Do not insert formatting, build, test, architecture, feature, E2E, metadata,
dependency, or timing commands between owner-deletion patches; the only
permitted progress checks are source inspection, targeted `rg`, diff hygiene,
and this ledger update. Performance measurement is outside this architecture
ledger and is governed by the separate performance plan.

**Current functional evidence (2026-07-24):** after the final typed transcript
fixture and architecture-guard truth-maintenance batch, this exact worktree
passed `cargo fmt --all --check`, `cargo run -p architecture-check --locked
--quiet`, `scripts/cargo-bounded.sh check --workspace --locked`,
`scripts/cargo-bounded.sh clippy --workspace --all-targets --locked -- -D
warnings`, `scripts/cargo-bounded.sh test --workspace --locked`,
`scripts/cargo-bounded.sh test -p agena-e2e --locked`, `cargo machete`,
`cargo deny check`, and `git diff --check`. The API-server test link emits a
non-failing platform linker warning about compact-unwind encoding size; it is
not a functional failure. Performance measurements are outside this plan and
were not part of this verification batch.

**Active queue closure rule:** no source item waits on a preliminary gate. A
Phase 6 audit ends with either a concrete-edge deletion or a short
proper-owner retention note; a Phase 8 audit ends with either a complete
State/Action/Effect/View deletion patch or a short concrete-effect retention
note; Phase 10 cleanup lands in that same patch. This prevents an unbounded
"look again after check" loop while keeping the documentation truthful. A
previous static closure is reopened by a newly identified substantive edit,
not by a check/test result. Only after those decisions have exhausted the
non-deferred content queue may the next task be the single functional
stabilization pipeline—never an exploratory source audit.

**Current Phase 6 root-surface disposition (2026-07-24):** after the snapshot
backend-probe closure, the remaining normal upper-layer Runtime free-function
uses are deliberate process-entrypoint utilities: bootstrap construction,
Tokio runtime creation, tracing preflight/filter setup, and CLI default-config
path selection. They do not return a concrete Runtime tree or bypass a
composed service, so wrapping them would add a facade rather than remove an
owner. E2E uses the same public bootstrap/entrypoint contract by design. The
next Phase 6 audit therefore targets only a newly discovered concrete return,
free helper with an upper-layer implementation dependency, or public detailed
executor/session/provider state—not these retained process utilities.

**Application authentication lifecycle closure (2026-07-24):** API and CLI
authentication now consume Application-owned provider resources and login
commands end-to-end. `Application` owns provider projection/list/readback,
API-key write/remove/refresh, browser/device start/finish/poll, the local
browser-callback wait-and-finish use case, the one Application-to-Runtime
login-kind conversion, Runtime reload after each persisted mutation or
successful login, and the provider readback returned by that completed login.
The former `Application::runtime_authentication` and
`AppState::runtime_authentication` escapes, API/CLI Runtime-auth imports,
Runtime-auth error mapping, local provider projection, and API/CLI-local
reload/readback/callback choreography are deleted. Runtime still owns the
underlying authentication service implementation; Application owns the
complete upper-layer use case. This is an owner deletion, not a forwarding
facade.

**Current normal Core-consumer inventory (source/manifest audit, 2026-07-23):**
empty. App, Studio, CLI, E2E, API-server fixtures, and Application fixtures use
Runtime directly. The workspace contains neither an `agena-core` package nor
an `agena` dependency alias. This is source evidence only until the final
locked metadata and executable architecture gates rerun after the active
source batch; the listed green evidence is historical only.

**Historical second-batch source audit (2026-07-22; superseded):** this table
records the four source seams that the completed continuous cutover removed.
It is retained to explain why the manifest edges could not be deleted before
their concrete bootstrap and presentation consumers moved:

| Consumer | Remaining normal Core seam | Required batch move |
| --- | --- | --- |
| `apps/agena` | `main.rs`/`lib.rs` still load Core configuration and call `AgenaRuntime::bootstrap_application_services`; `backend.rs` now consumes provider-owned draft overlays but still imports Core event/message presentation helpers; `app.rs`, transcript state/view, and tests still consume Core event/message/session projection values. The file-settings path, draft OAuth, live subscription, and historical timeline now use Runtime contracts. | Provide the concrete Runtime bootstrap, then migrate the remaining terminal event/message/session presentation values to typed Runtime/domain/application projections without restoring a Core/JSON bridge. Provider draft patch ownership is complete. |
| `agena-cli` | `cli_runtime.rs` is limited to `AgenaRuntime::bootstrap_application_services`, with its local `CliError` boundary already in place. | Point the existing Runtime bootstrap request at the Runtime-owned implementation and remove the manifest edge in the same normal-consumer batch. |
| `agena-studio-server` | `app.rs` calls only `AgenaRuntime::bootstrap_application_services`. | Use the same Runtime bootstrap implementation and remove the manifest edge. |
| `agena-e2e` | The two DSV4F binaries call only `AgenaRuntime::bootstrap_application_services`; their remaining operations use Runtime ports. | Use the shared Runtime bootstrap and remove the manifest edge; keep fixture ownership explicit rather than adding a Core facade. |

This table is historical execution evidence, not a current implementation
queue. All four moves and their manifest deletions are now complete and covered
by the final unified pipeline.

**Terminal message/event vertical audit (2026-07-23, static only):** do not
move Core `PartContent` wholesale into Domain: it contains Core
`OperationPart`, `AttachmentPart`, and `RequestPart` and currently has roughly
241 source references. The correct existing consumer contracts are
`Runtime::SessionUserMessagePart` for new user text/attachments and API
`MessagePartResource` for typed historical rendering. The live checkpoint/event
projection is complete: production terminal handling consumes
`RuntimePresentationEvent` and its `SessionProjectedMessagePart`, then projects
it with `message_part_resource_from_runtime`; it does not construct Core
`MessagePart`, call Core `project_message_part`, or match Core `DomainEvent`.
The former `DomainEvent`/`PartContent` live path remains `#[cfg(test)]`
fixture-only. The terminal composer/normal-submit/steer input path has also
moved from Core `PartContent` to API `MessagePartContent`; normal submit
forwards that wire value and steer reuses Application's canonical
wire-to-`SessionUserMessagePart` conversion. Production terminal/backend
sources now have no Core `PartContent`, event, message, or session projection
import; remaining references are explicit test fixtures. The remaining
final-app Core manifest blockers at that audit point were bootstrap,
configuration, and other non-live transcript Core projections. Those blockers
were subsequently moved into Runtime and the Core manifest edge was deleted.

**Generic timeline cutover (source audit, 2026-07-22):** `RuntimeEvent` and
Application `EventResource` deliberately remain generic protocol values
(metadata, kind string, JSON payload) for REST/WS consumers. The historical
terminal timeline no longer consumes them: `RuntimeEventQueryService` now has a
separate `list_timeline_events_before` projection whose `RuntimeTimelineEvent`
contains event metadata, stable type key, summary/detail text lines, search
text, and optional linked message id. Runtime formats its concrete event once;
`app_timeline_helpers.rs` only maps/localizes the display row, while
`agena_tui::timeline` owns the searchable picker, terminal sanitization,
preview/dialog rendering, and open-message intent. The terminal does not
deserialize JSON into a Core event, and the generic transport contract remains
unchanged. The full per-variant
presentation enrichment can evolve within this typed projection without
reopening the final-app Core edge.

**Settings/auth split (source audit, 2026-07-22):** do not reopen the file
settings port. Terminal provider settings reads/writes already call
`RuntimeConfigSettingsService` with Runtime-owned input/output values. The
remaining settings seam is local-only: `backend_config.rs` now uses the
Runtime-owned settings-path parser, while provider draft/catalog code still
constructs Core overlay records before submitting a Runtime settings edit. Move
those draft values to a typed Runtime/provider presentation boundary rather
than duplicating the file-service API. The required replacement is a stable
provider-draft patch contract that preserves the existing file-layer JSON wire
shape (auth/defaults/network/adapters and provider-specific defaults) but does
not expose or deserialize Core `ProviderOverlay`, `ProviderAuthOverlay`,
`ProviderAdapterOverlay`, or their Core enum values in the terminal. The
Runtime settings port remains the only file read/write boundary. Do **not** substitute the existing
`RuntimeAuthenticationService` blindly: that service operates on an already
saved provider id and credential store, whereas the terminal flow obtains
tokens before the provider draft is saved and places them into that draft.
The draft-auth Runtime/provider port is now present and the terminal
interactive state machine uses it. All OpenAI/Copilot device and
OpenAI/GitLab browser-PKCE OAuth transports are Runtime-owned; the temporary
Core adapter now only selects the provider branch, validates its small
Core-owned configuration input, and supplies the process version-derived user
agent for OpenAI. Existing Core OAuth free helpers remain only for the saved
credential `AuthManager` lifecycle and must move with that separate lifecycle,
not be retained as a terminal compatibility path. This preserves save timing
and draft semantics while removing the terminal Core import; it does not
reopen the completed settings file-service move.

**Runtime settings-document ownership update (source audit, 2026-07-23):**
the active Runtime settings adapter now delegates its file reads, path lookup,
recursive listing, JSON set/delete/patch mutations, formatting, directory
creation, and write behavior to `agena-runtime`. The only Core callback left
on that path is complete-document schema validation; it is passed as a narrow
Runtime validator callback and does not expose a Core config value through the
port. Runtime tests cover set/patch/read/delete persistence metadata. The
legacy Core editor still serves layered plugin configuration operations until
the configuration schema itself moves; it is not the implementation behind
the normal Runtime settings service anymore.

**Provider patch-value ownership update (2026-07-22):** the first pure-value
slice is complete. `ProviderAuthMode`, `ProviderApiSubtype`,
`ProviderSecretSourceOverlay`, and `ProviderGitlabApiAccessOverlay` are now
defined only by `agena-provider`; Core's overlay schema adapter, raw provider
parser, and override parser consume them directly, terminal draft code imports
them from that owner, and `agena::config` no longer re-exports them. This is
not a Core config facade. The later `ProviderAuthOverlay` migration is recorded
below; the remaining work is the adapter and outer aggregate boundary.

The adjacent pure patch-wrapper slice is now complete as well:
`ProviderProtocolPathsOverlay`, `ProviderDefaultsOverlay`, and
`ProviderNetworkOverlay` are defined only by `agena-provider`. Core's raw
provider parser/merge path consumes those values directly, terminal draft save
construction imports defaults/network directly from Provider, and
`agena::config` no longer re-exports any of the migrated provider patch values.
`ProviderAuthOverlay` owns its complete auth wire shape (credential, protocol
paths, provider gateway fields, and provider-native auth fields). The final
wrapper slice is complete too: `ProviderCapabilityFamilyConfig`,
`StreamTransportMode`, `OpenAiResponsesBackendConfig`, `ProviderAdapterOverlay`,
all nested provider-native-tool overlays, and outer `ProviderOverlay` are now
canonical `agena-provider` values. Core imports them only as a configuration
schema/runtime-adapter consumer; terminal draft construction imports the same
provider-owned contract through its backend boundary. `agena::config` exposes
none of these types. The remaining terminal/Core seam is concrete bootstrap,
not provider patch ownership. The terminal production user-message input path
also now uses API `MessagePartContent` and Application's canonical
wire-to-Runtime part projection; Core `PartContent` persists only in explicit
terminal test fixtures and Core internals.

The minimum draft-auth contract is intentionally small and provider-neutral:
an auth kind (`OpenAI ChatGPT`, `GitHub Copilot`, or `GitLab`), optional GitLab
instance URL/Copilot enterprise domain, and browser redirect URI enter the
port. Browser start returns authorize URL, state, and PKCE verifier; device
start returns verification URL, user code, device code, and polling interval;
browser finish/device poll return either a pending marker or the stable OAuth
draft token value (access token, refresh token, expiry, and optional account
identity). The terminal owns display-url shortening, clipboard behavior, and
the transient draft session; it writes a completed token value back into the
draft exactly as today. The port must not accept a Core provider overlay,
persist credentials, or require a saved provider id. This covers the existing
OpenAI browser/device, Copilot device, and GitLab browser branches in one
batch. The concrete terminal transport move is complete: the Runtime port
implements its provider-neutral OAuth HTTP/shared helper set directly and does
not call a Core helper or retain a Core re-export. Do not delete the legacy Core
free-helper exports merely because this draft flow has moved: saved credential
refresh/revocation remains on the existing saved-provider `AuthManager` adapter
until its own lifecycle move.

**Bootstrap entrypoint audit (2026-07-23, static only):** `apps/agena` still
uses Core `ConfigLoader` only before bootstrap to derive early tracing/log-writer
settings, while its composed UI preferences already come from
`RuntimeConfigurationService`. The terminal package now owns `AgenaAppError` for its entrypoint, app-server,
storage, database, configuration, and internal presentation failures; temporary
Core bootstrap errors are mapped at the process seam. Core `AppError` is no
longer a final-app source dependency. Do not mechanically replace either import: the real Runtime
bootstrap factory must first own an early configuration/tracing projection and
a stable bootstrap error contract. `agena-runtime` now owns the initial
`RuntimeBootstrapError` / `RuntimeBootstrapErrorKind` and
`RuntimeBootstrapPreflight` (workspace/tracing) contracts. Terminal command, TUI tracing initialization, and embedded TUI database
preflight now consume Runtime's schema-neutral
`resolve_runtime_bootstrap_preflight`, which layers global/project JSON tracing,
process tracing environment values, and raw `tracing.*` CLI overrides. The
preflight now reads both JSON layers through Runtime's shared
`read_config_json`/`parse_config_json` error contract, so bootstrap does not
carry a second ad-hoc file parser.
`RuntimeBootstrapRequest::into_composition_config` now also owns the complete
preflight, override-request, and field-wiring step; Core's temporary adapter
only maps the Runtime bootstrap error and invokes its concrete `new` builder.
`RuntimeCompositionConfig::resolve_workspace_root` reuses the preflight root
when the process request omitted one, so bootstrap does not resolve the
workspace twice.
The former Core `AgenaRuntime::bootstrap_preflight` method has been deleted, and
terminal main/lib import neither Core `ConfigLoader` nor `LoadConfigRequest`.
Embedded TUI also delegates connection/schema initialization to Runtime via
`database_url` and obtains the initialized connection from
`RuntimeBootstrapResult`; it no longer performs a duplicate local database
connect before bootstrap. Runtime also now owns long-lived generic process state through
`RuntimeProcessState` (loader/request/workspace/database/control), final generic
snapshot-state assembly through `RuntimeSnapshotCompositionInputs` /
`compose_runtime_snapshot_state`, and the
stable application-port bundle assembly through
`RuntimeApplicationServiceCompositionInputs` /
`compose_runtime_application_services`; Core supplies its resolved
configuration and concrete adapter ports as inputs. Runtime also owns the
resolved configuration value family itself: provider adapter/auth/options,
runtime/UI/session/agent settings, resolved provider/configuration records,
layer provenance, and native-tool harness configuration now live under
`agena-runtime::config_values`. Core's remaining parser, raw overlay merge,
credential persistence, and provider-registry construction consume those
Runtime-owned definitions; the former `crates/agena/src/config/types{,.rs}`
implementation is deleted. This moves bootstrap's durable configuration input
and output values before moving its Core-specific schema loader, without
introducing a Core re-export facade at a normal process boundary; the legacy
configuration module imports the values only crate-locally. The schema-neutral
settings-path grammar, JSON lookup, and path formatting values now belong to
`agena-domain`; Runtime owns the global/workspace settings-layer selector and
maps the Domain error within its concrete settings service. Core's layered file
editor delegates lookup/listing to their owning boundaries and retains only
file-layer selection, write sequencing, and full schema validation, while
consuming Runtime's schema-neutral `ConfigError` and settings-error conversion
helpers.
Runtime also owns parsed `--set` override values, raw-expression retention,
and their syntax/number/boolean validation. Core no longer defines or parses
that command-line configuration schema: its narrow override adapter only
materializes the Runtime value into the still-legacy raw document during
configuration loading. This preserves the remaining schema boundary while
letting a future Runtime bootstrap consume the same parsed override contract
without a Core value façade. The matching `LoadConfigRequest` is Runtime-owned
too, so Core's loader consumes a process/bootstrap request rather than
declaring one.
Runtime now owns the
default persisted model-catalog service factory (SQLite repository, cache
policy, public-source initialization, and its Runtime-generated Codex request
identity); Core's snapshot builder retains only its provider-registry adapter
construction. Runtime also owns plugin-host
compose-and-install policy plus generic host-client installation; Core supplies
only static registrations and its concrete callback/event-publisher adapters;
snapshot composition obtains the host version from Runtime's package-identity
helper rather than reading a Core package environment macro.
Runtime additionally owns generic tracing-filter composition/reload,
maintenance-loop registration, and background-task/task-control shutdown
ordering through its control state; Core retains concrete janitor/reload
futures, session-end broadcasting, lifecycle timing, and diagnostics.
The catalog-definition-to-default-provider-model-patch helper has also moved to
`agena-provider`; terminal catalog drafting no longer imports that pure helper
from Core config.
Runtime's public preflight and bootstrap-result contracts now return
`RuntimeBootstrapError`; Core `AppError` is classified once at its temporary
bootstrap adapter boundary rather than leaking to consumers. This error
cutover is real but does not by itself make the Core composition adapter a
Runtime factory. Only after concrete composition moves can terminal main/lib
remove these Core imports together with
`AgenaRuntime`.

**Atomic bootstrap cutover order:** keep this as one source/manifest train,
without validating its internal transitions: (1) extract the concrete runtime
factory from the temporary `AgenaRuntime` builder while preserving the existing
request/result types; (2) replace the seven normal call sites—terminal app-server,
embedded TUI, CLI's two helper paths, Studio, and the two E2E binaries—with
that factory; (3) remove each consumer's `agena` manifest dependency and Core
imports in the same sweep; (4) use targeted source scans to prove no normal
call site or manifest edge remains; then continue directly to the separate
timeline/settings-auth and TUI/monolith moves. Do not delete the Core builder
or manifest alias before step 2 has no remaining callers, and do not create a
bootstrap facade that merely re-exports the Core builder.

**Bootstrap implementation audit (2026-07-22):** the current
`AgenaRuntime::bootstrap_application_services` is not an eligible Runtime
factory yet. Runtime now parses the raw override expressions and constructs the
Runtime `LoadConfigRequest`; the temporary Core adapter loads the legacy
configuration schema, and builds the concrete
snapshot/session composition, installs the Core plugin-host client, and starts
the Core lifecycle/background tasks. Do not move only its public method name
to `agena-runtime`; that would retain the same Core factory behind a facade.
Extract the independent configuration-resolution, snapshot composition, and
lifecycle inputs/outputs in dependency order, then make Runtime own the
concrete factory before switching the four normal consumers.

**CLI cutover detail (source audit, 2026-07-22):** the initial CLI module-root
Core import paired `AppError` with `AgenaRuntime`, and the former error type was
threaded through the CLI command/render/helper surface. Treat the cutover as
two deliberate sub-slices, not one misleadingly small import edit: (1)
introduce a CLI/runtime-facing error boundary and convert the command surface
without retaining a Core error alias, then (2) replace the remaining
`AgenaRuntime::bootstrap_application_services` call only after the temporary
Core bootstrap adapter has a Runtime-owned implementation. Do not remove the
CLI manifest edge early merely by moving Core imports behind an internal CLI
facade; the manifest may be deleted only after both sub-slices are complete.

The first CLI sub-slice is now implemented in the fast batch:
`agena_cli::CliError` owns CLI configuration/provider/internal, I/O, JSON,
storage-config, and concrete database presentation errors, and the old Core
`AppError` import is gone from the CLI module tree. The temporary
`AgenaRuntime::bootstrap_application_services` seam maps its Core error into
the CLI-owned boundary locally; the final terminal app performs the inverse
presentation mapping at its temporary process seam. This deliberately leaves
the CLI manifest's Core dependency and `AgenaRuntime` import in place until
the actual bootstrap implementation moves, rather than claiming the normal
edge is already removed. The architecture checker records both the CLI-owned
error boundary and the explicit temporary bootstrap mapping.

**Studio and E2E cutover detail (source audit, 2026-07-22):**
`apps/agena-studio-server` has its own HTTP-facing `AppError` and its only
direct Core source use is `AgenaRuntime::bootstrap_application_services`; it
therefore has no separate error/value migration before the shared bootstrap
cutover. The two DSV4F Tool API binaries in `tools/agena-e2e` likewise use
Core only to call that bootstrap method, while their session/query/tool work
already crosses Runtime ports. Keep Studio and those E2E binaries on the same
bootstrap cutover train as CLI. Do not introduce a `bootstrap` facade crate
that merely re-exports Core: that would hide the edge and prevent the required
deletion of `crates/agena`.

**Final terminal-app cutover detail (source audit, 2026-07-22):** unlike
CLI/Studio/E2E, `apps/agena` has four distinct Core seams and must not be
scheduled as a single bootstrap-only edit:

1. **Bootstrap and process configuration:** `main.rs` and `lib.rs` still use
   Core `AppError`, `ConfigLoader`/`LoadConfigRequest`, and
   `AgenaRuntime::bootstrap_application_services` for early tracing and TUI/
   app-server composition.
2. **Live transcript projection:** `app.rs`, `app_types.rs`, and transcript
   state/view code still adapt Core event/message/operation/request values
   (`DomainEvent`, `MessagePart`, `PartContent`, `OperationPart`, and
   interactive request wrappers) into public/API presentation resources.
   The persisted/query path already has `SessionProjectedMessagePart` and
   Application mapping. The typed Runtime live-checkpoint projection at the
   Core event-stream adapter is now present; consume it in the UI next. Do not
   make the terminal app deserialize the generic `RuntimeEvent.payload` JSON
   into Core types, because that would preserve the same dependency under a
   less visible transport boundary. The current
   `Backend::subscribe_session_events` has now been replaced: it subscribes to
   the typed surface, keeps `RuntimePresentationEvent` in `LiveEvent`, maps
   non-incremental session/permission transitions to typed refresh markers,
   and retains ancestor-invalidation and lag force-refresh behavior. The
   historical timeline has also crossed its separate Runtime presentation
   boundary: it receives `RuntimeTimelineEvent` rather than reconstructing a
   Core event from generic JSON.

   **Implementation contract for this cut:** leave `RuntimeEvent` and the
   existing `RuntimeEventStreamService` subscription unchanged for REST, WS,
   SSE, IPC, history timelines, and external event consumers. Add a parallel
   Runtime presentation subscription/projection rather than changing those
   transport values. Its typed variants must cover (a) a checkpoint containing
   projected message metadata and `SessionProjectedMessagePart`, (b) the
   existing domain-owned `MessagePartDeltaEvent`, (c) the durable user-message
   id used to acknowledge an optimistic prompt, and (d) an
   assistant-message-finished refresh marker. Each presentation event retains
   `EventMeta` plus ancestor-invalidation information. Its lag item remains a
   force-refresh signal. A typed non-incremental refresh marker preserves the
   former run/tool/system/permission reload behavior; permission markers carry
   `force_refresh`. Core may perform the one-way conversion from its
   concrete event enum into this projection; Application and terminal UI may
   not receive that enum, `MessagePart`, `PartContent`, or Core JSON payloads.

   The first interface/adapter cut is now implemented:
   `RuntimeEventStreamService::subscribe_presentation_events` is optional, so
   generic transport consumers remain unchanged. Core's `SessionManager`
   maps checkpoint, delta, user-append, and assistant-finished events into the
   typed Runtime surface, skips unrelated events, and retains lag reporting.
   The terminal live backend/transcript consumer and the separate historical
   timeline consumer cutovers are now implemented. Do not conflate either
   typed presentation surface with generic transport consumers.
3. **Provider configuration and authentication adapters:** `backend.rs` and
   provider-draft code still constructs Core config overlays. Runtime now owns
   its settings-path parser and draft OAuth inputs/responses/transports; move
   the remaining overlay values to a Runtime/provider presentation boundary,
   not copied Core overlay structs in the app.
4. **Provider utility boundary:** the provider-client-version refresh currently
   calls a Core provider helper and must move to a Provider/Runtime owned
service before the final manifest edge is removed. This slice is now
complete: Runtime owns the bounded npm fetch, version validation, and typed
failure; `Application::refresh_provider_client_versions` owns the resulting
fetch/persist/conditional-reload use case, and the terminal backend calls that
Application command only. The Core refresh transport/export and its focused
regression tests are deleted or moved with the Runtime implementation.

Cut this app over in that order after the common bootstrap implementation is
available. Tests using Core fixture constructors are not a reason to retain a
normal dependency: migrate them after the production transcript projection is
typed, then keep any unavoidable fixture dependency explicitly dev-only until
the final monolith deletion batch.

Ledger reconciliation: provider mode/configuration/tool values have since been
migrated and are guarded by provider contract tests. The session read/control
slice is also now underway: `SessionExecutionControl` covers cancellation,
execution state, scheduler status, selected model, and cache statistics;
`SessionQueryService` covers tree/export, event sequence, session usage/cost,
workspace usage statistics, and pending interactive-request contexts.
`PendingInteractiveRequest`, the full aggregate `PermissionConfig`, and its
path/network/tool declaration values now belong to `agena-domain`; core
retains parsing, validation, policy compilation, and message-lifecycle
adapters. The declaration/read-side batch is complete. The first concrete
service moves are also complete: session tool execution now has a stable
runtime result/error boundary, permission-rule writes inside the active
transaction use the injected generic storage contract, and runtime owns both
the public-catalog enablement switch and executable default source list.
Remaining model-catalog work is provider/live-catalog and snapshot
composition plus the broader shared-SQLite schema migration; public-source
fetching, parsing, ranking, merging, and the concrete cache adapter now live
outside core. Pure values, collection,
canonicalization, duplicate curation, and reusable fallback merge now live
outside core. The immediate active batch is therefore narrowed to catalog
persistence/snapshot composition, the remaining core-bound tool implementation, and
session run/reply/compaction orchestration, followed by runtime-only
composition.

The terminal app and API server catalog read paths now consume
`agena_provider::ModelCatalogResponse` and the provider-owned catalog-ID
normalization directly rather than importing the core `model_catalog` facade. The CLI
apply-patch renderer likewise consumes the runtime-neutral
`ToolExecutionSummary` and decodes its stable JSON payload for patch metadata;
it no longer asks the core executor for its detailed transcript result.
Application services and terminal-app permission UI/state now consume
`PermissionConfig`, path access shapes, and tool-rule values directly from
`agena-domain`; core remains only at the concrete session-policy application
boundary. The architecture checker rejects reintroduced upper-layer aliases
through `agena::agent`.
The application session-execution projection helper now accepts only
`SessionExecutionControl` and `SessionQueryService`; command/query/API
dispatch passes the core manager merely as the current adapter implementation.
This removes `SessionManager` from that projection API while run/event
materialization remains in the explicitly pending session-orchestration slice.
The projection no longer accepts the core `Session` aggregate either:
`SessionQueryService::execution_context` carries the stable workflow,
selection, permission, workspace, task, and subtask state needed for the
application execution resource. Core performs that projection from its
persisted aggregate; application now receives only a session ID plus runtime
control/query ports.
The matching runtime `SessionExecutionCommandService` now covers continue,
compact, rewind, fork, import, permission/user-input replies, and selection
updates, returning only a stable session-ID outcome. Application command
dispatch no longer calls those `SessionManager` methods directly. User-message
submission now crosses the same port using runtime `SessionUserMessagePart`:
text is domain-owned and attachments are canonical plugin-SDK values. Core
converts that stable input once into persisted `PartContent`, so application
no longer imports the core message aggregate or calls `submit_user_message`
directly.
`Application` now adapts the legacy manager once into an
`ApplicationSessionServices` bundle of control/query/command ports; command
dispatch, run-option resolution, and session-state projection consume that
bundle and no longer name `SessionManager`. Message/event queries that still
materialize core transcript types remain separately tracked concrete adapters.
The same control boundary now owns managed-snapshot inspection: the runtime
`SnapshotRegistry` is exposed only through `SessionExecutionControl`; the
former Core `tool` re-export is crate-private, and its
active/managed projections are already runtime-owned. Application Git and
snapshot services therefore accept the control port rather than an
`AgenaRuntime`, and API Git endpoints no longer pass the core runtime into
those services or traverse `SessionManager -> ToolExecutor`. This is a
read-only composition move; concrete snapshot-tool execution and transcript
message/event projection remain explicit core adapters.
Model-catalog presentation and user-triggered refresh now follow the same
pattern through `ModelCatalogRuntimeService`: it exposes the provider-owned
catalog response, refresh activity, and runtime-owned background-task start
outcome without exposing a runtime snapshot. `AgenaRuntime` remains the core
composition adapter for the actual refresh workflow, but the API catalog
endpoints obtain that capability from `Application` and no longer call
`current_snapshot` or name the core runtime type.
The last core public-source wrapper is deleted as well: snapshot composition
passes only the process user-agent to runtime's
`build_default_public_model_catalog_source`; runtime applies its own source
list and enablement policy before constructing the HTTP/parser adapter.
`model_catalog::merge` is deleted too: its remaining provider-priority
calculation was configuration interpretation over resolved adapters, so it now
lives in `agena-runtime::provider_model_catalog_priorities` and produces the
provider-owned priority value consumed by runtime catalog composition. The core
catalog directory now retains
only decoration that still depends on concrete runtime provider models.
The concrete SQLite catalog-cache adapter has also moved out of core into
`agena-storage-sqlite`. It implements the backend-neutral
`ModelCatalogRepository` contract using the existing SQLite table layout but
does not import core entities or `AppError`; Runtime's catalog service selects
that adapter and Core snapshot composition supplies only its optional database
input. Its transactional replacement regression seeds an old
catalog, replaces it with an empty snapshot, and proves stale entry rows are
not observable after the replacement. SQLite schema lifecycle ownership has
now moved to `agena-storage-sqlite`: backend/FK validation, schema-version
checks, the initialization transaction, and migration-marker commit execute
there. SQLite invariant-trigger declarations, concrete table definitions, and
index definitions now live there as well. All callers invoke the storage
initializer directly; the temporary Core `db::init_schema` facade and
entity-derived schema module have been deleted.
The projected message/part ownership lookup adapter has moved alongside it:
`agena-storage-sqlite::SeaProjectionLookupRepository` performs the stable
message-ID and part-ID to session-ID queries using local SQL, without core
transcript entities or payload types. `SessionManager` composes that adapter
through the existing `ProjectionLookupRepository` port; the old core adapter
is deleted. Full transcript materialization remains a separate, intentionally
pending message-lifecycle boundary.
The first transcript read slice now follows the same division: the
storage-owned `MessageProjectionRepository` reads visible projected-message
headers (including cursor paging) from stable SQL columns and returns opaque
metadata/usage JSON, while Core first synchronizes the event projection and
then validates/decodes those JSON values into its message aggregate. Full
message/part materialization now uses the same repository for batch and
single part reads, with opaque part-content JSON decoded only in Core. Event
projection writes, repair, and aggregate assembly remain Core-owned, but
message/part read queries no longer use Core entities directly.
Projected-message headers now additionally cross `SessionQueryService` as a
runtime-owned read model with stable routing/state fields and transparent
metadata/usage JSON. Core serializes its still-pending message metadata/usage
values exactly once; this opens the list/header pagination cutover without
prematurely duplicating the remaining message-content types.
`ApplicationService::list_messages` now consumes that port for
`parts=none`: it preserves the public assistant-round merge, usage aggregation,
part counts, ordering, and cursor semantics while avoiding concrete
`SessionManager` materialization. `GetMessage(parts=none)` uses the same
header projection plus the stable message-to-session lookup, including lookup
by a merged provider-round ID. `ListMessageParts(mode=none)` uses that lookup
to preserve its not-found behavior without materializing the manager. Summary/
full lists and actual message-part reads still intentionally retain the concrete
transcript adapter until the remaining part-content and ownership-query
contracts move.
The same infrastructure crate now owns `SeaSessionStatsRepository`: workspace
counts, ready-child counts, and visible-message event statistics are computed
from stable SQL columns and the domain-owned message-creation tag set. Core,
API-server composition, terminal-app composition, and application tests now
construct this SQLite adapter directly; the former core adapter is deleted.
`SeaWorkspaceRepository` has moved to the same infrastructure crate. It owns
normalized path identity, CRUD, cursor-ordered listing, and id/path lookup
over stable workspace records; core session composition and the API/app
composition roots, along with CLI workspace-scoped permission writes, now
select it directly; the former core adapter and Core workspace CRUD module are
deleted.
`SeaPermissionRuleRepository` now also lives in `agena-storage-sqlite` for
the stable application-facing list/get/upsert/replace/revoke/delete/resolve
contract. It uses only the permission-rule table's stable columns and
domain-owned mode/scope values; API-server, terminal-app, application tests,
core's non-transactional rule resolution, and CLI list/create/replace/revoke
commands select that adapter directly.
`agena-storage-sqlite` retains the separately named
`SeaPermissionRuleTransactionWriter` for the upsert that must share a
`DatabaseTransaction` with session history writes, preserving the existing
rollback regression rather than widening the storage contract with a SeaORM
transaction type.
`SeaUsageRepository` likewise now lives in `agena-storage-sqlite`. It filters
ready workspace sessions and persisted assistant-usage rows in SQL, then
projects only the storage contract's provider/model identifiers and token/cost
fields from JSON. Core retains cost aggregation and presentation policy, but
no longer owns the concrete database usage query adapter.
`SeaSessionSummaryRepository` has moved to `agena-storage-sqlite` too. Its
ordinary get/list/tree/task-subagent reads and create/rename/delete mutations
now use the stable sessions, lineage, and event columns; it reads only the
single persisted agent-profile scalar from runtime JSON rather than importing
core runtime-state types. API/app composition and core's ordinary session
store paths select that adapter directly. Branch/history construction remains
core-owned because it shares the larger history-copy transaction.
The generic `SeaEventStore<K>` now lives in `agena-storage-sqlite` as well.
It persists and reconstructs the domain/storage event envelope using only
SQLite routing columns plus JSON payloads, so core supplies only its concrete
`EventKind` at composition time. The adapter owns empty/session watermarks,
append transactions, filtering, duplicate-sequence mapping, and millisecond
timestamp restoration; core no longer owns a SeaORM event entity adapter.
The SQLite `StoredRole`, `StoredExecutionStatus`, and `StoredPartKind` active
enums have moved with the other concrete persistence details. Core entities
and history code now import those encoding types directly from
`agena-storage-sqlite`; the numeric assignments remain tested as part of the
persisted activity-projection format rather than being treated as core values.
Plugin-initiated session tools now cross the separate runtime
`SessionToolExecutionService`: core performs session-specific permission
resolution and executor construction, while the terminal app and API server
receive only `ToolExecutionSummary` or a stable approval/denial/execution
error. The core opaque authorization capability no longer reaches those upper
layers.
CLI session-cost rendering now obtains the domain `SessionCostSummary` through
`SessionQueryService`; core alone traverses persisted message history to
aggregate usage and pricing. This is a read-side boundary move only—full
message/session materialization remains pending.
The CLI usage report follows the same boundary: its domain `UsageStatsQuery`
and `UsageStats` now cross `SessionQueryService`, while core retains the
workspace/database aggregation adapter.

**Active-batch pending gates:** all Cargo and formatting gates are deferred.
Do not stop after individual concrete-service slices for owner tests, consumer
checks, rustfmt, Clippy, architecture execution, timing, or E2E. During the
implementation batch, use only read-only source inspection and targeted `rg`
audits to keep ownership/import direction visible. Accumulate the complete
provider/storage/tool/runtime/TUI/monolith change set, then flush the final
verification pipeline once after the last source and manifest edit.

**Fast-batch progress update (2026-07-22):** Runtime bootstrap/database setup,
MCP config parsing and manager construction, LSP config parsing/enablement and
registry construction, plus PluginHost build configuration and previous-host
reuse, and plugin-driven provider-list dispatch have crossed into
`agena-runtime`. The active Core snapshot now remains
only the process adapter for database error mapping, Core-bound static plugin
registration, active-host installation, and the still-pending provider/session/
tool implementations. Runtime's parallel live-presentation subscription and
its terminal consumer, plus the separate historical timeline projection and
terminal consumer, are implemented. Continue as three uninterrupted batches: (1) Runtime/transcript plus remaining
provider-session-tool composition, (2) all normal App/CLI/Studio/E2E Core-edge
cutovers, then (3) remaining TUI work and monolith deletion. All share the one
pending gate, `final unified pipeline`; no per-item command queue is allowed.

**Historical storage slice (implemented; not a new stop point):** keep `TransactionEffects` and the
begin/commit/rollback wrapper runtime-owned. Introduce a capability for writing
permission rules *within the currently active persistence transaction*, then
inject that capability into session persistence choreography. The capability
must not expose `sea_orm::DatabaseTransaction` through `agena-storage`: core's
SeaORM entities/CRUD remain an adapter concern until their own schema-adapter
move. Preserve the existing atomicity regression (a failed session-row update
must not commit its permission-rule upsert) before and after the cutover.

That first transaction-writer cutover is now complete. `agena-storage` owns
the generic `PermissionRuleTransactionWriter<Transaction>` contract without a
SeaORM type; core's `SeaPermissionRuleTransactionWriter` implements it for its local
`DatabaseTransaction`, while the regular repository now lives in
`agena-storage-sqlite`; `SessionStore::persist` receives the writer through
composition rather than calling the Sea adapter statically. The existing
session-row failure regression remains the atomicity proof. The remaining
storage work is moving the wider schema/CRUD adapter surface, not restoring a
concrete repository call in session choreography.

Runtime now also owns the public model-catalog source enablement policy,
including the `AGENA_DISABLE_PUBLIC_MODEL_CATALOG_SOURCES` override. Core keeps
the concrete URL list and parser/fetch implementation for now, but consumes
the runtime decision instead of owning a parallel environment-policy helper.
The executable default URL list has now moved alongside that policy into
runtime as `default_public_model_catalog_sources`. Runtime now also owns the
complete public-source HTTP/parse adapter, concurrent collection,
source-qualified warnings, source-grade annotation, ordering, merge, and the
configured wrapper that implements `ModelCatalogPublicSource`. Core supplies
only the process-specific user-agent string to the runtime constructor. The
old core URL list and parser module are deleted, so there is one executable
source-list and source-adapter owner.
The last core catalog compatibility re-exports are deleted as well: snapshot
composition imports runtime `ModelCatalogService` and provider
`ModelCatalogSnapshot`/`ModelCatalogResponse` directly, and the architecture
checker rejects restoration of those facades. The remaining provider catalog
definitions and concrete live-provider decoration adapter are now `pub(crate)`
imports too; pure catalog baseline decoration (capability, metadata, and mode
merging) now lives in `agena-provider`, and no external caller uses
`agena::model_catalog` as a provider-value facade.
The complete provider-model catalog decoration loop now lives there as well:
`CatalogModelDecorationSource` supplies only runtime-specific model lookup and
capability hooks, while `agena-provider::decorate_provider_models` owns raw-ID
normalization, baseline merge, and appendable-model policy. Core supplies a
small `ModelRuntime` adapter; the terminal application calls the provider-owned
algorithm directly rather than importing the Core catalog facade.

### Fast-track execution batches

The fast-batch execution board above is the sole authoritative current
ordering. The historical Core-cutover trains described elsewhere in this plan
are completed and must not create a second implementation queue. The active
closeout is deliberately shorter and continuous:

1. Delete every remaining real Runtime/Application concrete edge that has a
   stable service or storage-contract replacement, including test-only escapes
   that would otherwise keep an implementation tree public.
2. Extract only complete Phase 8 presentation vertical slices, with the App
   owner deleted in the same patch; leave configuration, draft, filesystem,
   process, and Runtime effects in App when they cannot form that boundary.
3. Remove stale exports, files, dependencies, comments, and guard assumptions
   immediately with each deletion, then update the current-state portions of
   this plan.

There is no verification cadence inside this source train. At every internal
boundary, immediately continue with the next source/manifest/documentation
edit and use only source inspection, targeted `rg`, and diff hygiene. Keep one
ledger entry, `final unified pipeline pending`; do not add a per-slice command
queue or run a gate merely because an edge/export changed. After the last
cleanup edit, run the single functional pipeline, repair its full failure set
as one stabilization batch, and rerun the complete functional pipeline once.
Performance work is outside this plan and is tracked only in the separate
performance follow-up.

The model-catalog boundary now has one canonical raw-ID projection helper in
`agena-provider`, focused configured-definition/appendable-model/merge
coverage, and a standalone public integration test. Runtime consumes that
provider value while retaining its concrete fetch/ranking/persistence
composition; this is not completion of the remaining composition slice.

The event bridge has likewise crossed a runtime boundary: subscription receive,
lag handling, plugin broadcast, and cancellation are generic
`agena-runtime` behavior; core now supplies only its event-bus adapter and
event-envelope projection.

Snapshot-scoped asynchronous registration follows the same boundary: the
generic cancellable batch loop is now `agena-runtime::spawn_registration_batch`;
core's snapshot builder supplies only the LSP-specific config-to-spec mapping
and registry handle.

Database bootstrap composition is now crate-private Runtime behavior through
`connect_runtime_database`: Runtime resolves the storage URL, prepares its
parent directory, reuses or opens the SeaORM connection, and initializes the
SQLite schema in lifecycle order. Core supplies only bootstrap
inputs and maps the Runtime error to its local application error.

The active tracing reload handle is also retained by `RuntimeControlState`; the
runtime performs filter replacement while core retains only configuration
projection and invalid-filter diagnostics.

Snapshot plugin shutdown orchestration is now runtime-owned through
`agena-runtime::plugin_shutdown_guard`; core supplies the concrete plugin host
handle and retains only plugin configuration/event behavior.

Plugin-to-host-to-plugin reentrancy protection is runtime-owned as well:
`agena-runtime::try_enter_invocation` tracks a target by stable session/call
scope and releases it with an RAII guard. The core host-client adapter only
supplies its concrete plugin id and translates a repeated entry into its host
error; it no longer owns a process-global invocation map. The architecture
checker rejects restoration of that core-local guard.

Best-effort resolved-configuration notification is likewise runtime-owned by
`agena-runtime::dispatch_config_if_nonempty`; core supplies only the serialized
configuration value.

Optional asynchronous service gating is now represented by
`agena-runtime::build_optional`; snapshot code supplies only the MCP-enabled
predicate and concrete manager builder.

Model-catalog refresh task choreography now uses
`agena-runtime::run_cancellable_refresh`; core supplies only the catalog
refresh operation and runtime reload closure, while cancellation/shutdown
ordering is runtime-owned.

The cache-age portion of startup policy now uses the runtime-owned
`agena-runtime::is_stale` helper; core retains only the catalog-empty semantic
check and cache-age configuration value.

The combined provider-neutral decision is now
`agena-runtime::should_refresh`; core supplies only whether the catalog has
entries plus its snapshot timestamp and configured max age.

The runtime coordination aggregate is now generic as well:
`agena-runtime::RuntimeControlState<S, E>` owns snapshot swapping, reload
serialization, background-task registry state, and task-control shutdown;
`AgenaRuntimeInner` supplies only the concrete `RuntimeSnapshot` and
`AppError` parameters.

Status correction for the active migration: provider configuration/auth,
storage repository ports, and tool policy/input contracts are no longer
unstarted work. They have independent crates and architecture guards now;
the remaining work is the provider model-resolution tail, storage transaction
adapters, core-bound tool execution, runtime-only composition, and eventual
deletion of the legacy `crates/agena` package.
The stable `CatalogModelDefinition`, `ModelCatalogDocument`, snapshot/response
values, and the non-serialized `CatalogDefinitionSourcePriority` ranking
sidecar now live in `agena-provider`; core's former `types.rs` and
`ranking.rs` definitions are deleted. Concrete remote source kind/tier/grade
values now live in `agena-runtime`, which can compose provider contracts
without depending on the legacy core. Runtime also owns the URL defaults,
HTTP fetching, source-specific parsing, and source-priority policy; core only
provides the process-specific user-agent string at construction. The pure
default source-name/kind-to-grade policy lives with those runtime values.
The configured remote-source aggregate (name, kind, grade, and URL list) is
runtime-owned as well; core's source adapter consumes it for fetch/parse only.
The pure domain-`Model` to catalog-definition projection and capability-patch
conversion likewise now live in `agena-provider`; core consumes the provider
function and retains only configuration-derived priority selection plus concrete
source merge orchestration.
The reusable catalog merge primitives (recursive JSON/request-override filling,
capability selection, stable de-duplication, and pricing fallback merge) are
also provider-owned now, with their focused regression coverage moved there.
The generic definition fallback merge (including mode/default and ranking-sidecar
handling) is likewise provider-owned; core retains only the distinct
public-source priority policy that selects which concrete remote field wins.
Live adapter-model collection (priority ordering, per-provider failure
aggregation, blank-ID filtering, definition projection, duplicate merge, the
cross-provider live-document merge, and priority-aware public-source merge now
also run through
`agena-provider`. The catalog reasoning-mode inference/enrichment pass is
provider-owned as well; core source parsing supplies only source-specific
metadata. The raw-ID-to-catalog-key projection shares the runtime
canonicalization owner, leaving core decoration as a consumer only. Catalog
snapshot publication now uses the runtime-owned lock-free `SnapshotStore`;
the storage-record/provider-snapshot cache codec is runtime-owned too; core
retains only the catalog refresh decision and concrete adapters. Live provider
catalog collection, curation, partial-success warnings, and source-priority
consumption are likewise runtime-owned. Runtime also owns the final public/live
result merge, warning/failure decision, final curation, and thinking-mode
enrichment; core public-source code now emits only a typed fetch result. Pure
catalog curation (canonical IDs, duplicate selection, origin normalization, and
JSON fallback merge) now lives in `agena-runtime`, which consumes provider
catalog values without a legacy-core dependency; core retains only its
public-source composition, cache, and snapshot work.
The stable `generated`/`cache` provenance tags are now encoded and decoded by
`ModelCatalogSnapshotSourceKind` in `agena-provider`, leaving core cache code
to translate only between the storage record and concrete SeaORM adapter.

The next concrete-composition slices are now tracked by source boundary:

1. `crates/agena/src/provider/cataloged_models.rs` and its snapshot
   construction call sites: move fetch/ranking/persistence orchestration behind
   a runtime-owned catalog composition API while keeping source metadata and
   SeaORM records core-bound until their repository transaction boundary is
   explicit. The wrapper itself remains core-bound today because it implements
   the core `ModelRuntime` trait and translates `AppError`/tool-mode behavior;
   canonical model-ID values now belong to `agena-provider`, independent of
   Runtime fetch and ranking adapters. The adjacent
   `model_catalog::decorate` helper remains the concrete convergence point,
   and both provider decoration paths call the single provider-owned
   `catalog_model_id_for_raw` value. The previous temporary split is deleted
   rather than creating a third model-ID utility. A standalone
   model-catalog integration test now exercises the public decoration boundary;
   the full 309-test `agena-core` suite remains the broad regression evidence,
   with
   focused canonicalization, configured-definition decoration, appendable
   model, nested fallback merge, order-preserving deduplication, and pricing
   fallback merge tests now in place before the implementation moves.
2. `crates/agena-runtime/src/model_catalog_service.rs` and
   `crates/agena/src/runtime/builder.rs`: startup-staleness and refresh
   task choreography behind the provider/runtime catalog composition boundary.
   The cancellation/shutdown/reload ordering is now generic
   `agena-runtime::run_cancellable_refresh`; the remaining slice is the
   catalog-specific fetch/ranking/persistence operation and empty-catalog
   startup policy; cache-age staleness itself now uses
   `agena-runtime::is_stale`, and the combined decision uses
   `agena-runtime::should_refresh`.
   Public-source fetch/parse/rank/merge is runtime-owned through
   `model_catalog_http`; the core helper only maps its process user-agent into
   the runtime constructor. The runtime service owns live-provider ordering, fetch-error
   aggregation, final merge/curation, cache-port codec calls, snapshot
   publication, and refresh policy; core no longer retains
   `ModelCatalogService`. The
   `SeaModelCatalogRepository` cache write now uses one SeaORM transaction for entry
   replacement and snapshot-state update, and its corrupt-cache repair path
   now clears entries and metadata in one transaction as well. The
   provider-specific persistence/snapshot composition remains core-bound until
   its repository and provider inputs have an explicit contract. The old
   core-owned public-source merge/fetch implementation is deleted.
   The collector now consumes the narrow provider-owned
   `ProviderModelSource` contract rather than naming `ProviderRegistry`; core
   retains only the registry adapter while Runtime owns the configuration-driven
   priority policy.
   The collector receives priority as a narrow callback, so its provider
   ordering loop no longer depends on `ConfigResolution` either.
   The service entry point is correspondingly named `refresh_from_source`, so
   the public API no longer advertises a concrete-registry dependency.
   `AgenaRuntime` now implements that source contract and the background
   refresh task passes the runtime adapter directly, keeping the concrete
   catalog-source registry inside the runtime snapshot.
3. `crates/agena/src/runtime/snapshot/{mod,builders}.rs`: split concrete
   service construction from the runtime snapshot facade, then move provider,
   plugin, MCP/LSP, session, and tool construction one service family at a time.
   Model-catalog source-registry and service construction now lives in the
   dedicated `build_model_catalog_services` factory; the remaining provider,
   plugin, MCP/LSP, session, and tool factories are still core-bound. Runtime
   provider-registry construction from the catalog snapshot is likewise
   isolated in `build_runtime_provider_registry`. Previous-snapshot plugin
   host reuse, MCP injection, and active-host installation are now isolated in
   `build_plugin_services`; `agena_runtime::compose_plugin_host` owns the
   concrete `PluginHostBuildConfig` policy and `PluginHost::new` construction,
   while Core supplies only its static registrations and installs the active
   host. MCP config parsing, static-bridge gating, and manager construction
   now live in `agena-runtime::mcp_runtime`; snapshot composition only supplies
   the resolved plugin config and process client identity. Core retains only the
   static MCP plugin manifest and model-visible MCP tool implementation. The
   unused Core `default_tool_host` helper and its direct `PluginHostBuildConfig`
   construction are deleted; Runtime snapshot composition is the sole normal
   host-construction path. The
   plugin-driven provider-list dispatch rule, including its empty-host fast
   path and host-facing descriptor boundary, now lives in
   `agena_runtime::dispatch_provider_list_patch`; Runtime also owns the
   remove-before-add patch ordering through `ProviderListPatchTarget` and
   `apply_provider_list_patch`, while Core only supplies the concrete registry
   adapter.
   remaining plugin behavior and session/tool construction stay core-bound.
   Subagent discovery and concrete registry registration remain isolated in
   `build_agent_registry`, while Runtime now owns the normalized configured-agent
   entry projection (`configured_agent_registrations`) consumed by that adapter.
   Scheduler composition now lives in `agena_runtime::compose_scheduler`: the
   Runtime owns the in-memory store, polling interval from
   `RuntimeSchedulingPolicy`, and task startup; Core supplies only the concrete
   session-backed scheduler sink.
   LSP configuration values, plugin parsing, enablement gating, registry
   construction, and cancellable registration retention now live in
   `agena-runtime::lsp_config`; snapshot composition supplies only the
   workspace and process identity. The Runtime MCP and LSP constructors consume
   only projected plugin configuration, not the full `ConfigResolution`; this keeps the snapshot facade's first
   optional-service factories on the typed-input path. The remaining plugin host, provider,
   session, and tool factories still require narrower projections before they
   can move out of core. `build_agent_registry` now follows the same rule,
   receiving only the workspace/config parent and configured-agent map rather
   than the aggregate resolution. Session construction now uses a
   `SessionBuildConfig` projection containing only default selection/agent,
   permission, compaction, and tool-presentation policy; session and tool
   factories no longer need the aggregate resolution for those values.
   Plugin-host composition now has the Runtime-owned
   `compose_plugin_host` adapter: the snapshot factory passes plugin
   configuration, workspace root, previous-host state, optional MCP handle,
   and Core's static registrations explicitly rather than asking a Core
   builder to reopen `ConfigResolution`. Provider-registry
   construction now follows the same path through
   `build_provider_registry_from_inputs`: it receives only the resolved
   provider map, config path, plugin host, and optional catalog snapshot. The
   provider-config projection itself is now typed; the remaining work is to
   move concrete provider-registry construction out of core once its adapter
   construction has a runtime-owned contract.
   Snapshot watch-path collection now also consumes the two config paths and
   plugin configuration directly, rather than reopening `ConfigResolution`.
   Adapter-model discovery follows the same boundary: the public composition
   helper is now `list_provider_adapter_models_with_providers` and accepts only
   the provider map, request target, and environment. Runtime and app callers
   no longer pass the aggregate resolved configuration.
   `RuntimeSnapshot::provider_configs` now exposes that narrow map as the
   provider configuration boundary, so runtime and app adapter-model listing,
   saved-target resolution, and catalog refresh no longer re-read the full
   resolution just to obtain provider settings.
   Workspace and studio health/diagnostic projections now use the same
   provider-map accessor for provider counts and IDs, keeping those
   presentation paths independent of the aggregate resolution for provider
   enumeration.
   Config path/found metadata now has snapshot accessors as well; provider
   settings, workspace settings, selection, and studio health projections use
   those accessors instead of reopening resolution metadata.
   Configured-provider listing and workspace-config mutations now use the same
   provider-map/project-config-path projections.
   `config_json_sources` now obtains paths, found flags, and applied-layer
   descriptions from snapshot accessors; only the effective-config augmentation
   remains intentionally tied to the resolved config value required by that
   settings API.
   Effective-config serialization and its legacy default aliases now use the
   snapshot `config_value`, `default_provider`, and `default_agent` projections;
   the workspace settings path no longer needs the aggregate resolved config.
   API settings REST handlers now use the same snapshot projections for
   effective-config serialization, config paths, and found flags; that
   settings surface no longer calls runtime `config_resolution()` directly.
   Application runtime status, CLI agent/MCP presentation, and API auth/
   marketplace handlers now use snapshot projections for default-agent,
   provider-map, plugin policy, and config-path values; those migrated
   presentation files are covered by the architecture guard as well.
   The workspace no longer exposes `RuntimeSnapshot::config_resolution` or
   `AgenaRuntime::config_resolution`; all production callers use capability
   projections, and an architecture test guards deletion of both aggregate
   runtime facades.
   Snapshot state now stores `Arc<ResolvedConfig>` plus dedicated resolution
   metadata, rather than carrying `ConfigResolution` as the runtime state
   payload; this separates runtime-owned capability inputs from loader
   metadata while preserving the serialized configuration projections.
   Snapshot no longer imports or stores the aggregate `ConfigResolution` type;
   it destructures the loader result at the boundary and serializes an
   equivalent `{ config, meta }` payload only where the plugin API requires it.
   The obsolete `ConfigResolution` plugin/provider host wrapper methods have
   also been deleted; only typed `build_*_from_inputs` adapters remain in the
   registry module.
   The remaining `ResolvedConfig` provider-registry methods have also been
   removed; CLI and runtime callers use the public
   `build_provider_registry_from_configs` input adapter directly.
   Plugin storage and secret-store construction has likewise moved out of
   `ResolvedConfig`; snapshot capability accessors now own those concrete
   adapters, while resolved config remains a value-only structure.
   The last unused `ResolvedConfig::provider_model_tool_bindings` helper has
   also been removed; provider-native binding projections remain in the
   provider/runtime composition paths that actually consume them.
   `ConfigResolution::render` has likewise been removed; CLI config output
   serializes the loader result explicitly at the command boundary, keeping
   the configuration value types free of presentation methods.
   The core runtime module no longer publicly re-exports the concrete
   EventBus-to-plugin bridge; snapshot construction uses the crate-private
   adapter while generic bridge lifecycle remains runtime-owned.
   The runtime module no longer publicly exposes the host-client module or
   no-op host-client constructor; host-client wiring is now strictly internal
   bootstrap composition.
   A workspace-wide architecture guard now verifies that no production Rust
   source under `crates/` or `apps/` calls `config_resolution()`; capability
   accessors are the only supported runtime configuration boundary.
   Provider selection/draft/route/model presentation now also uses
   `snapshot.provider_configs()` throughout; that backend no longer reaches
   through `config_resolution()` for provider-only values.
   A workspace architecture guard now scans app and studio Rust sources and
   rejects direct `config_resolution()` reopening in presentation code; the
   current workspace test run passes all targets.
   Plugin storage and secret-store acquisition now also have snapshot-level
   narrow accessors; host-client persistence callbacks no longer reopen the
   full resolution for those adapters.
   The host-client `read_config` callback now consumes the snapshot's
   serialized `config_value` projection, keeping JSON-path presentation at
   the snapshot boundary while preserving the complete configuration payload
   required by the plugin API.
   Tracing reload application now uses a snapshot `tracing_config` projection,
   and runtime provider-catalog listing/saved-target resolution uses the
   provider-map accessor rather than reopening the aggregate resolution.
   Previous-snapshot plugin reuse now uses the snapshot `plugin_config`
   projection instead of reaching through the complete resolution.
   Complete and resolved configuration JSON documents now use Runtime's
   `config_resolution_json_value` and `resolved_config_json_value` helpers;
   Core snapshot methods only delegate those projections.
   Applied-layer description formatting is Runtime-owned through
   `ConfigResolutionMeta::applied_layer_descriptions`; Core no longer walks
   provenance entries itself.
   Workspace agent registration/default-agent selection and UI configuration
   reads now use narrow snapshot/configuration projections (`default_agent` and
   `ui_config`); the unused `configured_agents` Core accessor is deleted, and
   the concrete agent builder consumes Runtime's normalized registration list
   directly. These paths no longer reopen the aggregate resolution for their
   narrow values.
   Model-catalog refresh and startup priority selection now use the same
   provider-map input rather than `Option<&ConfigResolution>`; refresh
   orchestration remains runtime-owned while core retains only provider
   definition ranking logic.
   Optional database-to-session-manager construction is now isolated in
   `build_session_service`; session resume/reconciliation and tool/session
   behavior remain core-bound after the handle is built. First-build event
   sequence resume and interrupted-execution reconciliation are now isolated
   in `resume_session_state`. EventBus-to-plugin
   event bridge construction is now isolated in `build_event_bridge`, while
   the EventKind projection remains the deliberate core adapter. Final
   `RuntimeServiceBundle` generic assembly is now isolated in
   `build_runtime_services`; the facade retains only snapshot metadata/task
   state composition.
4. `crates/agena/src/runtime/host_client/`: expose runtime-owned service
   handles before moving plugin callback adapters; no host-client move is
   accepted if it reintroduces a dependency from `agena-runtime` to core.
5. `crates/agena/src/event/bridge.rs`: retain the core EventBus and EventKind
   adapter in the event boundary, but keep task spawning and lifecycle guards
   in `agena-runtime`. The former runtime-local bridge has been deleted.
6. `crates/agena/src/runtime/builder.rs`: move configuration/database wiring
   only after the preceding service factories have typed runtime contracts;
   the final builder move must preserve snapshot swap, shutdown, and reload
   gate semantics without recreating a core umbrella facade.

Each slice is complete only when its old core implementation is deleted or
reduced to a concrete adapter, the architecture checker has a matching
dependency/source guard, and the focused crate tests plus core tests pass.
The permission repository port already owns ordinary CRUD and resolution;
`upsert_in_transaction` remains a SeaORM adapter method because it receives a
concrete database transaction for atomic session-history writes. The storage
contract now owns the backend-neutral `TransactionEffects` queue, which runs
registered effects only after a successful commit; `agena-storage-sqlite` owns
the concrete SeaORM begin/commit/rollback runner, while Core supplies the
session-persistence transaction closures.
The architecture checker now also asserts that the storage contract source has
no SeaORM or transaction type leakage. Any future transaction-port extraction
must preserve atomic coupling between permission-rule writes and session-row
updates rather than replacing it with two independent repository calls.

The earlier API message-part migration and its consumer migration are
complete. Do not reopen those slices by adding a compatibility return path.
The architecture checker forbids API/client dependencies on the current
canonical `agena-core` package name (and retains the final `agena` rule for
after monolith deletion). The mandatory remaining order is:

1. **[x] Provider execution contract complete.** The normalized completion
   values, non-streaming `CompletionResponse`, fixed Tool API function
   identity, registry-free `ToolApiDefinition`, and the request contract from
   `crates/agena/src/provider/types.rs` have moved into `agena-provider`.
   `CompletionRequest` is now defined in `agena-provider`, rather than being a
   core-owned composite over core messages/configuration. A typed
   provider-owned conversation-input contract
   now exists and core projects persisted messages into it, preserving roles,
   provider-visible parts, attachment sources, tool calls/results, and replay
   state, including terminal tool-result status and invocation arguments.
   Provider-input equivalents of the tool-disabled history and prompt-envelope
   history/protocol-state projections now preserve that information without
   reading persisted core operations. `CompletionRequest.messages` and every
   production adapter request root now consume that contract; session and
   compaction code retain core messages only until their explicit projection
   boundary. The non-message request configuration surface is now represented
   by the provider-owned request type; core no longer re-exports
   `CompletionRequest`, and all core consumers import it directly from
   `agena-provider`. Replaced core facades/helpers have been deleted. The
   normalized stream event and its provider-native
   tool output values are now provider-owned; session orchestration projects
   them into persisted message presentation blocks without a JSON/string
   bridge. The request-contract slice is complete; retain the contract-shape
   tests and keep the provider-owned boundary free of core facades.
2. **[x] Provider configuration/value ownership complete.** Routing,
   hosted-tool user-location, and remote-connector configuration values have
   moved. Provider-owned hosted-tool configuration, connector, binding
   references, the native-tool aggregate, and the plugin-host configuration
   alias now import directly from their owning crates; generic
   browser/shell/editor harness configuration remains in core. The migrated
   definitions have been deleted rather than retained as re-exports; remaining
   work is concrete provider composition.
3. **[x] Provider model resolution and capability composition closed at the Runtime owner.** The
   typed `ProviderCatalog` and narrower `ProviderModelSource` ports now cover
   provider existence, default-model selection, model execution options,
   provider listing, draft/saved adapter-model discovery, and live model
   listing; `agena-application` consumes those operations without reaching
   into registry internals. The remaining concrete composition is now in
   `crates/agena-runtime/src/{model_catalog_service.rs,model_catalog_source.rs,
   model_catalog_composition.rs,provider/registry/listing.rs}`: fetch public
   sources, rank and merge definitions, persist/refresh the snapshot, decorate
   live models, and resolve a `ModelRef` against the registry. Source-ranking
   metadata and SeaORM persistence deliberately remain in their concrete
   Runtime/storage adapters; the historical monolith surface is deleted.
4. **[x] Storage port family closed at the contract and concrete-adapter boundary.** Repository/event-store contracts and
   SeaORM adapters now sit behind `agena-storage`. The model-catalog cache
   replacement and corrupt-cache repair paths in
   `crates/agena-storage-sqlite/src/model_catalog_repository.rs` are now transactionally complete
   (entry rows and snapshot metadata change together, with the focused
   `writing_cache_replaces_entries_and_snapshot_state_in_one_operation` test
   covering replacement). The concrete SeaORM transaction runner is now in
   `agena-storage-sqlite` (not `agena-storage`, which intentionally remains
   backend-neutral), and Runtime composition injects that adapter. Session and
   history transaction choreography deliberately remains at this concrete
   adapter boundary rather than becoming a second port family.
   `agena-storage` continues to own only the `TransactionEffects` post-commit
   queue. Permission-rule writes must remain atomically coupled to
   session-row/history effects while that adapter moves. The focused
   `permission_rule_upsert_rolls_back_when_session_update_fails` regression
   now proves that a rejected session update does not leave its transaction-
   scoped permission-rule upsert committed independently.
5. **[x] Tool contracts and implementation boundary closed at the Runtime owner.** The provider-independent
   contract slice is complete for invocation policy, permission checks,
   shell/process request and result values, read/task input policy,
   truncation policy, patch-result values, snapshot backend policy, builtin
   profile policy, availability, cron summaries, and process events; these are
   owned by `agena-tool` or `agena-domain` and are covered by tool-contract
   behavior tests. `agena-tool::code_search` now also owns the complete
   ast-grep/Tree-sitter structural-search and syntax-tree algorithm: language
   aliases/inference, path traversal, parser/search execution, stable result
   values, and text formatting. Runtime's built-in code plugin now supplies
   only plugin-SDK input annotations, workspace/permission context, output
   serialization, and error classification; it has no direct ast-grep or
   Tree-sitter dependency. The remaining concrete slice is executor composition and
   payloads in `crates/agena-runtime/src/tool/{payload,result,tool_registry,
   orchestrator,executor}`: `ToolApiBinding`, the typed
   `ToolPayloadInput`/`ToolPayloadOutput` variants, attachment/message-part
   projections, and `ToolExecutor` still depend on Runtime session/message
   types. They intentionally remain Runtime-private: a runtime-neutral public
   executor result would recreate the concrete edge this refactor removed.
   Provider continues to depend on declarations, never on the executor
   implementation.
6. **[x] Session orchestration boundary closed at the Runtime owner.** Read-side session boundaries have
   already moved behind `agena-storage`'s `SessionSummaryRepository`,
   `SessionTreeRecord`, and `ProjectionLookupRepository`; application list/tree
   services and `SessionStore` consume those ports instead of issuing ordinary
   session CRUD/statistics queries directly. The remaining write/execution
   slice is still concrete in
   `crates/agena-runtime/src/session/{manager,manager/replies,manager/compact,store}`:
   run lifecycle, history/compaction, usage, permission resolution, and
   `ToolExecutor` orchestration are constructed by `SessionManager::new` and
   close over concrete database and runtime state. They remain private Runtime
   composition so atomic session-plus-permission effects are preserved. The
   generic in-process execution
registry (exclusive ownership, cancellation, steer channel, lifecycle
   transitions, and abort escalation) now lives in `agena-runtime`; remaining
   adapters supply only their concrete message-payload projection.
The first concrete session-service capability is now exposed as the narrow
runtime `SessionExecutionControl` port: active execution state is the
domain-owned `ExecutionLifecycle`, while cancellation is ID-based and maps
adapter failures to a runtime-owned error value. `SessionManager` implements
that port, and application cancel/status paths plus scheduler-backed
automation inspection consume it instead of calling those control methods or
traversing `SessionManager -> ToolExecutor` through the core type. This is intentionally not a
claim that the whole manager has moved: submission, replies, compaction,
   message/event projection, and construction still require their concrete
   Runtime contracts. The same port now returns the persisted effective
`ModelRef` for run-option resolution, so application no longer loads a core
`Session` merely to inspect model selection; malformed persisted selections
remain an adapter error at the runtime boundary. Runtime status likewise reads
the domain-owned `SessionCacheStats` through the same port instead of reaching
into the core cache implementation.
Session tree and JSONL export command reads now use the separate runtime
`SessionQueryService`: the tree returns domain `SessionSummary` values and the
export returns text, so command dispatch no longer calls those core manager
methods directly. Full session materialization, message/event queries, and
import remain outside this narrow query contract until their own stable
projections exist. The same query port now supplies latest event sequence and
domain `SessionUsage` for execution-resource projection; core retains event
envelope materialization and full session loading behind its adapter.
`PendingInteractiveRequest` is now domain-owned as well: it is a tagged
permission/user-input request wrapper whose payloads were already stable
domain values. Core keeps `RequestPart` and `InteractiveRequestPart` because
they are concrete message lifecycle projections, but session/application
interactive-request consumers now import the pending value from
`agena-domain` directly; the old core message re-export is deleted.
`SessionQueryService` now returns `PendingInteractiveRequestContext` records
for a session and its active descendants. Core retains tree traversal,
execution-liveness checks, and full session materialization in its adapter;
application only maps the stable context/payload into API wire resources.

The execution-snapshot permission-configuration prerequisite is now complete:
`PermissionConfig`, `PathPermissionConfig`, `PathAccessModes`,
`PathAccessRuleConfig`, `NetworkPermissionConfig`, `ToolPermissionConfig`, and
`ToolPermissionRules` are domain-owned declarative values composed only from
stable permission modes, strings, and ordered maps. Core retains path
shorthand parsing, pattern validation, policy compilation, and plugin-host tag
conversion as adapter behavior; the old core value definitions are deleted.
Do not add untyped JSON to the runtime session snapshot as a shortcut. The
effective-permission and permission-ceiling fields may now join the stable
execution snapshot once their concrete session projection moves. The required
domain/core/application/architecture coverage remains in source, but its
execution is deferred with all other gates to final stabilization.
The first value in that batch, `PathAccessModes`, now lives in
`agena-domain` with its pure read/write overlay behavior. Core imports it for
policy compilation and host/config adapters but no longer defines a duplicate
path-mode struct; the larger path-rule/configuration and policy behavior move
remains intentionally pending.
`PathAccessRuleConfig` has now crossed the same value boundary. Its
`Modes`/`Shorthand` enum shape is domain-owned, while core retains the
`path_access_rule_to_modes` adapter and shorthand parser because interpretation
is policy behavior rather than a value contract.
`NetworkPermissionConfig` is now domain-owned too, including its declarative
defaults and ordered rules plus pure overlay behavior. Core retains
`apply_network_permission_config`, which validates concrete network patterns
and compiles them into the runtime policy.
`ToolPermissionRules` now likewise lives in `agena-domain` as the stable
mode-or-ordered-rule shape; core retains `apply_tool_permission_rules` for
concrete tool-policy compilation and plugin-host adapters.
`ToolPermissionConfig` now completes the declarative tool aggregate in
`agena-domain`, including default/tag/name/plugin/rule overlays. Core retains
`apply_tool_permission_config` to interpret plugin-host tags and compile the
concrete tool policy; no core value duplicate remains.
`PermissionConfig` now completes the aggregate configuration boundary in
`agena-domain`, including global defaults and pure path/network/tool overlays.
Core exposes only explicit policy-compilation functions over that domain value;
it no longer owns a configuration struct or a compatibility facade.
`PathPermissionConfig` now completes the declarative path aggregate in
`agena-domain`; core retains `apply_path_permission_config` for pattern
validation and concrete policy compilation.
The pure prompt token budget calculation is runtime-owned too; core prompt assembly retains only
   message/tool/provider-payload projection. Stable session cache limits are
   likewise converted into the runtime-owned cache policy before the concrete
   core Session cache applies them. ContextGovernor's hard/proactive threshold
   policy is runtime-owned as well; the processor supplies the core-specific
   message payload character estimate. The pure context-window, reserve,
   auto-compaction-limit, and usage-percent calculations are runtime-owned
   too; session run/compact adapters supply their concrete model limits.
7. **[x] Presentation and composition source scope closed.** The completed TUI
   vertical slices and CLI/Application consumer cutover delete their prior
   owners. The residual App aggregates have documented concrete-effect
   retention, Runtime remains the composition layer, and the deleted monolith
   boundary is protected by current CI, documentation, and architecture guards.

### Verified progress and non-completion rules

- The repository currently builds with the provider contract crate isolated
  from the legacy core, HTTP, database, CLI, and TUI packages. API and client
  currently have no legacy-core dependency, and the checker enforces that
  absence against the canonical `agena-core` package name.
- At the **historical metadata verification checkpoint**, `cargo metadata
  --no-deps --format-version 1 --locked` reported direct `agena-core` edges
  (through the workspace alias `agena`) for `agena`, `agena-studio-server`,
  `agena-api-server`, `agena-application`, `agena-cli`, and `agena-e2e`.
  That is historical verification evidence, not the current consumer count.
  The current authoritative source/manifest inventory at the top of this
  document identifies four normal consumers: `agena`, `agena-studio-server`,
  `agena-cli`, and `agena-e2e`; `agena-application` and `agena-api-server`
  retain Core only in explicitly dev-only fixtures. The remaining normal edges
  are intentional migration dependencies while their capability slices are
  removed; their existence proves the monolith-deletion milestone is not
  complete, not an approved final facade.
  The reproducible consumer audit is:

  ```bash
  cargo metadata --no-deps --format-version 1 --locked \
    | jq -r '.packages[]
        | select(any(.dependencies[]?; .name == "agena-core"))
        | .name' \
    | sort -u
  ```
  The deletion gate is set-based rather than count-based: completion requires
  this command to return no package names. Removing one consumer while another
  package (including `agena-e2e`) still resolves `agena-core` is only an
  intermediate migration step and must remain recorded as incomplete.
- A type is counted as implemented when its authoritative definition and all
  callers have moved and its old core definition is deleted. It is counted as
  fully verified only after the final unified pipeline passes. A re-export
  from `crates/agena` does not satisfy either state.

Historical verification matrix (last executed checkpoint; do not rerun during
fast implementation mode):

| Scope | Command | Result |
| --- | --- | --- |
| Core catalog/composition | `cargo test -p agena-core --locked --quiet` | 304 unit + 1 integration passed |
| Catalog canonicalization | `cargo test -p agena-core --lib catalog_model_id_projection --locked --quiet` | 1 passed |
| Catalog appendable projection | `cargo test -p agena-core --lib catalog_wrapper_appends --locked --quiet` | 1 passed |
| Catalog merge behavior | `cargo test -p agena-core --lib pricing_merge_fills --locked --quiet` | 1 passed |
| Catalog integration boundary | `cargo test -p agena-core --test model_catalog_integration --locked --quiet` | 1 passed |
| Provider contract/source boundary | `cargo test -p agena-provider --locked --quiet` | 30 passed |
| Runtime primitives/lifecycle | `cargo test -p agena-runtime --locked --quiet` | 42 passed |
| Runtime lint gate | `cargo clippy -p agena-runtime --locked --quiet -- -D warnings` | passed |
| Application consumers | `cargo test -p agena-application --lib --locked --quiet` | 11 passed |
| API-server consumers | `cargo test -p agena-api-server --lib --locked --quiet` | 2 passed |
| Architecture guards | `cargo test -p architecture-check --locked --quiet` | 39 passed |
| Workspace regression | `cargo test --workspace --locked --quiet` | all workspace test targets passed |
| Workspace lint gate | `cargo clippy --workspace --locked --quiet -- -D warnings` | passed |
| Patch hygiene | `git diff --check` | passed |

### Fast execution and batched verification policy

The evidence above is a historical verified checkpoint, not a command list to
repeat. The remaining implementation uses two modes only:

1. **Fast implementation mode.** Make all queued Rust, manifest, architecture-
   guard, TUI, and documentation edits continuously. Do not run `cargo check`,
   `cargo build`, `cargo test`, Clippy, rustfmt, architecture executables, E2E,
   feature matrices, dependency analyzers, or timing probes. Read files and use
   targeted `rg` searches to find definitions/callers and to prevent obvious
   facade reintroduction. Temporary compile failures and formatting drift are
   acceptable while the batch is moving toward the declared final graph.
2. **Final stabilization mode.** Enter this mode only after Runtime cutover,
   TUI migration, monolith deletion, workspace dependency cleanup, and removal
   of migration-only shims are all implemented. Run formatting and the complete
   verification pipeline once. Collect all failures, repair them as one batch,
   and during repair rerun only the failed command or narrow failing target.
   After every reported failure is fixed, rerun the complete pipeline once to
   prove the final revision.

During fast implementation mode, every change kind has the same command policy:

| Change kind | Allowed during implementation | Deferred evidence |
| --- | --- | --- |
| Markdown, Rust, file moves, imports, manifests, public exports, architecture guards, API, TUI, Provider, Runtime, session, storage, schema, features, build graph | File inspection, targeted `rg`, and plan/status updates only | Formatting, compilation, tests, Clippy, architecture execution, metadata/dependency audits, E2E, feature matrix, and timing |

Persistent-format changes still require a migration and fixture coverage in
the source batch, but those fixtures are executed with the final pipeline—not
immediately after they are written. Do not run an intermediate command merely
for reassurance, and do not create per-slice verification checkpoints.

The functional final gate is deliberately consolidated. Run it only after the
last source/manifest/documentation cleanup edit; do not use a formatter that
rewrites files outside the required patch workflow:

```bash
cargo fmt --all --check
cargo run -p architecture-check --locked --quiet
scripts/cargo-bounded.sh check --workspace --locked
scripts/cargo-bounded.sh clippy --workspace --all-targets --locked -- -D warnings
scripts/cargo-bounded.sh test --workspace --locked
scripts/cargo-bounded.sh test -p agena-e2e --locked
cargo machete
cargo deny check
git diff --check
```

Feature-matrix and platform-specific release checks remain CI obligations.
Performance measurements are deliberately excluded from this functional gate
and belong to the separate performance follow-up. Do not add a performance
command to this gate or reinterpret its result as a source-train failure. CI
retains its independent platform jobs.

Deferred verification remains visible as a single ledger item: `final unified
pipeline pending`. A migration slice may be marked `implemented, pending final
verification`; it is not marked fully complete until the final pipeline proves
the combined revision.

### Definition of done for every migration slice

Before declaring a slice implemented, all of the following source conditions
must be true:

- The new owner and intended dependency direction are present in manifests and
  architecture guards; executable metadata evidence is deferred.
- All callers use the new typed API; no facade, deprecated re-export, alias,
  feature-gated dual path, or string/JSON conversion bridge remains.
- The old implementation is deleted in that same slice.
- Required tests, fixtures, and architecture assertions exist in source, but
  remain unexecuted until final stabilization.
- Persistent-format changes include an explicit migration and preservation
  fixtures for existing user sessions, databases, configuration, and credentials.
- Full completion requires the single final unified pipeline to pass after all
  slices, monolith deletion, TUI work, and cleanup are combined.

Do not use `scripts/check-runtime-slice.sh` during fast implementation mode; it
is retained only as a diagnostic helper if the final pipeline reports a failure
inside the runtime/session/tool boundary.

## Non-negotiable decisions

1. The terminal product has exactly one production binary: `agena`.
   `agena-tui` is deleted; it is not retained as a second Rust compatibility
   binary.
2. The final product package is `apps/agena`, with Cargo package name `agena`
   and binary name `agena`.
3. The current `crates/agena` monolith is dismantled and deleted. It must not
   survive as an umbrella facade which re-exports the new crates.
4. There is one process-level startup path and one CLI parser. Only the final
   app binary parses process arguments, initializes tracing, constructs the
   Tokio runtime, handles signals, and selects a launch mode.
5. The TUI calls application services directly in-process. It must not depend
   on `agena-api-server`, HTTP, WebSocket, JSON-RPC, Axum, SQLite, or concrete
   provider adapters.
6. `agena-api` owns pure wire contracts. It does not re-export domain structs
   and does not depend on the runtime or application implementation.
7. `agena-client` depends on the API contract and transport libraries only; it
   does not depend on domain, runtime, storage, or provider implementations.
8. Compatibility shims, deprecated re-exports, dual parsers, and temporary
   facade modules are not accepted as a final state. When a slice is migrated,
   callers move and the old slice is removed in the same migration phase.
9. This is destructive for internal source APIs, package names, module paths,
   and obsolete commands. It is **not** permission to silently delete user
   sessions, databases, configuration, or credentials. Persistent-format
   changes require explicit, tested migrations.

## Baseline diagnosis (historical)

The following described the baseline before this refactor began. It is kept to
explain the target, but it must not be used as a statement of current state;
the execution ledger above is authoritative.

- `apps/agena-cli/src` is about 85k Rust LOC, and its library target is mostly
  the terminal application despite being named `agena_cli`.
- `crates/agena/src` is about 119k Rust LOC and combines domain objects,
  configuration, provider implementations, sessions, tools, plugins, storage,
  runtime, and the complete Clap CLI.
- The current `agena-cli` package produces two heavily overlapping binaries:
  `agena` and `agena-tui`.
- The `agena` and `agena-tui` entry paths maintain separate TUI-oriented Clap
  models and convert them into the same TUI launch implementation.
- `agena-api-server/src/local_api` contains roughly 4.7k LOC of transport-
  neutral business services, while the TUI depends on that server crate.
- `agena-api` and `agena-client` currently depend on the giant runtime crate,
  so a wire contract/client cannot evolve or compile independently.

The goal is not to create a large number of crates mechanically. A new crate
is justified only when it owns a coherent capability, has an independent
dependency set, is reused by several consumers, forms a stable test boundary,
or prevents a costly rebuild from spreading.

## Target workspace

```text
apps/
├── agena/                         # package + sole terminal binary: agena
└── agena-studio-server/           # binary: agena-studio

crates/
├── agena-domain/                  # stable values, invariants, events
├── agena-config/                  # configuration syntax, merge, validation
├── agena-provider/                # provider contracts and models
├── agena-tool/                    # tool contracts and invocation model
├── agena-storage/                 # repository/event-store contracts
├── agena-session/                 # session execution/state orchestration
├── agena-application/             # product use cases and services
├── agena-runtime/                 # concrete composition and lifecycle
│
├── agena-provider-openai/         # concrete adapter families
├── agena-provider-anthropic/
├── agena-provider-google/
├── agena-provider-aws/
├── agena-provider-local/
├── agena-storage-sqlite/          # SQLite/SeaORM implementation
├── agena-tools/                   # concrete built-in tools
│
├── agena-cli/                     # argument schema, dispatch, text/JSON output
├── agena-tui/                     # terminal UI application library
├── agena-tui-components/          # reusable, dependency-light widgets
├── agena-api/                     # versioned wire DTOs only
├── agena-api-server/              # HTTP/WS/SSE/JSON-RPC transport adapter
└── agena-client/                  # remote API client

tools/
└── agena-e2e/                     # real-provider/MCP/plugin end-to-end tools
```

Existing specialised crates (`agena-plugin-sdk`, `agena-plugin-host`,
`agena-mcp-client`, `agena-mcp-server`, `agena-lsp`, `agena-skills`, and
`agena-scheduler`) remain only if they respect the dependency rules below.

## Dependency contract

The dependency graph is strictly one-way.

```text
                           apps/agena
                                |
              +-----------------+-----------------+
              |                 |                 |
         agena-cli         agena-tui       agena-api-server
              |                 |                 |
              +-----------------+-----------------+
                                v
                        agena-application
                                |
             +------------------+------------------+
             |                  |                  |
       agena-session      agena-provider      agena-tool
             |                  |                  |
             +------------------+------------------+
                                v
                            agena-domain

  concrete provider/storage/tool/plugin adapters -> their contracts
  agena-runtime -> application + concrete adapters + configuration
  apps -> runtime + presentation adapters
```

### Forbidden edges

The repository must add an architecture check based on `cargo metadata` and
reject at least these dependencies:

```text
agena-domain          -X-> tokio, reqwest, sea-orm, clap, ratatui, axum
agena-application     -X-> axum, ratatui, concrete provider/storage adapters
agena-tui             -X-> agena-api-server, SQLite, concrete providers, clap
agena-cli             -X-> ratatui, agena-api-server
agena-api             -X-> agena, agena-runtime, agena-application, concrete adapters
agena-client          -X-> agena-domain, agena-runtime, agena-application
```

`agena-api -X-> agena` becomes an active checker rule immediately after the
message-part migration; it is listed here now so the intended final graph is
unambiguous. Only app packages may construct concrete runtime implementations
or perform process-wide initialization.

## Boundary ownership

### `agena-domain`

Owns value types, IDs, roles, messages, event envelopes, permission value
types, model values, execution preferences, and invariants. It allows only
small data-oriented dependencies such as `serde`, `uuid`, `time`/`chrono`,
`smol_str`, `indexmap`, and `thiserror`.

It contains no I/O, async runtime, database access, terminal/UI type, CLI
parsing, concrete provider SDK, or plugin loading.

### Contract and implementation pairs

`agena-provider`, `agena-tool`, and `agena-storage` define contracts.
Concrete implementations live in provider adapter crates, `agena-tools`, and
`agena-storage-sqlite`. Application code depends on contracts, never on
concrete implementations.

Provider adapter crates are separated by substantial external SDK and build
cost, not one crate per source file. In particular, AWS and Google dependency
trees must not be rebuilt by routine TUI work.

### `agena-session` and `agena-application`

`agena-session` owns session state, execution, permissions, rewind/fork,
compaction, history orchestration, and usage accounting. It consumes contract
traits rather than concrete providers, tools, or storage.

`agena-application` owns product use cases. The existing transport-neutral
services currently under `agena-api-server/src/local_api/service` move here
and are split by capability, for example:

```text
SessionService          PermissionService       ProviderService
MessageService          PluginService           ModelCatalogService
WorkspaceService        MemoryService           RuntimeTaskService
```

The application layer may expose a small `Application` container, but must not
become a single unstructured service with hundreds of methods.

### `agena-runtime`

This is a composition layer, not a second monolith. The crate now owns the
runtime builder, reload/background-task contracts, snapshot/task-control
primitives, and tracing reload handle. It must grow to read resolved
configuration, build storage/provider/tool/plugin implementations, create
application services, manage reload/background tasks/shutdown, and return a
typed application handle to presentation layers while keeping those concerns
out of the legacy core.

### Presentation crates

`agena-cli` owns Clap schema, command dispatch, output formatting, and exit
status. It returns a typed launch mode instead of relying on sentinel errors:

```rust
pub enum LaunchMode {
    Tui(TuiLaunchRequest),
    Command(CliCommand),
    AppServer(AppServerRequest),
}
```

`agena-tui` receives an `Application` handle and a typed launch request. It
does not parse process arguments, initialize global tracing, construct a
database, or register providers.

`agena-api` owns versioned request, response, notification, error, pagination,
and envelope DTOs. Domain-to-wire mapping belongs at the API server adapter;
the protocol does not re-export domain structs. Breaking wire changes require
an explicit protocol version bump.

`agena-api-server` owns transports and mappings only: HTTP/WS/SSE/JSON-RPC,
middleware, status codes, request limits, and subscriptions. It does not own
business use cases.

`agena-client` owns remote transports and the public wire client only.

## Final terminal application

The final top-level application is intentionally small:

```text
apps/agena
├── parses process arguments once
├── initializes tracing/runtime/signals once
├── constructs agena-runtime once
└── dispatches CLI, TUI, or API-server launch mode
```

The sole terminal binary is:

```toml
[package]
name = "agena"
autobins = false

[[bin]]
name = "agena"
path = "src/main.rs"
```

The obsolete command is removed:

```text
agena-tui
```

The supported interface is:

```text
agena
agena tui
agena tui --session 42
agena exec "..."
agena provider list
agena app-server
```

## TUI V2 internal architecture

Moving the UI into its own crate is necessary but insufficient. The current
large mutable `App` and its many distributed `impl App` files must be replaced
by a feature-oriented state architecture:

```text
Terminal/Input -> Action -> update(State, Action) -> Effects
                                           |
                                           v
                                      read-only View

Effects -> Application Services -> Backend Result -> Action
```

Rules:

- `State` holds only state.
- `Action` represents user input, terminal events, and backend results.
- `update` changes state and emits effects; it does not render or do I/O.
- `Effect` owns asynchronous/external work.
- rendering reads state/layout only and cannot mutate state or call services.

Recommended layout:

```text
crates/agena-tui/src/
├── lib.rs
├── launch.rs
├── state.rs
├── action.rs
├── update.rs
├── effect.rs
├── backend.rs
├── runtime/
│   ├── terminal.rs
│   ├── input.rs
│   ├── capabilities.rs
│   ├── graphics.rs
│   ├── protocol.rs
│   └── lifecycle.rs
├── features/
│   ├── transcript/
│   ├── composer/
│   ├── sessions/
│   ├── permissions/
│   ├── providers/
│   ├── plugins/
│   ├── settings/
│   ├── usage/
│   └── help/
├── view/
└── testing/
```

The transcript is a dedicated vertical slice:

```text
TranscriptDocument
TranscriptLayout
TranscriptNavigation
TranscriptSelection
TranscriptClipboard
TranscriptRenderer
```

Semantic document units, visual layout rows, navigation, selection, copy
extraction, and rendering must stay distinct. This preserves the required
behavior: a formula or image that occupies multiple visual rows is atomic,
while a code block, table, or plain text block remains logically line- and
text-selectable.

## Migration phases

Each phase may contain several reviewable commits, but every commit must build
and be bisectable. Mechanical moves and semantic changes are separate commits.
When a slice moves, its old implementation is deleted before the phase ends;
no enduring compatibility facade is permitted.

### Phase 0 — Baseline and guardrails

**Current status: complete for the V2 functional baseline.** The metadata architecture checker
is present and enforces the single-binary and forbidden-edge rules, including
API/client legacy-core exclusions using the canonical `agena-core` package
name. The locked workspace Clippy/test/format gates and protocol fixtures pass
on the current worktree. The TUI characterization matrix is now guarded as
well. Its
source-level guards also cover deleted runtime shims, core-free `agena-runtime`
source, and workspace-wide rejection of migrated runtime primitives through old
core facade paths.

1. **[x]** Make full workspace Clippy strict and green.
2. **[x]** Capture CLI help, exit-code, stdout/stderr, and JSON golden tests.
3. **[x]** Capture API request/response/notification fixtures and protocol
   tests.
4. **[x]** The TUI characterization matrix covers startup rollback, terminal
   restore, transcript navigation, mouse/text selection and copy, image and
   formula rendering, and responsive layout. The architecture checker guards
   the named regression tests so this coverage cannot silently disappear.
5. **[x]** Add a metadata-based architecture checker with forbidden-edge
   assertions.
Exit criteria:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

all pass, and the baseline timings/target graph are committed as documentation
or reproducible benchmark instructions.

### Phase 1 — Remove the duplicate terminal entrypoint

**Current status: complete for the entrypoint slice.** `agena-tui` is no
longer a binary target and `agena` is the sole production terminal binary.
The executable architecture checker now proves the remaining parser/startup
code has not regressed to a duplicate path; any future parser change must keep
that gate green.

1. Delete `apps/agena-cli/src/bin/agena-tui.rs`.
2. Remove the `agena-tui` `[[bin]]` target.
3. Delete `AgenaTuiCli`, `TuiCommand`, duplicate `RunCommand`, `run_cli`, and
   duplicate launch-argument conversion.
4. Use the full CLI's typed TUI request for both no-command and `tui` cases.
5. Rename obsolete TUI log naming where appropriate.
6. Update all documentation and tests.

Exit criteria:

```text
Cargo metadata contains no bin named agena-tui.
There is exactly one TUI-oriented parser.
The terminal product has exactly one Rust executable: agena.
```

### Phase 2 — Rename the app and extract CLI presentation

**Current status: substantially complete.** `apps/agena` and
`crates/agena-cli` exist and the final app package owns the `agena` package
and binary name. The CLI presentation boundary itself is extracted; remaining
legacy-core edges are capability consumers (provider/session/runtime slices),
which must disappear as those owners are extracted rather than being recreated
as CLI facades.

The old core package still temporarily owns the name `agena`, so first rename
the application directory to `apps/agena` and use temporary package name
`agena-app` with binary name `agena`.

1. Move `apps/agena-cli` to `apps/agena`.
2. Create `crates/agena-cli`.
3. Move `crates/agena/src/cli.rs` and `crates/agena/src/cli/*` to it.
4. Centralize parsing and launch-mode selection in the new CLI crate.
5. Make `apps/agena` the only dispatcher for `Tui`, `Command`, and
   `AppServer` launch modes.
6. Remove Clap from the old giant core crate.

Exit criteria:

```text
The old core has no Clap dependency.
No command handler returns "must be handled by the binary" sentinel errors.
There is one parser and one top-level dispatch path.
```

### Phase 3 — Extract application services and sever TUI -> API Server

**Current status: substantially complete.** The application crate owns the
moved transport-neutral service slice and the TUI no longer depends on the
API server. Remaining work is to split oversized capability services and
remove the last server-local business logic/runtime-type consumers.

1. Create `crates/agena-application`.
2. Move all transport-neutral functionality from
   `agena-api-server/src/local_api/service` into capability-specific
   application services.
3. Move neutral request/result models out of API server transport modules.
4. Update TUI backend to use application services directly.
5. Update API server handlers to map wire requests onto the same services.

Exit criteria:

```text
agena-tui has no dependency on agena-api-server.
agena-api-server owns no business use case implementation.
CLI, TUI, and API server use the same application services.
```

### Phase 4 — Decouple wire API and client

**Current status: substantially complete.** `agena-api` no longer depends on
the legacy core or application crate, and `agena-client` currently has no
legacy-core/domain/runtime/application dependency. The checker names the
current `agena-core` package, so those legacy-core absences are guarded. The
dependency and public-source audit is now executable, and `agena-client` now
has a protocol-version assertion plus a shared `Command` JSON round-trip test;
its REST/SSE transport also has parser coverage for CRLF normalization,
multiline data, comments, and default event names;
the client additionally completes a real loopback TCP/HTTP round-trip against a
minimal responder for `/api/v1/health`, asserting the request path and decoding
the shared `HealthResponse` resource, and verifies that a non-success JSON
response maps to the shared `ApiError` contract;
the loopback responder now serves checked-in protocol fixtures for both the
successful health response and the not-found error, and architecture checks
validate that those fixtures exist and contain valid JSON;
the architecture checker also locks the server router's `/api/v1/health` route
to the client health transport entry point;
`agena-api-server` now has a real router contract test that builds an in-memory
SQLite schema, constructs `AgenaRuntime`/`AppState`, dispatches health/runtime
query requests and a workspace-create command through the actual Axum router,
and decodes the shared `HealthResponse`, `RuntimeStatusResponse`, and
`WorkspaceResource`; it also requests a missing workspace through the same
router and decodes the shared `ApiError`/`NotFound` envelope. The remaining
work is extending that harness to additional commands. WebSocket hello, pong,
and error frames now have checked-in JSON fixtures decoded by `agena-client`
through the shared `ServerMessage` contract, with architecture checks covering
fixture presence and the server's corresponding hello/pong paths; an actual
upgrade-level server/client loopback now runs against a real bound Axum listener:
the test upgrades with a tungstenite client, verifies the shared protocol hello,
sends a ping, decodes the echoed pong, then sends a shared `Query::Health` and
decodes the request-correlated `QueryResult::Health` through the server
dispatch path. It also creates and deletes a temporary workspace through shared
`Command` frames, validating request correlation and the wire resource/result
mapping. The same loopback also sends a global `SubscribeRequest` and matching
`Unsubscribe`, validating the shared `Subscribed` and `Unsubscribed` acks;
the test then publishes a typed fixture event through the runtime event bus and
decodes the shared `Notification::Event` payload, including its subscription id,
kind tag, and JSON payload. Remaining work is broader event-kind coverage and
resume/replay semantics.

The fixture tests deliberately read those checked-in JSON files at test runtime;
they do not use `include_str!`/`include_bytes!` source embedding. This keeps the
fixture contract observable while satisfying the repository-wide source-
composition prohibition enforced by the architecture checker.

1. Remove the runtime dependency and domain re-exports from `agena-api`.
2. Make API DTOs independently owned and versioned.
3. Implement explicit domain/application <-> wire mappings in API adapters.
4. Remove the runtime dependency from `agena-client`.
5. Upgrade the protocol version if wire compatibility changes.
6. Update Studio, VS Code, and other in-repository consumers in the same
   phase.

Exit criteria:

```text
agena-api has no runtime/application/provider/storage dependency.
agena-client has no runtime/domain dependency.
All protocol fixtures and client/server contract tests pass.
```

### Phase 5 — Extract stable domain values

**Current status: complete for the stable-value scope.** The high-value IDs,
message/part, model, permission, event, and session lifecycle value slices
have moved and their replaced definitions were deleted. The remaining
runtime-bound behavior is intentionally tracked by the provider, storage,
tool, and session-orchestration work in Phase 6; helpers that interpret
runtime attachment/configuration data are not domain values merely because
they return a domain type.
The current audit confirms that the migrated pure-value set (`TodoItem`,
`TableColumn`, `SearchResultItem`, `ArtifactRef`, `PluginInvocation`, and the
UserInput request/question/reply values) has no remaining core struct
definition or `agena::message` re-export; remaining Phase 5 work is therefore
behavior/lifecycle extraction rather than duplicate-data cleanup.
The remaining public-looking message values are intentionally not counted as
unmigrated pure domain data: `MessageProviderState` and `MessageMetadata` still
carry SeaORM JSON-query adapters and provider-persistence fields, while
`MessagePart`, `OperationBlock`, and the interactive `RequestPart` wrappers
couple persistence or lifecycle behavior to core attachment/message types.
Their eventual extraction belongs to the storage/session orchestration phases,
not to a duplicate domain struct move in this phase.

Create `agena-domain` and migrate in dependency-light slices:

1. IDs and newtypes.
2. Roles and model value types.
3. Message value types.
4. Permission value types.
5. Event envelopes and filters.
6. Execution preferences and pure agent configuration.

Each move updates all callers and deletes the original module immediately;
there is no old-core re-export.

The message-activity `TodoItem` slice now follows this rule: `TodoItem`,
`TodoStatus`, and `TodoPriority` are exported from `agena-domain`; the former
core struct and `agena::message` re-export are deleted, while operation-block
consumers import the domain value directly. An architecture test guards the
domain ownership and prevents the core facade from returning. The same slice
now includes the pure `TableColumn` and `SearchResultItem` values used by
operation blocks; their former core definitions and message re-exports are
also deleted. `ArtifactRef` now follows the same path: core constructs it from
attachment data but the stable serialized value is domain-owned.
The plugin-facing `PluginInvocation` projection is likewise domain-owned; core
permission and executor code consumes it directly without a message facade.
The interactive `UserInputOption` and `UserInputQuestion` values now follow the
same rule. `RequestPart` and `UserInputRequest` behavior remain core-owned,
while the reusable question/option payloads are exported by `agena-domain` and
the AskUser input wrapper retains only its core ToolInput validation. The
corresponding `UserInputRequest` and `UserInputReply` values, including their
tolerant single-string/array answer decoding helpers, are domain-owned as well;
`InteractiveRequestPart` remains the core lifecycle wrapper.
`PendingInteractiveRequest` has now moved to `agena-domain` together with its
stable context record. Core still owns `RequestPart` and
`InteractiveRequestPart`, which project concrete persisted message lifecycle
state into that domain value; application consumes the domain request/context
through `SessionQueryService`. The old core enum and message re-export are
deleted, and the architecture checker now protects domain ownership rather
than the former interim arrangement.
The workspace architecture check now scans all production `crates`, `apps`,
and tooling sources for reintroduced `agena::message` paths for these moved
values, rather than checking only the core crate's definitions.

Exit criteria:

```text
agena-domain has no I/O, async runtime, DB, transport, UI, CLI, or SDK deps.
```

### Phase 6 — Extract ports and infrastructure implementations

**Current status: source-disposition closure and unified functional
verification passed on the current worktree.** The provider
catalog/discovery, normalized completion values, non-streaming completion
response, registry-free tool declarations, model metadata/mode resolver
contracts, and the provider-native configuration values (routes, user
location, hosted tools, connectors, and harness references) are in
`agena-provider`. Completion request/stream contracts and the configured
model/capability contracts (with `ModelCapabilityFeature`, full capability
selection patches, `ConfiguredModelSpeedMode`, `ConfiguredModelThinkingMode`,
their mode-map/default conversion values, and the public
`ConfiguredModelDefinition` schema/application API now provider-owned); the
replaced core configured-definition and capability-patch implementations are
deleted. `ModelCapabilityPatch` is no longer re-exported by core; model
catalog, application, API, and app/TUI consumers import it directly from
`agena-provider`. `ModelCapabilityFeature` is also no longer re-exported by
core; configuration overlay and multi-adapter consumers import the provider
contract directly. `ConfiguredModelSpeedMode` is no longer re-exported by core
either; model catalog, configuration, API, and app/TUI consumers use the
provider-owned mode value directly. Deletion of remaining legacy provider
exports is no longer a provider-value task; the remaining work is concrete
model-resolution/composition. `ConfiguredModelThinkingMode` is likewise consumed directly
from `agena-provider` by configuration, catalog, API, and app/TUI code; the
core facade no longer exports that mode value.

**Runtime root-surface closure (active source batch):** Runtime's public root
now exposes application-facing services and stable request/result contracts,
not its bootstrap choreography. `RuntimeBootstrapRequest` and
`RuntimeBootstrapResult` remain the cross-process startup contract. The narrow
`RuntimeBootstrapPreflight` value and
`resolve_runtime_bootstrap_preflight` function are also intentionally public:
the final App process entrypoints need the already-resolved workspace/tracing
projection before they initialize tracing. They do not expose configuration
loading or composition. Bootstrap result composition/lifecycle internals, the
bootstrap factory callback, application-service composition inputs,
snapshot/session/tool/database composition inputs, completion-request builder,
generic execution registry, event-forwarder bridge, task guards, invocation
guard, context governor, and runtime control aggregate are crate-private.
They have no external consumer: the free `bootstrap_application_services`
factory remains the concrete bootstrap entrypoint and
`RuntimeApplicationServices` remains the consumer-facing service bundle. This deletes public concrete
composition edges instead of introducing a facade; the architecture checker
locks the reduced root surface.

The root also no longer publishes any concrete implementation module tree:
agent, configuration, database, event, message, model-catalog, permission,
plugin, provider, session, and tool internals are crate-private. The final
Application session-list fixture no longer reaches through `runtime::db` to
seed Runtime CRUD rows or entities; it creates its parent/child summaries and
event envelopes through the existing storage and storage-sqlite contracts.
Intentionally public Runtime contracts continue to be explicitly re-exported
at the root. This removes the last broad concrete import escape hatch rather
than hiding it behind a compatibility namespace or a test-only Runtime facade.

The same rule applies to parsed configuration internals: `ConfigError`, the
raw override parser and its parsed request/value/error types are crate-private.
Process entrypoints retain only `RuntimeBootstrapRequest` with raw override
expressions; Runtime performs parsing and schema-adapter wiring during
composition. The public settings service and explicit bootstrap request remain
the supported cross-layer contracts.

**Application stable-port disposition (2026-07-24):** the remaining public
Application getters for configuration settings/control, plugin host, session
query/execution, and event query/stream capabilities are deliberately typed
Runtime service contracts, not concrete Runtime trees, snapshot records, or
implementation helpers. Their consumers need capability-specific operations
such as a session stream subscription, a long-running background task, or an
authenticated plugin callback; moving a getter without moving every such
lifecycle would create a forwarding facade rather than delete an owner. By
contrast, the authentication, model-catalog, runtime diagnostics, and agent
listing paths that had complete upper-layer lifecycle/projection owners were
moved into Application and their old service escapes deleted. The remaining
draft-provider authentication path is similarly retained only while its
terminal draft/config validation lifecycle is inseparable from the concrete
editor; it must be reconsidered only with a complete opaque draft use case,
not by wrapping `RuntimeDraftAuthenticationService`.
The project/default-workspace path helpers are private too; only the
CLI-required default-config path remains an explicit public process utility.

**Phase 6 source-disposition closure (2026-07-24):** the five `[x]` entries
below preserve the limits of the extracted contracts; they are not a second
unbounded source queue. Provider catalog fetch/ranking/persistence is Runtime
composition policy, not a Provider-contract implementation. SQLite
transactional session/history choreography remains with the concrete
storage/session adapters so permission and history writes stay atomic. Detailed
executor payloads, attachments, streaming state, and session-manager write
orchestration remain Runtime-private because no runtime-neutral contract can
carry them without recreating a concrete public edge. The residual
presentation/composition work is governed by the Phase 8 complete-slice rule.
These owners were audited against normal consumers and recorded as deliberate
retentions; only a functional failure or a newly identified complete deletion
can reopen them.

The built-in web plugin follows the same composition boundary. Its plugin ID,
configuration tree, concrete plugin type, and constructors are Runtime-private;
Runtime's plugin registration remains the only owner. Application and transport
layers consume plugin/runtime services rather than obtaining a web-plugin
construction escape hatch.

The concrete plugin-host JSON-RPC bridge is private for the same reason:
`dispatch_plugin_rpc` accepts `PluginHost` only inside Runtime's implementation
of `PluginRuntimeService`. API and other transport code call the service method
instead, so the host cannot reappear as a public callback-dispatch dependency.

**OAuth callback listener closure (current):** Runtime retains the TCP
bind/accept loop, callback URL parsing, CSRF-state comparison, provider-error
description/request-ID diagnostics, and HTML escaping behind
`RuntimeAuthenticationService`. `Application::complete_auth_browser_callback`
now owns the upper-layer wait-and-finish lifecycle, so CLI receives neither a
Runtime callback value nor the Runtime authentication port. The former public
`wait_for_oauth_callback` root function is crate-private, so a process
consumer cannot bypass the authentication capability and recover a concrete
listener edge.

**Provider client-version fetch closure (active source batch):**
`Application::refresh_provider_client_versions` still owns the complete user
use case—fetch, settings patch, and conditional reload—but it now obtains the
network result through `RuntimeControlService::fetch_provider_client_versions`.
The underlying Runtime HTTP helper and its fetch-error type are root-private.
This preserves Runtime ownership of npm-registry transport, timeout, and active
client-version state without letting Application or a transport consumer call
the concrete network helper directly.

**Process-metrics closure (active source batch):** Application now reads the
stable `RuntimeMetricsSnapshot` through `RuntimeControlService::runtime_metrics`.
The global atomic snapshot function is root-private Runtime implementation
detail; Application continues to project it into its transport resource. This
removes the last upper-layer call to a Runtime global-process helper without
moving metric ownership or counters out of Runtime.

The same public-surface rule now closes the Runtime background-task machinery:
the registry, mutable task index, registration specification, and terminal
completion enum are crate-private algorithms. Upper layers retain the stable
task snapshots/start results and `RuntimeControlService`; they cannot acquire
or compose the registry itself. This deletes an unused concrete root edge
without weakening task-control behavior behind a facade.

The concrete `AgenaRuntime::current_snapshot` escape hatch is now explicitly
crate-private as well. Every call site is Runtime-internal (reload, host-client,
or service implementation); normal consumers receive the appropriate
application/runtime capability instead of an `Arc<RuntimeSnapshot>`. This
removes another concrete return edge without relocating Runtime composition
policy into Application.

The same visibility closure now applies inside `RuntimeSnapshot`: all of its
configuration, provider-registry/catalog, session, MCP/LSP, PluginHost, model
resolution, reload, and maintenance accessors are `pub(crate)`. Although the
snapshot type was already crate-private, leaving those methods `pub` created a
misleading future re-export surface for concrete composition. Runtime services
remain the only intended external boundary, and the architecture checker locks
the representative concrete accessors to crate visibility.

The RIFT binary environment lookup is private for the same reason: it is used
only while Runtime composes snapshot capabilities, not by Application's stable
snapshot query/control contracts.

`SnapshotMetadata` follows that closure as well. Its generation/timestamp pair
is an internal snapshot-state bookkeeping value with no upper-layer consumer,
so both the type and its root re-export are crate-private rather than a second
snapshot-lifecycle contract.

The same closure now applies to `ToolExecutor` and
`SessionManager::tool_executor`: both are Runtime-private concrete execution
machinery. External callers use `SessionToolExecutionService` and its summary
contract; no App, CLI, Studio, API-server, or E2E source directly obtains the
executor. Their declarations are explicitly `pub(crate)`, preventing a future
public concrete execution escape hatch from growing behind the service boundary.

**Snapshot inspection closure (active source batch):**
`SessionExecutionControl` no longer returns the mutable
`SnapshotRegistry` (`Arc<RwLock<HashMap<…>>>`) to Application. Runtime now
projects `RuntimeSnapshotStatus` with stable active/managed rows directly from
its private registry; Application maps that projection to its Git/snapshot
resources and no longer calls `list_active_snapshots` or
`list_managed_snapshots`. `SnapshotRegistry`, `SnapshotSession`,
`ActiveSnapshot`, `ManagedSnapshot`, and their constructors/list helpers are
root-private Runtime implementation details. This deletes the actual
Application-to-Runtime concrete return edge while keeping snapshot discovery,
backend inspection, and filesystem/process composition in Runtime.
The final process-probe escape on that path is closed too:
`snapshot_backend_capabilities` is now root-private and Runtime implements it
through `RuntimeControlService`. `Application::snapshot_status` asks its
composed control capability for the stable `agena-tool`
`SnapshotBackendCapabilities` value, then performs its existing DTO
projection. Application no longer invokes a public Runtime free function to
start `git`/Rift capability detection; the public contract is the stable
result value, not the concrete process probe.

`SessionManager::event_publisher` is likewise crate-private. The prior comment
describing an API-server transport consumer was stale: transport publication now
uses Runtime/Application event services, and the repository contains no
external manager publisher consumer. Keeping this publisher inside Runtime
prevents direct event-bus wiring from bypassing the typed event boundary.

`ProviderRegistry` is explicitly crate-private too. It remains Runtime's
concrete adapter registry for provider clients and plugin patch installation;
the public surface is the provider catalog/configuration/query service family,
not the mutable adapter map. No normal consumer imports the registry type.

**Active repository-composition cutover:** Runtime now carries its
contract-typed `RuntimeApplicationRepositories` alongside the application
service bundle and constructs the concrete `MemoryStore`/SQLite repository
adapters exactly once at Runtime composition. App, CLI, and Studio use
`Application::from_composed_runtime_services`; they no longer
obtain `DatabaseConnection`, construct `Sea*Repository`, or retain normal
`agena-storage-sqlite`/`sea-orm` manifest edges merely to build an Application
handle. The public bootstrap result likewise returns only composed service
contracts and lifecycle control, not a `DatabaseConnection`. This is a real
concrete-edge deletion, not a compatibility facade: Application receives only
`agena-storage` repository traits through Runtime; the former public
`ApplicationRepositories` wrapper and alternate consumer-side constructor are
deleted. API server's SQLite/SeaORM dependencies are dev-fixture-only; normal
API routing consumes the precomposed Application handle. Isolated tests build a
Runtime composition fixture instead of reintroducing a second application
assembly path.

The same boundary now owns database URL/path realization: App, CLI, and Studio
pass raw `database_url`/`database_path` intent in `RuntimeBootstrapRequest`.
Runtime alone applies `StorageConfig` URL selection, parent-directory creation,
connection reuse, and schema initialization. The terminal App and Studio no
longer have an `agena-storage` manifest edge just to bootstrap SQLite; the API
server's now-unused normal storage dependency is deleted too, and the CLI no
longer maps a storage-bootstrap error before Runtime has started. This deletes
the upper-layer concrete bootstrap policy rather than merely moving its call
into a helper.

The process-facing `RuntimeBootstrapRequest` now follows the same rule: normal
entrypoints provide workspace/config/database URL intent, never an injected
SeaORM connection. Runtime's internal composition config retains explicit
fixture injection only where isolated adapter tests need it; that internal seam
is not a normal consumer contract.

The sole public bootstrap path is now `bootstrap_application_services`.
`AgenaRuntime::from_bootstrap_request` is deleted and concrete
`AgenaRuntime::new` is Runtime-private; API-server fixtures use the same public
bootstrap request/result as production instead of constructing a runtime with
an injected SQLite connection. Accordingly API server no longer has normal or
dev-only SQLite/SeaORM manifest edges for that fixture.
The `runtime` implementation module, `AgenaRuntime`, and `RuntimeSnapshot` are
crate-private too; external callers can name neither the live runtime nor its
snapshot registry, and must use the bootstrap result's stable services.
The generic process/snapshot state and every model-catalog, plugin, session,
tool, database, and snapshot composition-input record are now explicitly
`pub(crate)` as well. Their prior `pub` declarations were only accidentally
hidden by a private module; making the declaration private removes that
ambiguous concrete surface and leaves only the actual Runtime service/request/
result contracts public.
`RuntimeCompositionConfig`, its database-connection fixture input, bootstrap
preflight, and composition callback/lifecycle values are Runtime-private as
well, so no normal crate can bypass the process bootstrap contract.

The default-workspace terminal memory commands now follow the same storage
boundary. `MemoryRepository` owns index creation as well as list/get/save/
forget; Application exposes typed index-path, entry-path, and forget use cases.
The App backend no longer recreates `MemoryStore` from its workspace root, and
adapts only those Application results to editor/clipboard/process effects.
The CLI's explicit `memory --workspace` commands now bootstrap the chosen
Runtime workspace and use the same Application memory use cases; the prior
CLI-local `MemoryStore` construction, repository trait cast, and duplicate
index-file helper are deleted.

The same root audit also makes model-catalog cache/source/HTTP/curation/build
helpers, monitor registry construction, plugin-host assembly and slot
installation, project-path calculations, provider-list composition, local
model selection, provider ranking, and configured-agent registration
crate-private. The public catalog refresh service remains the Runtime boundary;
model-ID normalization is a provider value consumed directly by
Application/API/App. This removes the Runtime utility export while preserving
the Runtime service boundary.
Scheduling and periodic-loop helpers, prompt budget/merge policy, provider
client-version cache internals and SSE parsing, refresh/watch machinery, generic session
cache, snapshot creation/pruning maintenance operations, task/snapshot storage state, tool-output
truncation, usage aggregation, and watch-path bookkeeping follow the same
crate-private rule. `RuntimeReloadCause`/report and the concrete service ports
stay public where transport/application code consumes them. The public
provider-version refresh operation now crosses `RuntimeControlService` for the
App's explicit network-refresh action. Application's Git use cases consume the
stable snapshot-status projection and the control-port capability result; they
do not retain public managed-snapshot list helpers or a public backend-process
probe. Snapshot creation/pruning implementation helpers remain internal.
Compaction limits, LSP/MCP/memory plugin assembly, and metric-recording hooks
are internal for the same reason: their concrete registries and lifecycle
arrangement are Runtime composition policy. Runtime remains the metric-counter
owner, but the API metrics endpoint now consumes an Application metrics
projection rather than calling the Runtime snapshot function directly.
Runtime likewise remains the persisted UI-configuration owner, while
`Application::tui_preferences` projects its locale/theme/color/graphics values
into an Application resource for terminal startup and palette reload. The App
no longer carries `RuntimeUiConfiguration` or Runtime presentation enums;
this removes a concrete Runtime return edge without moving configuration
resolution or persistence out of Runtime.
The terminal Model Catalog backend now follows the same command/query boundary:
`Application` owns catalog item projection, query matching, pagination,
canonical-ID lookup, and the user-refresh command, while Runtime retains
catalog curation, persistence, and task execution. The old App-local Runtime
response projection/search helpers and direct `ModelCatalogRuntimeService`
calls are deleted rather than retained beside the Application use case.
API model-catalog list, origin filtering, canonical-ID lookup, and refresh now
consume those same Application operations. The REST module no longer projects
a Runtime catalog response, starts a Runtime catalog task, or exposes an
`AppState`/`Application` `ModelCatalogRuntimeService` accessor; its former
resource/search conversion tree is deleted with the direct service edge.
Provider client-version refresh is likewise an Application command: it performs
the Runtime-owned network fetch, settings patch, and conditional reload as one
operation. The App retains only localized feedback and no longer owns the
cross-layer fetch/persist/reload choreography.
Terminal agent/default-agent and diagnostic-summary reads now use Application
projections too. Runtime remains the status-service implementation, while App
no longer reads its status record directly merely to render provider/plugin
counts or select the configured default agent. API health, readiness, and
metrics consume that same compact Application summary; the obsolete public
Application Runtime-status port and API-state getter are deleted. The one
remaining full-status accessor is `pub(crate)` for Application's existing wire
query dispatch only; it is not an upper-layer Runtime escape hatch.
The complete JSON configuration-source read model now has the same boundary:
`Application::config_json_sources` assembles the resolved paths/layer evidence,
file and workspace-file JSON, and effective JSON default aliases from Runtime
services once. App settings/provider/permission/plugin presentation consumes
that Application resource directly; the former App-local `ConfigJsonSources`
owner and effective-config augmentation helper are deleted. Concrete settings
writes and reload effects remain on their explicit Runtime settings/control
ports. API effective settings/list routes and marketplace configuration-path
lookup consume the same Application projection, so transport code no longer
reads `RuntimeConfigurationService` directly for this read model. The former
public `Application::runtime_configuration` accessor is now `pub(crate)` for
Application projection implementation only, preventing a new upper-layer
configuration escape hatch.
The Codex user-agent constructor, connection-reuse helper, installation-id
resolver, and all prompt-window estimators are crate-private. The former
public `context_usage_percent_used` helper was a terminal-only display policy,
so it is deleted from Runtime rather than kept as a public budgeting escape
hatch; the coherent terminal status projection now owns that calculation.
`ConfiguredModelModeMap` is now also consumed directly from `agena-provider`
across configuration, catalog, API, and app/TUI boundaries; the core provider
facade no longer exposes the mode-map container.
`ConfiguredModeDefault` follows the same boundary: catalog merge, API mapping,
and app/provider configuration code now import the provider-owned default value
directly, with no core facade re-export.
`ConfiguredThinkingStrategy` is now direct-provider-owned at the remaining
configuration, API, and app boundaries as well; the core provider facade no
longer re-exports the strategy enum.
The public `ConfiguredModelDefinition` schema is also consumed directly from
`agena-provider` by configuration, catalog, multi-adapter, API, and app/TUI
code; its former core provider re-export has been removed.
`ModelCatalogSnapshotSourceKind` (`generated`/`cache`) is now provider-owned
as catalog provenance; core model-catalog storage/service and
API/application mapping code import it directly from `agena-provider`, with no
core catalog re-export.
The resolved runtime model-catalog projection is now provider-owned as
`ProviderModelCatalog`; core retains source-ranking and persistence records,
then converts the merged result once at runtime composition. `CatalogedModelsProvider`
therefore consumes only provider `ConfiguredModelDefinition` values rather than
holding a core `ModelCatalogProviderRecord`.
The provider contract deliberately excludes persistence format, source-ranking
metadata, and refresh timing; those concerns are now documented and tested as
runtime/catalog-composition responsibilities.
`ProviderClientVersions` is provider-owned as a stable three-version value;
Runtime now owns its bounded HTTP refresh, process-local active-version state,
and Codex/Claude/Gemini HTTP identity construction. Core snapshot composition
only projects the already-resolved three values into Runtime once per snapshot;
the old Core provider-runtime helper module is deleted. The Runtime web plugin
therefore constructs its own configured Claude fetch identity without a Core
registration callback.

The remaining detailed executor surface is Runtime-private as well:
`ToolPayloadInput`, `ToolPayloadOutput`, `ToolOutputTruncator`, and
`BuiltinToolSet` are no longer public Runtime re-exports. They carry concrete
executor payloads, truncation state, and built-in plugin assembly, so external
consumers must use the stable session-tool service/summary boundary instead.
No external source consumer remains; this removes a concrete return/composition
edge rather than introducing an adapter facade. The same rule now includes the
tool-registry wildcard: `ExecutionTool`, `ToolApiBinding`, `ToolExecutor`,
streaming executor state, and built-in plugin constructors are Runtime-private
composition details rather than a second public tool-execution API.

Runtime's managed credential lifecycle is private too: `ManagedCredential`,
SAP AI Core key parsing, and retry-status classification are no longer public
provider exports. They remain concrete Runtime adapter concerns and external
callers consume the corresponding Runtime services rather than credential
implementation state.

`ModelRuntime` and the model-catalog decoration adapter are private Runtime
composition details as well. The former is the concrete trait implemented by
Runtime provider adapters, while the latter borrows that trait to decorate a
catalog during Runtime composition; neither is a stable provider contract.
External consumers use `agena-provider` contracts and Runtime catalog/query
services instead of constructing or downcasting Runtime adapters.

The concrete Runtime `provider::auth` implementation is crate-private too.
It contains keyring storage, device/browser OAuth exchange, refresh, and
provider-specific credential management; the terminal, CLI, and API use the
stable `RuntimeAuthenticationService` rather than importing that adapter tree.

Session subtask execution has the same boundary: the concrete
`SessionSubtaskRequest`/`SessionSubtaskResponse` records and `run_subtask`
method are Runtime-private. Host-client composition invokes them internally;
outside consumers use the stable application/session capability surface.

`SessionManager` itself is now private as well. `RuntimeSnapshot` and
`AgenaRuntime` retain crate-private manager accessors only for concrete
composition/lifecycle work; all external process consumers receive the
Runtime application-service ports. The API-server WebSocket contract fixture
now publishes its notification through `RuntimeEventPublishService::PluginEvent`
instead of obtaining the manager's concrete event bus.

The adjacent concrete session models are private too: `Session`, its runtime
state/prompt-window/compaction records, `SessionProcessor`, and the historical
projected-header record are no longer public `agena_runtime::session` exports.
They are storage/execution implementation state, not the public session-query
projection; external consumers use `SessionQueryService` and its
`SessionProjected*` values instead. `project_message_part` remains public
because the terminal app currently consumes that explicit, stable projection.

The per-session cost adapter is private for the same reason:
`session::cost::summarize` accepts concrete Runtime `Message` history and is
only used by the session-query implementation. Consumers receive its stable
Domain `SessionCostSummary` through `SessionQueryService::usage_stats`, rather
than importing an algorithm coupled to Runtime message storage.

Provider prompt-cache control is now owned by `agena-provider` too:
`PromptCacheControl::ephemeral` and cache-target selection are shared protocol
values used by OpenAI, Anthropic, and Bedrock adapters. Core's former
`provider/prompt_cache.rs` implementation is deleted; its remaining adapter
sources use the provider crate through a private import alias while their SDK
request construction is still being extracted.
Provider stream and item identifiers now follow that same contract boundary:
`ProviderStreamKey`, `ModelToolCallId`, `ProviderItemId`, and OpenAI Responses
call-ID normalization live in `agena-provider`. Core's former
`provider/protocol_ids.rs` implementation is deleted; its current stream and
adapter consumers use a private provider-crate import alias until their
concrete request/stream assembly is extracted from Core.
The associated tool-call stream accumulator is provider-owned as well:
`ToolStreamInput`, `ToolStreamUpdate`, and `ToolStreamAccumulator` preserve
provider event identity and reconcile argument deltas without knowing anything
about sessions or persistence. Its explicit `ToolStreamError` is converted to
Core's application error only at the adapter boundary, and Core's former
`provider/tool_stream.rs` implementation is deleted.
The strict prompt-envelope protocol values are provider-owned too:
`PromptToolCallsEnvelope`, `PromptToolCall`, `PromptToolDefinition`, and
`PromptToolResult` define the shared serialized shapes consumed by request
construction, response validation, streaming decode, and replay-history
projection. Core retains only the prompt construction and execution adapter;
the architecture checker rejects restoring duplicate Core protocol structs.
The incremental prompt-envelope decoder is co-located with those values in
`agena-provider`: it preserves split markers, validates only complete JSON
call payloads, and safely returns malformed or incomplete blocks as ordinary
text. Core now consumes its decoded text/call items while retaining only the
`AppError`-typed adapter stream wrapper.
Anthropic Messages content blocks are provider-owned too: the text/thinking,
tool-use/result, image/document source records and their JSON-object handling
now live in `agena-provider`. Core's Anthropic adapter only projects requests
and response/session behavior through those wire values; their protocol-only
unit regressions moved with the definitions.
The remaining Anthropic Messages wire family has now moved with them: request
and response envelopes, model-list records, token/cache usage, and streamed
event/delta payloads are all `agena-provider` protocol data. The former Core
`anthropic_wire.rs` module is deleted; Core retains concrete HTTP transport,
credential refresh, and completion/session projection only.
Anthropic thinking policy is provider-owned as well: model-specific adaptive
and effort rules, request fragments, token/cache usage mapping and merging,
thinking replay metadata, and transient streamed-thinking state now live in
`agena-provider` with their contract regressions. Core imports the policy and
maps only its concrete adapter stream into its application error boundary.
Gemini's model-specific thinking policy follows the same boundary: the
Gemini 2.5 budget clamps, Gemini 3 thinking-level mapping, display behavior,
and serialized `GeminiThinkingConfig` now live in `agena-provider`. Core's
Gemini adapter consumes that provider request fragment directly without
retaining a parallel policy implementation.
Gemini usage decoding is provider-owned too: `GeminiUsageMetadata` preserves
the provider wire names, while its normalized projection accounts for the
cached-token-inclusive prompt total and separate thought-token count. Core
stream handling now consumes the provider conversion directly.
Gemini model-list wire values and their input/output limit projection are
provider-owned as well. `GeminiModel::metadata` converts published provider
ceilings into the stable domain metadata contract; Core only merges that
projection into its concrete adapter/catalog result.
Gemini shared content/function-call wire is provider-owned too: request
projection and streamed response decoding now use the same `GeminiContent`,
`GeminiPart`, function call/response, and inline-data contract from
`agena-provider`. Core retains validation that maps malformed provider output
to its application error only.
Gemini's complete request wire has moved as well: generate-content and Live
conversation envelopes, generation options, function declarations, and tool
configuration are all provider-owned. Core now decides concrete tool routes
and sends the serialized contract, but no longer defines Gemini request JSON.
Gemini generate-content response envelopes and candidate records are now
provider-owned too. Core retains only the adapter-local projections that turn
candidate text, reasoning, malformed function calls, and metadata into its
application completion/error flow.
Gemini Live server envelopes are provider-owned as well: setup, server-content,
and tool-call records now live in `agena-provider`. The concrete adapter keeps
only the metadata projection and its streaming/error orchestration, expressed
as Core free functions rather than inherent methods on Provider protocol types.
Ollama's tags, chat request/response, tool-definition, options, and function
call records are likewise provider-owned wire data. Core retains only
adapter-specific endpoint/HTTP behavior and maps tool-call validation failures
at the application-error boundary.
Ollama token counters now normalize in `agena-provider` too: the shared
conversion preserves Ollama's prompt/eval counters as stable completion usage,
while Core only decides when the concrete stream has completed.
Provider HTTP utility ownership is now explicit as well: request-shape
fingerprints, base-URL and authorization-header normalization, case-insensitive
header composition, cache-safe header selection, JSON-object request patches,
and optional-text normalization live in `agena-provider`. Core retains only
the plugin-host hook, concrete HTTP I/O, logging, and `AppError` translation;
its adapter call sites continue through a private re-export during this
continuous source batch rather than duplicating protocol helpers.
Runtime now also owns the concrete HTTP stream decoder used by provider
adapters. Its `JsonEventPayload` parser preserves split UTF-8 frames and SSE
`[DONE]` handling, while `ProviderJsonStreamError` keeps transport/JSON failures
outside the provider contract. Core maps that explicit Runtime error at the
adapter boundary; the former Core `provider/sse.rs` is deleted.
Copilot model-list response projection also belongs to `agena-provider`:
`CopilotModelExtension` deserializes provider-specific response metadata and
derives shared model capabilities/limits without any configuration or session
knowledge. The former Core `provider/copilot_models.rs` is deleted, leaving
only the concrete OpenAI/Anthropic adapter request paths in Core.
Tool-mode request policy is now provider-owned too: applying a route's tool
mode, removing provider-native body fields, projecting disabled tool history,
and validating a disabled-mode response all operate solely on provider
contracts. `ProviderToolModeViolation` crosses into Core only through its
application-error conversion. Core retains only the `AppError`-typed stream
guard required by its current adapter stream abstraction.
OpenAI Responses response wire values have also moved to `agena-provider`:
the response/status/error envelope, output/reasoning items, token-usage
records, and reasoning-delta recognition are protocol data independent of
Core. Core's remaining OpenAI response helper module now owns only concrete
request-item cache hints, session text projection, Runtime client-version
selection, and adapter-local conversions.
OpenAI-compatible Chat Completions usage decoding is provider-owned as well:
the alternate prompt/input and completion/output field forms, cache and
reasoning detail records, and normalized `CompletionUsage` projection live in
`agena-provider`. Core's Chat wire adapter retains response/error and message
projection behavior, while all existing usage regressions continue through the
provider-owned conversion.
The matching Chat response-format encoder (`text`, JSON object, and JSON
schema) is provider-owned too, so Core's request builder consumes the provider
wire value directly rather than defining a parallel serializable enum.
OpenAI-compatible reasoning-effort selection is provider-owned as well. The
`ThinkingRequest` mapping, GPT-5 `none` support, and provider-specific effort
normalization now live with the Chat protocol contract; Core's request builders
only invoke that policy.
The Chat completion/stream response envelope, choice/delta records, and tool
call wire payload now live in `agena-provider` too. Core keeps the conversion
from those values into application errors and completion/session projections.
OpenAI-compatible tool-definition and function-schema wire values are likewise
provider-owned; Core's request projection creates the provider types directly
without maintaining duplicate serializable declarations.
The complete OpenAI-compatible Chat request envelope, message record, stream
usage option, and request-side tool-call record now live in `agena-provider`.
Core's remaining `chat_wire` code is therefore an adapter-only projection and
validation layer, not a home for protocol data definitions.
The obsolete Core `provider/types.rs` compatibility slot is deleted as well;
provider usage values are consumed directly from `agena-provider` and no Core
conversion remains behind that module name.
`ProviderHttpClientConfig` now follows the same boundary as a dependency-light
timeout value; only `ProviderRegistry` constructs the concrete `reqwest` client,
so provider configuration callers no longer pass a core-owned HTTP config type.
`GeminiStreamMode` is likewise provider-owned as the adapter transport policy;
core configuration parsing maps its persisted `StreamTransportMode` into that
value, while the Gemini adapter consumes it directly without a core facade.
`OpenAiResponsesBackend` now follows the same rule: the provider adapter owns
the runtime backend policy, and core configuration maps its persisted backend
enum into the provider value without re-exporting the adapter policy.
`OpenAiProfile` is now provider-owned as the OpenAI-compatible identity policy;
registry composition and adapter options import it directly from
`agena-provider`, while core keeps only concrete adapter construction.
`AnthropicProfile` follows the same boundary for Anthropic-compatible identity
selection; registry composition imports the provider value directly and the
core facade no longer exposes the profile enum.
`AuthSecretSelector` and `AuthRefreshStrategy` are now provider-owned credential
policy values as well; core retains `ManagedCredential`, `AuthData` handling,
refresh I/O, and persistence integration, while registry/auth-resolution code
imports the strategy enums directly from `agena-provider`.
The SAP AI Core service-key payload (`SapAiCoreServiceKey` and its nested URL
value) is provider-owned too; core's JSON parser returns the provider contract,
while credential storage and token acquisition remain implementation details.
The complete provider auth value family (`AuthData`, `CredentialIssuer`,
`OAuthUserInfo`, `OAuthTokenResponse`, and `CopilotDeployment`) now lives in
`agena-provider`; API, CLI, app, configuration, and adapter consumers import
those values directly. Core `AuthManager`, `AuthStore`, OAuth callback/device
flows, and credential persistence remain concrete implementations with no auth
value definitions or compatibility re-export.
`OAuthCallback` is also provider-owned as the parsed `{code, state}` result;
core retains only redirect URL parsing, localhost listener handling, and error
presentation, while CLI/auth orchestration consumes the provider value directly.
`GitlabProviderConfig` is now provider-owned as the runtime routing/configuration
payload; core keeps only its default construction (which supplies the dynamic
user-agent headers) and the concrete GitLab adapter implementation.
The `ModelModeResolver` port is likewise no longer re-exported by core; its
implementation and consumers use the provider-owned trait directly.
The remaining core `ToolApiBinding`, `ToolPayloadInput`, and
`ToolPayloadOutput` values are executor-layer contracts rather than provider
contracts: they carry `RegisteredTool`, core attachment, message-part, and
`OperationBlock` types. They remain in core until the executor/session
composition slice introduces an explicit runtime-neutral result boundary. The
first piece of that boundary now exists as `agena_tool::ToolExecutionSummary`:
core `ToolExecutionView` projects title/output/metadata into it, while
attachment metadata now projects through `ToolAttachmentSummary`; concrete
attachment source/content types remain deliberately core-bound. Both
`ToolPayloadExecution` and `ToolInvocationExecution` expose this projection,
so callers do not need to know which core payload variant produced the result.
The summary now also carries the optional serialized JSON payload, allowing
transport and presentation adapters to forward structured results without
calling core `ToolOutput::to_json_payload` themselves.
The plugin after-hook input now consumes that projection for title/output and
metadata, making the boundary an active execution path rather than a test-only
adapter. Session tool-completion assembly now uses the same summary for
title/output/metadata while retaining only the core attachment list and
operation-block conversion locally. Model-output boundary calculation also
uses the summary's output text before comparing against provider payload JSON.
The REST plugin-tool response, in-process plugin router, and runtime host-client
mapper now consume the same summary for their title/output/metadata fields;
only concrete attachment transport remains a core-owned concern in those
adapters.
CLI MCP tool results, skill prompts, and JSON apply rendering also consume the
summary for their read-only text/title projections, keeping presentation code
independent of the concrete execution view.
The plugin after-hook payload and CLI MCP skill-list payload now use the
summary's serialized payload as well, removing their last direct payload
conversion at the boundary.
Plugin after-hook presentation updates now return through
`ToolExecutionView::apply_neutral_fields`; the hook no longer writes title,
output, and metadata fields directly, while core-owned attachments remain
untouched. This is the first explicit mutable-application seam for the
executor result boundary.
Result-policy and model-output-boundary metadata/output updates now use the
same `set_neutral_output` and `insert_neutral_metadata` seam; only payload
truncation/marking remains a concrete core operation.
The executor-level output truncator now uses `set_neutral_output` for its
presentation text while retaining concrete payload truncation locally.
The summary contract accepts legacy JSON without attachment metadata, so adding
the boundary does not invalidate existing persisted or plugin-produced result
shapes.
The external executor return boundary is now complete: the detailed
`ToolExecutionView`/payload/invocation results, the opaque post-authorization
capability, and the host-invoked detailed execution path are crate-private
Runtime implementation details. Public callers cross only
`SessionToolExecutionService` and receive `ToolExecutionSummary`. The detailed
objects remain inside Runtime because they carry attachment, payload, and
streaming execution state; extracting them as a contract would recreate the
concrete boundary rather than remove it.
The pure `ExecutionLifecycle` state machine and its transition error have now
also moved to `agena-domain`; session execution registries consume the domain
value directly, while orchestration remains in core.
`SessionUsageLimitBasis` follows the same rule: the context-window versus
prompt-threshold decision is domain-owned, while API keeps its own explicit
wire mapping.
The session request/read-model boundary has also crossed the core facade:
`SessionCreateRequest`, run/execution/reply/permission/fork/rewind requests,
and agent restore/switch outcomes are runtime-owned request values; the
transport-neutral `SessionListRequest` and `SessionSummary` live in
`agena-domain`. Application, CLI, app, API, and E2E callers import those
contracts directly rather than through `agena::session`, and the old public
core facade exports are deleted. This is deliberately only a value/read-model
move: the concrete `Session` message projection and `SessionManager` write /
execution lifecycle remain core-bound until the service-port slice is moved.
The `SessionUsage` measurement aggregate is now domain-owned as well; core
session manager code computes it, but does not define or re-export the value.
`PromptTokenUsageSnapshot` is now domain-owned as a reusable token measurement
value; core retains only the adapter from its message-usage persistence value
and the larger prompt-window runtime state.
`PromptCompactionActivity` is also domain-owned as the safe, provider-agnostic
activity-log projection; core retains only the checkpoint content/runtime
state that depends on messages and provider-native payloads.
`DoomLoopPolicy` and `DoomLoopHit` now follow the same split: policy/result
values live in domain, while the message-scanning detector remains in core.
`ContextPolicy` is also domain-owned; `ContextGovernor` remains core because it
coordinates prompt-window runtime behavior.
`UsagePeriod` is now domain-owned as the stable reporting-period value; core
retains calendar-window construction and usage aggregation in `UsageStatsQuery`.
`SessionAutoCompactionConfig` is now a domain policy value nested inside the
Runtime-owned `RuntimeSessionManagerConfig`; Core imports it directly at the
concrete SessionManager adapter boundary. Orchestration remains Core-bound,
while cache policy construction is Runtime-owned.
`SessionCacheStats` is now a domain-owned result value; the cache policy and
in-memory cache implementation are Runtime-owned, while Core retains only the
session-specific cache-entry adapter.
The session-manager facade for this value has also been removed; the public
`SessionManager::cache_stats` method returns the domain type directly.
The presentation-safe cost values (`SessionCostSummary` and its
`ModelCostBreakdown` entries), the transport-neutral `UsageStatsQuery`, and the
usage-report response values (`UsageStats`, totals, and every breakdown) now
live in `agena-domain`. Core retains only private folds from Provider
`CompletionUsage`, persisted-record filtering, and calendar aggregation; the
provider-pricing policy is Provider-owned. `ExecutionControlError` likewise
remains an in-process registry
error because it describes cancellation/steering channel state and maps
directly to core `AppError`.
`CapabilityFamily` and `CapabilityResolver` are now direct provider contracts
through the adapter, registry, and capability-registry implementations; the
core provider facade no longer re-exports either type.
The capability rule table and `default_capability_registry` implementation
now live in `agena-provider`; core adapters call that provider-owned resolver
directly, and the former core `provider/capabilities.rs` implementation is
deleted rather than retained as a facade.
`CapabilitySelectionPatch` and `CapabilitySelectionPatchBody` are also
consumed directly from `agena-provider` by catalog, configuration, API, and
app/TUI code; the core facade no longer exposes the selection patch wrappers.
Provider-native tool artifact, output-block, and search-result values are also
directly consumed from `agena-provider`; their former core facade exports have
been removed.
The `configured_thinking_payload_selector` helper is likewise called directly
from `agena-provider` by model-catalog enrichment; core no longer re-exports
that provider helper.
`ModelMetadataRegistry` and `default_model_metadata_registry` are now also
used directly from `agena-provider` by provider core, with their former core
facade exports removed.
Provider wire-only stream values (`ChatStreamChunk`/choice/delta and
`ResponsesToolEvent`) now live in `agena-provider` as well. Core retains the
JSON event parsing, normalization, and `AppError` mapping, while OpenAI and
Bedrock transport code consumes the provider-owned values without a second
core definition.
`ModelModeRegistry` and `default_model_mode_registry` now follow the same
boundary: the complete provider model-thinking rule table lives in
`agena-provider`, core adapters call it through the provider-owned resolver,
and the former core `provider/model_modes.rs` module is deleted.
The pure configured-model merge and thinking-mode conversion helpers
(`apply_configured_modes`, `apply_configured_thinking_modes`, and the named
mode conversion functions) now live in `agena-provider` as well. Multi-adapter,
configuration, catalog, and app consumers call them directly; the former core
`provider/configured_models.rs` module and its facade exports are deleted.
The unused core `NamedProvider` wrapper and its registry facade export are also
deleted; provider registration now retains only the concrete registry and
runtime paths that have active callers.
The last unused adapter-agnostic runtime macro in core provider composition is
also deleted; the remaining runtime macros all have active adapter consumers.
`CompletionStreamEvent` has now crossed the same boundary: provider adapters,
registry/transport layers, session processing, and config stream typing import
the event directly from `agena-provider`; the core provider facade no longer
re-exports the stream event.
The dependency-free `ProviderModelRouteKey` routing key is now owned by
`agena-provider` and used by `MultiAdapterProvider`; the route payload itself
remains core-owned because it contains generic tool/harness configuration.
Accordingly, `ProviderModelRoute` is now crate-private rather than part of the
core provider public facade; only the internal runtime/config composition
paths consume that core-owned payload.
The initial `agena-storage` event-store contract now exists with no database
implementation dependency. `SeaEventStore` implements that contract directly,
and event publisher, session history/workspace stores, application composition,
and API event listing consume it; the former core event-store module,
compatibility re-export, and duplicate core `EventStoreError` wrapper are
deleted. Event bus/publisher errors now reference the storage-owned error
directly. The provider-independent
`PersistedPermissionRule` storage value is also owned by `agena-storage`, with
Runtime permission resolution, session persistence, application services, and
CLI callers migrated to it. The active repository work now targets only a
real remaining public upper-layer edge; Runtime-internal SQLite composition is
not a migration candidate. The storage-owned `MemoryType` value
now supplies memory frontmatter and API/CLI/plugin consumers, and the
storage-owned memory document values (`MemoryFrontmatter`, `MemoryRecord`,
`NewMemory`, and `MemoryError`) now sit beside it; the filesystem
`MemoryStore` adapter and the workspace-scoped `MemoryDir` path contract now
live in `agena-storage` as well. The old Core store module is deleted. The
`MemoryRepository` port is implemented by `MemoryStore`, and application
memory operations, the memory plugin's list/get/write/delete paths, and CLI
memory list/forget operations consume that port; filesystem-specific directory
permission reporting and edit-path resolution remain on the adapter.
The session history `MessageIdAllocator` port and its sequential test adapter
now also live in `agena-storage`; the run buffer and processor use that port
without retaining a core allocator definition. The shared in-memory
message/part allocator state used by `SessionStore` is storage-owned as well;
session persistence and orchestration remain core responsibilities.
The storage port family now also includes `WorkspaceRepository` for normalized
workspace identity lookup/creation. `agena-storage-sqlite` owns the concrete
`SeaWorkspaceRepository`; runtime composition and CLI workspace-scoped
permission writes select it directly, rather than calling core
`workspace_crud`; the former Core workspace CRUD module is deleted. Shared
SQLite table/index definitions are storage-owned as well.
The model-catalog cache now has a storage-owned `ModelCatalogRepository` port
and opaque `ModelCatalogCacheRecord` value. `agena-storage-sqlite` owns the
concrete `SeaModelCatalogRepository`, which uses the established table layout
without importing core entities or `AppError`; core snapshot composition delegates
to the Runtime-owned optional composition helper, which selects it. Cache
replacement and corrupt-cache repair are atomic because entry rows and
snapshot metadata change in one database transaction. Runtime
`ModelCatalogService` reads/writes through the storage port and owns catalog
refresh composition, including the optional-database startup decision exposed
by `compose_default_optional`. A focused adapter regression seeds an existing
cache,
replaces it through the public repository port, and verifies both the empty
entry set and updated snapshot metadata. The shared SQLite schema is fully
storage-owned: lifecycle/version markers, invariant triggers, concrete tables,
and indexes all live in `agena-storage-sqlite`; no Core schema facade remains.
`SessionStore` workspace identity lookup/ensure now consumes the same port, so
session history no longer reaches into workspace CRUD for identity resolution.
Permission-rule list, single-record, upsert, replace, revoke, and delete
application operations now consume a storage-owned `PermissionRuleRepository`
port through its SeaORM adapter. Session-transaction rule effects now use the
storage-owned SQLite transaction writer while Core retains only their
session/history orchestration.
Application permission resources now map directly from storage-owned
`PermissionRuleRecord` values; the application layer no longer reconstructs a
SeaORM permission model merely to build an API response.
SessionStore permission-rule resolution now consumes the same repository port
for non-transactional reads. The transaction-bound permission upsert SQL is
also storage-owned through
`agena-storage-sqlite::SeaPermissionRuleTransactionWriter`; `SessionStore`
retains only the session-plus-rule atomic transaction orchestration and event
metadata. The
former Core `db::crud::permission_rule` module is deleted: the writer owns only
the limited upsert required by the active transaction, while every ordinary
permission read/write path uses the storage adapter.
The process-local `SequenceAllocator` used for persisted event sequence
numbers is now storage-owned as well; event publisher and session-manager
composition consume `agena_storage::SequenceAllocator`, and the former core
event sequence module is deleted.
Application session resource statistics now consume a storage-owned
`SessionStatsRepository` for visible-message counts and child-session counts;
the Sea adapter owns the grouped event/lineage queries.
Session usage/cost detail reads now use a storage-owned `UsageRepository`:
SeaORM owns session filtering and assistant usage-row extraction, while core
retains only the provider/model cost aggregation and presentation logic.
The application `get_session` read path now consumes a separate
`SessionSummaryRepository` that exposes session metadata without core-owned
runtime JSON; ordinary list reads now use the same summary boundary, while
runtime-heavy session loading and branch-aware mutations remain core-owned.
Application session rename/delete now use the corresponding
`SessionMutationRepository` operations and the summary/stats ports for their
resource responses; session creation and branch-aware mutations remain on the
core transaction path.
`SessionStore::rename_session` now uses the same mutation repository, updating
an already-cached runtime session title in place or reloading the runtime only
when the cache does not contain the session.
The ordinary application `create_session` path now uses the same mutation
repository and summary/stats response mapping; branch-aware creation remains
on the core transaction API.
Application session existence/version/workspace checks now consume
`SessionSummaryRepository` as well, so the service layer no longer reads a
core `SessionRecord` directly for those guards.
`WorkspaceRepository` now also exposes path lookup by workspace id; the
application workspace-existence guard consumes that port instead of querying
the workspace entity directly.
Workspace list and get now consume storage-owned `WorkspaceRecord` values and
repository filtering/cursor queries; application retains only session-count
aggregation, filesystem access, and workspace mutation mapping at its boundary.
Ordinary workspace create, path update, and delete now also use
`WorkspaceRepository` mutations; path normalization and conflict policy remain
application concerns.
Workspace file-tree and file-download APIs now resolve their root path through
`WorkspaceRepository::path_by_id`; only canonicalization, containment checks,
and filesystem traversal remain in application code.
Workspace session-count aggregation now uses `SessionStatsRepository`, so
workspace resources no longer issue grouped session entity queries directly
from application services.
`ApplicationService` no longer retains or exposes a `DatabaseConnection`; the
database is used only by runtime composition to construct concrete repository
adapters, keeping application services contract-driven after construction.
Workspace path normalization errors now use application-owned error values as
well, so production application services no longer import SeaORM error types;
SeaORM remains only in runtime composition and adapter/test code.
`Application::from_composed_runtime_services` now consumes the complete
Runtime-owned `RuntimeApplicationServices` bundle. Its nested
`RuntimeApplicationRepositories` contains only the memory,
workspace/session/permission storage traits; Runtime constructs every SQLite
adapter before the handoff. The outer bundle supplies provider catalog,
event-publisher/query/stream, session, control, tool, plugin, configuration,
authentication, and status ports directly. Application no longer chooses an
`AgenaRuntime`, rebuilds an adapter, or receives event-store/event-bus
implementation state. This is the sole normal construction path; its old
`ApplicationRepositories` wrapper and consumer-side factory are deleted.
Permission-rule publication therefore crosses `RuntimeEventPublishService`,
and API/application event reads cross the Runtime query/stream ports rather
than a concrete publisher, event store, or `SessionManager`.

The terminal `Backend` itself now retains `Application` and an explicit
workspace path rather than an `AgenaRuntime` field. Its construction adapter
still receives the concrete runtime only long enough to create Runtime
application services; repository setup now takes the explicit workspace path.
The application manifest now keeps `sea-orm` test-only, making that dependency
direction explicit at Cargo metadata level as well as in source imports.

The embedded TUI composition now receives those services from
`RuntimeBootstrapResult` as well: it calls `bootstrap_application_services`,
passes `RuntimeApplicationServices` into `Backend`, retains the bootstrap
result for the UI lifetime, and explicitly shuts it down after terminal
restoration. Process tracing configuration, filter construction, and database
connection setup are now Runtime-owned, so terminal startup no longer reaches
through a compatibility tracing facade before composition.

Initial embedded-TUI locale, theme, color-scheme, and graphics choices now
come from `RuntimeConfigurationService` after bootstrap, not from an early
configuration-loader result. Runtime retains schema validation and composition
internally, but its tracing value and helpers are no longer exposed to terminal
startup. The post-composition reload seam
now consumes that same `RuntimeUiConfiguration` directly through
`Backend::ui_configuration` and the shared `tui_config_from_runtime` mapper;
the former Runtime → Core `UiConfig` → TUI round trip is deleted and guarded.
The schema-neutral dotted JSON-path grammar, lookup, and formatting values now
live in `agena-domain`. Runtime's settings service maps the Domain error into
its concrete settings error internally, while App, API, and the plugin
workbench call the Domain value directly for display-only traversal. The former
public Runtime `get_json_path`, parser, and formatter exports are deleted;
this removes the remaining presentation-to-Runtime utility edge rather than
leaving a convenience façade. The architecture guard covers the app root,
backend configuration helper, workbench schema utility, and REST module so a
consumer cannot restore either the Core or Runtime helper path.
Agent Studio now follows the same one-way projection rule. Runtime retains the
registry plus `RuntimeAgentStatus`/`RuntimeAgentProfile` implementation
projections, while `agena-application` owns distinct
`RuntimeAgentResource`, `RuntimeAgentProfileResource`, and selection-resource
values with the one-time conversion. `Backend` and App consume those
Application resources rather than naming Runtime profile/status/selection
types; the App still edits the projected data and Markdown persistence
serializes it only at its file-output adapter. This deletes a real App-facing
Runtime concrete edge instead of adding a type alias. The architecture checker
rejects restoring the direct Runtime imports alongside the older
`agena::agents::*` and Core permission configuration imports.
The terminal has also stopped importing the plugin-SDK-owned attachment values
through the Core message facade: composer staging, backend attachment mapping,
and presentation labels now use `agena_plugin_sdk::{AttachmentItem,
AttachmentKind, AttachmentSource}` directly. Core `PartContent` remains only
where the session/message lifecycle adapter genuinely requires it. The same
terminal-source guard rejects reintroducing any of the three attachment aliases
through `agena::message`.
The normal dependency tree may still contain SeaORM transitively through the
legacy core package during the monolith transition; the direct application
manifest and production source contain no SeaORM or concrete storage adapter
edge.
The architecture checker now enforces this as a normal-dependency invariant
while intentionally allowing application test fixtures to retain a dev-only
SeaORM dependency.
Application memory operations now receive an injected `MemoryRepository`; the
filesystem-backed `agena_storage::MemoryStore` is constructed by app/API/CLI
composition, so application memory services no longer instantiate or import a
concrete Core adapter per request. The memory plugin consumes the same storage
adapter through the repository contract.
The memory service API no longer accepts a runtime handle for these operations;
its public boundary is now entirely repository/request based.

**Runtime memory-plugin ownership update (2026-07-23, static only):** the
in-process `agena.memory` plugin configuration/hooks, prompt construction, and
project-instruction discovery live in `agena-runtime`; its persistent Tantivy
index now lives in the independent `agena-memory-index` leaf. The leaf owns
only the stable memory document/index API and storage-derived index location,
not plugin/provider/session/runtime state; Runtime consumes it directly with
no re-export. Core no longer exposes a `memory` module; static plugin
registration and built-in manifest construction call Runtime exports directly.
The plugin's schema metadata uses the public plugin-SDK support API instead of
Core's private tool-definition helper. This does not change the already-
completed `MemoryRepository`/`MemoryStore` storage boundary, and remains within
the one deferred final verification batch.

**Runtime web-plugin ownership update (2026-07-23, static only):** the
complete `agena.web` plugin—its fetch/crawl/search configuration and schema,
network-permission checks, host throttling, in-memory fetch cache, crawl-store
integration, and local-browser policy—now lives in `agena-runtime` alongside
the `agena-web` adapter. Core no longer exposes a `web` module. Runtime derives
its version-based web user-agent internally; the unused configurable
constructor is deleted, so no external registration path can reopen web-plugin
composition. This is another source move in the deferred unified verification
batch, not a new stop point.

**Domain agent-profile values update (2026-07-23, static only):**
`AgentSelectionConfig` (provider, adapter, model, and mode defaults) and
`AgentToolsConfig` (the exact tool allowlist) now live in `agena-domain`.
Core configuration, session subtask requests, and plugin-host mappings import
these values directly. The remaining Core `agents` module is only the
filesystem/runtime profile registry and normalization adapter; it no longer
defines the portable profile-selection values. This leaves agent-registry
lifecycle extraction for the Runtime/session composition batch and does not
introduce a Core re-export facade.
The raw VCS diff application operation now follows the same rule and derives
its workspace path from the injected application service context; only git
operations that inspect runtime snapshot/session state retain a runtime input.
All application git operations now derive their filesystem path from that
same service context; runtime is used only for optional snapshot/session
counts in status responses.
Workspace path-to-id resolution in application services now also uses
`WorkspaceRepository::lookup_id`, leaving core workspace CRUD behind the Sea
adapter except for tests and core transaction composition.
`SessionStore::find_subagent_by_task_id` now resolves the stable subagent
summary through `SessionSummaryRepository` before loading the full core
runtime session, keeping the runtime-heavy materialization at its explicit
core boundary.
`SessionStore::load_session` now uses `SessionSummaryRepository` for cached
session version/lifecycle validation; SeaORM runtime materialization is only
performed when the cache is missing or stale.
The remaining session work must therefore be tracked as one concrete-service
batch rather than reopened as another read-model extraction: introduce the
runtime/application-facing session execution and query capabilities, project
the core-only message/event/session state at that adapter, then move the
manager constructor and write/compaction/reply orchestration. A superficial
trait that still names core `Session`, `Message`, `DomainEvent`, or `AppError`
does not satisfy this boundary and must not be counted as a migrated port.
Projected message/part to session-id lookups now use a small
`ProjectionLookupRepository`; the history store no longer owns these direct
lookup helpers, while full projection/message materialization remains in the
core history boundary.
The message read boundary now follows the same rule: SQLite owns list, cursor
page, single-message, and single/batched part queries through
`MessageProjectionRepository`. Core synchronizes or repairs the projection,
then decodes storage records (including opaque metadata, provider state, usage,
and part content) into domain messages. The legacy Core entity-backed message
read helpers have been removed; Core entity access in this area is retained
only for projection writes and rebuilds.
The application session list path now also consumes `SessionSummaryRepository`
for ready/workspace/root/parent/search/cursor filtering; SeaORM query details
and core runtime materialization stay in the adapter.
The session tree read path now uses the same repository through a
`SessionTreeRecord` boundary, including grouped visible-message, child-count,
and last-message statistics; `SessionStore` only converts those storage-owned
records into its API summary type and no longer calls session CRUD/statistics
queries directly for tree rendering.
The core `SessionManager::list_session_summaries` path now uses the same
summary and stats ports as the application list path, including the
workspace, offset/limit, and subagent-visibility filters; session CRUD reads
are no longer needed for ordinary summary listing.
Workspace session-id enumeration now reuses the summary repository as well,
removing the last direct session-id CRUD read from `SessionStore`.
Full tool execution/orchestration and session orchestration remain in the
monolith, while the provider-independent tool contract slice has moved into
`agena-tool`; provider configuration values, storage repository ports, and
tool policy/input values now have independent split slices, while concrete
execution and runtime composition remain in core.
The first tool-contract slice is now present in `agena-tool`: the
provider-independent `ToolPermissionCheck`, `PreparedToolInvocation`,
`PreparedShellCommand`, and `ToolOutputTruncationPolicy` values are owned
there, while core executors and API/TUI composition callers consume the new
contracts. `ShellRequest` and `ShellOutput` are now tool-owned adapter values;
`ShellError` is tool-owned as well. The shell default timeout, platform command
vectors, bounded output formatting, and stable command-analysis/result values
including exit-code interpretation, shell tokenization, command segmentation,
initial-command extraction, and quote-aware redirection detection are tool-owned
pure behavior. Network-effect classification also now runs in `agena-tool`, so
the plugin router no longer reaches Core for that policy. The complete
mutating/read-only classifier also now runs in `agena-tool`; Core maps only its
result into filesystem effect/permission behavior, including the pure
mutation-reason projection. Process execution and
permission/error adaptation remain Core implementations.
The curl cookie/data/form local-file predicates, PowerShell web-cmdlet
`-InFile`/`-OutFile` analysis, and the complete conservative filesystem-touch
classifier are Tool-owned. Core's remaining filesystem adapter delegates to
that classifier only to produce its
existing permission error.
Patch-result values
(`AppliedFileChange`, `ApplyPatchExecution`, and `PatchOpKind`) now follow the
same boundary: `agena-tool` owns their serialization-stable definitions, while
core retains only the filesystem patch parser/executor.
The pure `BuiltinToolProfile` model-id policy is now tool-owned too; core
`BuiltinToolSet` retains only plugin-manifest enumeration and availability
mapping over core `RegisteredTool` values.
Snapshot backend policy values (`SnapshotBackend`,
`SnapshotBackendSupport`, and `SnapshotBackendCapabilities`) are now tool-owned
as well. Core snapshot code retains git/rift probing, lifecycle operations, and
the in-memory session registry, while CLI/application consumers receive the
policy values through the tool contract boundary.
`ToolAvailability` is now tool-owned as the presentation-neutral result of
availability evaluation; core `BuiltinToolSet` computes it from plugin
definitions and agent state without owning the result value.
The model-facing `CronJobSummary` result value is now tool-owned too; core
cron execution materializes the contract, while the larger payload enum stays
in core because it still contains message, attachment, and plugin-specific
variants.
Tool web-search payloads now reuse the domain-owned `WebSearchResult` value;
the former core-only `WebSearchHit` duplicate is deleted, keeping message and
tool result projections on one stable result shape.
Background-process shell, stream, status, event, and summary values now live
in `agena-domain`; tool monitor execution and shell adapters consume those
stable values through the message facade without retaining a second core
definition.
Filesystem access/effect and network-effect declarations now follow the same
rule: `agena-domain` owns the stable permission-relevant values, while
`ShellCommandInput` remains a core tool-schema wrapper because its validation
macro and path argument annotations are core-specific.
The read-tool rendering policy (`ReadMode`) is now owned by `agena-tool`; the
core `ReadToolInput` schema continues to provide validation and simply embeds
the contract value.
Delegated-task model selection overrides (`TaskModelSelection`) now follow the
same boundary in `agena-tool`; `TaskToolInput` remains core-owned only for its
tool-input validation and task orchestration integration.
The architecture checker now locks these tool policy values to their contract
crate and rejects reintroducing their former core definitions.
The `agena::message` compatibility re-exports for `ReadMode` and
`TaskModelSelection` have now been removed; core consumers import the values
directly from `agena-tool`.
The message facade for domain-owned filesystem/network effect values has also
been removed; permission, shell, router, and output-helper code now imports
`FilesystemEffect`/`NetworkEffect` directly from `agena-domain`.
The process shell/stream/status/event/summary values have likewise lost their
message facade; monitor, shell, payload, and host-client code imports them
directly from `agena-domain`.
Core tool-module facades for `BuiltinToolProfile`, `ToolAvailability`, and
`CronJobSummary` are also removed; builtin-tool and cron implementations now
import those values directly from `agena-tool`.
Patch-result facades (`AppliedFileChange`, `ApplyPatchExecution`, and
`PatchOpKind`) are removed from the core tool module as well; CLI and core
patch consumers now import the result contract directly from `agena-tool`.
Snapshot policy facades (`SnapshotBackend`, `SnapshotBackendSupport`, and
`SnapshotBackendCapabilities`) are removed too; application and CLI consumers
now import the policy values directly from `agena-tool` while core retains
backend probing and lifecycle implementation.
Model-catalog pricing values no longer pass through a core facade either;
catalog enrichment and registry code import `ModelPricing` and
`ModelPricingTier` directly from `agena-domain`.
`ExecutionStatus`, `PartKind`, and their transition error are now all imported
from `agena-domain`; the message aggregate and `MessagePart` structures retain
those values as fields without re-exporting them through the core facade. This
required the dedicated message/session import migration rather than a local
type alias.
The in-memory history `RunBuffer` has now completed its local slice and imports
`ExecutionStatus` directly from domain; the remaining facade consumers are
limited to the message/session aggregate paths that still need that broader
API migration.
The history store persistence adapter now follows the same rule: SeaORM state
columns remain `StoredExecutionStatus`, while domain transitions and mapping
logic use `agena_domain::ExecutionStatus` directly.
Provider wire-message projection has now completed its analogous slice; it
imports the domain status directly while retaining core message/operation
structures for the actual projection.
The history event payload module now does the same, separating event payload
composition from the domain-owned execution lifecycle value.
Session manager history/rewind logic now imports the same domain status
directly, so both persistence and control-flow history slices avoid reopening
the message facade.
The event checkpoint payload follows the same rule and now carries
`agena_domain::ExecutionStatus` directly alongside its core message-part
payload.
The larger session model, processor, and manager modules now import the
domain status directly as well; their remaining core imports are message
aggregates and execution orchestration rather than status-value ownership.
The private `message/part/common.rs` status/kind prelude has now been deleted;
part implementations import `ExecutionStatus` and `PartKind` from domain
directly, completing the internal value migration rather than leaving a hidden
shim.
The prompt-window projection and manager test harness have now joined that
set, allowing the obsolete `agena::message::ExecutionStatus` module facade to
be deleted entirely; `MessagePart` internals still use the domain value through
their private part module import.
The former `PartStateTransitionError` alias is deleted as well; `MessagePart`
now returns the domain error type directly.
The remaining core `pub use` scan is now explicit: status/kind values belong
to the message aggregate API, provider-native configuration names belong to
the configuration schema namespace, and the provider wire-value re-exports
are confined to a private runtime utility module. No remaining entry is an
accidental duplicate definition facade from the completed provider/tool
contract slices.
The provider module's domain-ID prelude (`ModelId`, `ProviderId`, `ModelRef`,
and related identifiers) is used extensively by adapter/registry child
modules through `super` and remains an internal runtime prelude; removing it
requires a separate provider-module import cleanup, not a contract move.
The provider wire-value utility imports (`ChatStreamChunk` and Responses tool
events) are crate-visible only; the former public-looking `pub use` in the
private utility module is now `pub(crate) use`, so no provider wire facade is
exported outside core.
The first configuration API facade slice is now complete: `ProviderNativeToolFreshness`
is no longer re-exported through `config::types` or `config`; overlays and
OpenAI request construction import it directly from `agena-provider`.
`ProviderNativeToolHarnessKind` now follows the same configuration boundary:
the harness config implementation and overlay import it directly from the
provider contract, with no config facade.
`ProviderNativeToolRoute` has now been removed from the config facade as well;
raw/overlay configuration, provider adapters, and application test setup use
the provider-owned route enum directly.
`ProviderNativeToolKind` is now removed from the config facade too; raw
configuration and all provider adapters import the provider-owned kind enum
directly.
`ProviderNativeToolHarnessRef` is now removed from the config facade as well;
the harness registry/configuration helper imports the provider-owned reference
value directly.
`ProviderNativeToolBinding` is likewise imported directly by the provider and
resolved configuration schemas, and the unused
`ProviderNativeToolHarnessBindings` facade export has been removed.
The provider-hosted tool option structs (`ProviderHostedToolConfigs` and its
hosted web-search, file-search, code-execution, image-generation, and URL
context values) are also no longer re-exported through the core config facade;
the raw loader imports the aggregate directly from `agena-provider`.
The aggregate `ProviderNativeToolsConfig` has now been removed from that
facade too; session/provider runtime code and the application preset helpers
use the provider contract directly.
The former `PluginConfig` alias is removed from the core config facade as
well; raw and resolved schema code imports `agena_plugin_host::PluginsConfig`
directly.
The provider-owned tool/configuration re-export block has now been removed from
`config::types` and `config`; remaining configuration names in that namespace
are core schema types or intentionally composed configuration wrappers. This
keeps the configuration API boundary explicit without pretending that the
schema layer itself is a provider contract.
The tool contract crate now has direct behavior tests for builtin-profile
inference, snapshot backend selection, and availability values, so these
policy boundaries are verified independently of the core executor.

Perform sub-phases in dependency order:

1. Provider contracts, then concrete adapter crates.
2. Storage contracts, then SQLite/SeaORM implementation.
3. Tool contracts, then built-in tools.
4. Session execution against those contracts.
5. Configuration syntax/loading/validation.

The application layer always consumes contracts; concrete adapters are selected
only by runtime composition.

Exit criteria for every sub-phase:

```text
Contract crate does not depend on implementation crate.
Application does not depend on a concrete adapter.
No Cargo dependency cycle exists.
The old monolith slice is deleted.
```

### Phase 7 — Build runtime composition and delete the monolith

**Current status: complete and verified.** Runtime owns the
concrete bootstrap, snapshot, provider/session/tool adapters, host callbacks,
and lifecycle; every process consumer uses the Runtime factory and the legacy
package/alias is deleted. The unified locked pipeline, feature matrix, E2E,
dependency analyzers, and static boundary scans pass.

**Historical migration record (superseded by the completed status above).**
The following paragraphs explain the staged pre-deletion order and retain old
paths only as audit evidence. They are not a current source queue: every
referenced `crates/agena` implementation and `agena-core` dependency was
deleted during the completed cutover.

The first runtime-composition slice is now extracted: the independent
`agena-runtime` crate owns the Tokio application runtime builder and its worker
stack policy. `apps/agena`, the studio server, and CLI test tools use that
crate directly; the old `agena::runtime::build_app_runtime` facade is deleted.
The remaining work is the concrete snapshot/configuration builders,
plugin-orchestration, host-client/event-bridge adapters, and provider/session/
tool composition still housed under `crates/agena`; background-task registry,
scheduler primitives, control-state aggregation, and generic lifecycle
orchestration have already moved.
The architecture checker now also rejects any reintroduction of the deleted
`agena::runtime::build_app_runtime` facade; process entrypoints must import the
builder from `agena-runtime` directly.
The locked Cargo metadata audit is the deletion gate for this phase: until the
`agena`, `agena-studio-server`, `agena-api-server`, `agena-application`,
`agena-cli`, and `agena-e2e` packages no longer depend on canonical
`agena-core`, the old crate must remain present and the phase cannot claim its
final exit criteria.
The concrete-composition boundary has since narrowed further: runtime now owns
control-state aggregation, connection/optional-service orchestration, plugin
shutdown/config dispatch, and generic model-catalog refresh cancellation and
cache-age staleness policy. Core still owns provider-specific live-catalog
composition and concrete provider/session/tool service creation. The concrete
model-catalog cache adapter now lives in `agena-storage-sqlite`, preserving
atomic replacement and corrupt-cache repair without leaking SeaORM into the
backend-neutral storage contract. Shared schema ownership and snapshot concrete service
composition remains core-bound, with its large runtime state expressed through
core-local aliases over the generic runtime snapshot/service bundle. The
snapshot facade now delegates concrete construction through dedicated
builders for model catalog, provider registry, plugin host, MCP, agents, LSP,
session, event bridge, and final service-bundle assembly; architecture checks
reject reintroducing those calls into the facade. The next move therefore
requires moving concrete builder ownership and return types; the typed builder
inputs are already runtime-owned contracts, so another facade-local input
wrapper would not advance the boundary.
The current source audit identifies the remaining composition consumers
precisely: `apps/agena/src/{main,lib}.rs`,
`crates/agena-cli/src/cli/cli_runtime.rs`, Studio, E2E, and the API-server
router tests still obtain the concrete `agena::runtime::AgenaRuntime`, but the
production CLI/Studio/E2E entrypoints now use the Runtime-owned
`RuntimeBootstrapRequest` rather than manually constructing Core
composition values. The lower-level `RuntimeCompositionConfig` and its
`LoadConfigRequest` are Runtime-owned as well; Core now only invokes Runtime's
bootstrap-request adapter and performs the legacy schema/bootstrap adaptation.
The resolved `RuntimeSessionBuildConfig` value likewise belongs to Runtime;
Core still adapts it into its concrete session manager and tool executor. The
manager's former Core-owned `SessionManagerConfig` aggregate is now the
Runtime-owned `RuntimeSessionManagerConfig` aggregate, including its
cache-policy projection, Domain-owned cache limits, and tool-concurrency
policy.
The Core compaction adapter now consumes Runtime-owned bounded-history,
per-message, output-token, and retry limits; only prompt text and
message/session traversal remain Core-bound.
Persisted usage-stat rows and their cross-session filtering, local-day
bucketization, cost accumulation, and reporting aggregation are Runtime-owned.
The Core cost reducer now remains only as the adapter from its legacy
message/session model for a single session's history summary.
Provider owns the shared recorded-versus-estimated cost contribution rule, so
both summaries use the same pricing fallback and unpriced-run accounting.
The fallback prompt-budget threshold is Runtime-owned too; Core session
orchestration consumes it directly rather than entering the prompt-window
implementation merely to calculate a policy value.
The shared conservative character-to-token approximation is Runtime-owned as
well, leaving Core prompt-window code only with message/request calculations.
Core also no longer publicly re-exports Runtime's context governor or tracing
reload handle; internal adapters consume those Runtime contracts directly.
The conservative provider/model pricing table and normalization policy are
Provider-owned; the reducer calls that policy rather than retaining a second
provider-behavior implementation.
The last Core `MessageUsage` compatibility module is deleted too: messages,
history, session state, and storage now name Provider `CompletionUsage`
directly.
`RuntimeApplicationServices` is now the runtime-owned
builder result for the application boundary: it carries the
provider/catalog/plugin/configuration/control/authentication/status ports plus
event and session ports, without carrying a concrete session-manager value.
Runtime's concrete builder assembles that result once;
`Application::from_composed_runtime_services` consumes it, and both API-server state
and the desktop backend have cut over to that construction path. `Application`
no longer stores or imports a runtime handle or `SessionManager`, and its
normal manifest edge to the legacy core is deleted. Event publication,
persisted/live event queries, execution commands, and transcript reads all
cross runtime ports. API model-catalog routes now consume Application list,
lookup, and refresh use cases rather than the Runtime catalog service. These
are concrete composition/session boundaries, not new contract values. The process bootstrap
already returns `RuntimeBootstrapResult`, never an `AgenaRuntime`; schema and
lifecycle composition are Runtime-private. The E2E direct edge is likewise
concrete rather than contractual:
`tools/agena-e2e/src/bin/dsv4f_tool_api_suite.rs` and
`tools/agena-e2e/src/bin/dsv4f_tool_api_probe.rs` now create a
`RuntimeBootstrapRequest`. The probe and suite retain
`RuntimeBootstrapResult` from the same public Runtime bootstrap path, while all
of their
provider/tool/session assertions consume Runtime service ports and projected
transcripts rather than a `SessionManager` or concrete session aggregate.
Those harness entrypoints already use the Runtime composition boundary; retain
that direct factory/service use rather than a compatibility wrapper.
The CLI read-side helpers and session list/tree/export commands now use
`SessionQueryService` for session listing, recent-session selection,
latest-event lookup, tree projection, and JSONL export. JSONL import now uses
`SessionExecutionCommandService` for mutation and the runtime-owned
`SessionPresentation` query for its result, so it no longer materializes a
Core `Session` merely to render the stable session-detail response. The
`cost` and `usage` commands likewise use the query port for their complete
read path. `resume`, `continue`, and `fork` now use the execution-command port
for mutation and `SessionPresentation` for output; `continue` and the agent
override branch of `resume` resolve their model through runtime control rather
than reading a Core session. Permission replies follow the same command/query
flow, and the old CLI `session_detail(Core Session)` helper is deleted. The
remaining CLI Core use is therefore limited to concrete runtime bootstrap and
local apply-patch adaptation rather than session command/query,
provider/tool, or transcript materialization.
Continue that migration through the same port family rather than introducing
CLI-local session wrappers.
Prompt/`exec` now follows that same command/query flow: `create_session` and
text submission are runtime commands, while blocked state, stable session
detail, and final assistant text come from `SessionPresentation` plus the
projected transcript. The old CLI helper that walked Core message aggregates
for the final assistant text is deleted.
The CLI MCP backend likewise retains its concrete tool executor only for tool
execution: session-resource listing now uses `SessionQueryService`, and MCP
tool-call audit events use `RuntimeEventPublishService`. It no longer stores a
concrete `SessionManager` merely to query session metadata or publish an event.
CLI plugin `status`, `inspect`, and `logs` follow the same rule: they now use
the Runtime-owned `PluginRuntimeService` rather than traversing a concrete
runtime snapshot to reach the plugin manager. The port includes both the
stable status list and exact-plugin status lookup, so CLI validation does not
need to recover that query from a Core host handle.
The CLI agent list now consumes `RuntimeStatusService` too. Its local output
projection preserves the previous `agents` command JSON fields (`defaults`,
`tools.allow`, scope, and source path) while keeping the Core agent registry
behind the Runtime status adapter.
CLI run-option construction now resolves default and explicit model targets
through `ProviderCatalog`; the provider contract owns the stable `ModelRef`
result while Core adapts its reload-aware registry once at the runtime
boundary.
The CLI `provider` list/models/capabilities commands now use that same
`ProviderCatalog` directly, including the provider-owned execution-options
projection for model capabilities and metadata; they no longer build a Core
`ProviderRegistry` from a CLI-local configuration resolution. This command
path now requests `RuntimeBootstrapResult` directly and retains only its
application services, rather than briefly holding a concrete `AgenaRuntime`.
CLI snapshot and git inspection now receive the workspace root and optional
snapshot registry from `RuntimeApplicationServices` and
`SessionExecutionControl`; they no longer walk through a concrete session
manager merely to reach its tool executor.
The remaining CLI session render paths now consume the same service bundle:
list/import/export, resume, cost, usage, permission replies, continue, debug,
prompt/`exec`, and fork obtain query, command, and execution-control ports
directly. `cli_render.rs` therefore no longer accesses `SessionManager`; the
MCP fallback tool-executor composition is now behind
`RuntimeToolExecutionService` as well: CLI receives stable tool descriptors
and invocation summaries, while Core selects the active session executor or a
bootstrap fallback executor inside the Runtime adapter.

**Runtime-bootstrap consumer audit (current):** the four remaining normal
Core consumers do not all carry the same debt, so they must not be migrated by
one manifest-only sweep:

The local browser OAuth callback cut is complete: Runtime owns listener bind,
HTTP response generation, callback URL parsing, state validation, provider
error-description/request-ID diagnostics, and HTML escaping. Application owns
the terminal/API callback completion use case, while CLI receives only
Application resources and commands; the former Core callback module is
deleted. Runtime callback unit tests and the locked architecture gate protect
this boundary. This removes a process capability from the CLI/Core seam, but
does not make the concrete snapshot builder eligible for deletion.

The embedded TUI has also begun consuming those projections at its concrete
backend seam: its terminal diagnostic summary now awaits
`RuntimeStatusService::runtime_status` through `Application`, preserving the
generation/load/provider/plugin display without calling `current_snapshot()`.
The settings-studio callback-URL paste flow likewise uses Runtime's OAuth URL
parser directly. The remaining TUI backend snapshot reads are still tracked
as separate configuration, plugin, session, and provider selection slices.

The TUI configuration-source panel now also consumes Runtime contracts: the
configuration projection carries global/project path and provenance details
(including applied layers and resolved defaults), while the settings service
reads the Runtime-selected global and workspace documents. It no longer
serializes or reads those values through `current_snapshot()`; write/reload
operations now use the same settings/control ports for global and workspace
paths. The remaining TUI configuration work is schema-specific provider/agent
editing behavior, not generic file selection, validation, or reload.

TUI plugin presentation and invocation no longer traverse the Core plugin
manager either. `PluginRuntimeService` now projects permission-tool catalog
rows, statusline/content/theme/command catalogs, registered-tool identity, and
plugin command invocation; the terminal backend has no `current_snapshot()`
or `plugin_manager()` use in its plugin module. Session-scoped permission and
tool execution remain a separate session-orchestration boundary.

Provider list and configured-provider presentation now likewise map the
provider-owned `ProviderCatalogEntry` contract into API resources. The former
TUI-local traversal of Core provider registry/configuration and its duplicate
native-tool summary projection are deleted. Provider draft editing and model
selection still depend on their separately tracked schema-specific slices.

The TUI workspace backend now has no `current_snapshot()` traversal. Its agent
directory/default/profile projection and configured-agent lookup use Runtime
status/configuration contracts; UI locale/theme/color/graphics preferences use
the Runtime configuration projection and are mapped once into the legacy TUI
record at the presentation seam. The backend still retains an `AgenaRuntime`
only for session-manager and concrete snapshot-tool operations, which remain
the next explicit orchestration boundary.

That session boundary has now begun to shrink: TUI usage statistics, active
execution cancellation, session permission and agent selection, snapshot
registry inspection, session-runtime availability, and session-scoped plugin
tool execution consume Runtime query/control/command/tool ports. Direct
manager use remains only for event/transcript materialization, interactive
permission orchestration. Snapshot enter/exit now use the explicit Runtime
`SessionSnapshotCommand` value/result rather than generic host payload
execution. Interactive session plugin commands now use
`SessionPluginCommandService`, carrying slash/raw/workspace invocation metadata
while Core retains the authorization decision. Steer input now
uses the stable text/attachment `SessionUserMessagePart` contract; Core alone
converts it into persisted `PartContent`. Each remaining manager use requires
a distinct stable contract rather than a wider manager escape hatch.

The Session permission-studio projection now reads selected permission from
`SessionQueryService::execution_context`, alongside its separately projected
effective permission; it no longer loads a concrete session runtime merely to
inspect that selection.

Session refresh now obtains its latest event watermark and bounded incremental
event count from `RuntimeEventQueryService`; it no longer materializes the
complete Core event list merely to decide whether to refresh. The separate
timeline projection still owns its remaining full-event migration.

Live session subscription now uses `RuntimeEventStreamService` and resolves
child-to-parent invalidation through `SessionQueryService::is_descendant_session`.
The legacy TUI reconstructs a `DomainEvent` only from Runtime's stable
meta/kind/payload projection at its presentation seam; it no longer accesses
the Core event bus or follows parent links through `SessionManager`.

The persisted TUI timeline now uses the same event-query port. Runtime returns
the newest bounded stable event projections; the presentation seam restores
the legacy `DomainEvent` only for existing timeline helpers and sorts by global
sequence. The terminal session backend no longer directly obtains a
`SessionManager`.

The synchronous Skills prompt action now uses the explicit
`SessionToolExecutionService::render_session_tool_output` port. The terminal
resolves only the stable plugin-tool descriptor and Core retains the concrete
executor needed for detailed prompt rendering. No terminal backend module now
obtains a `SessionManager`.

Provider Studio's global-file reads, adapter/default reads, patch/save writes,
and reloads now use Runtime configuration/settings/control ports as well; it
no longer reads a concrete snapshot merely to obtain the selected config path.
Saved-provider and unsaved-draft live adapter-model discovery now both use
`ProviderCatalog`: the Runtime adapter resolves the current saved provider
after reload, while a stable draft request union preserves HTTP, no-auth,
Cline, GitLab, credential/OAuth, and Bedrock SigV4 shapes without exposing
`ProviderAdapterModelsTarget` outside Core. Cline's model-list endpoint
override travels as an explicit stable request field. The terminal no longer
constructs a Core adapter-model target for this feature; remaining Provider
Studio work is the broader configuration-selection schema, not model
discovery or file editing.

The runtime-model-selection portion of Provider Studio is also now port-based:
model-catalog browsing, lookup, search/pagination projection, and user refresh
consume Application catalog use cases; Application performs the sole Runtime
catalog response-to-resource projection and invokes the Runtime-owned
background refresh task. Default model resolution plus thinking/speed/verbosity
metadata continue to consume `ProviderCatalog`. The terminal backend no longer
traverses the Core provider registry, model-catalog instance, or Runtime catalog
port directly. Remaining Provider selection snapshot reads are the separately
tracked configured-provider draft and local configured-model decoration
projections.

When Provider Studio opens an existing model for editing, its persisted model
overlay is read through `RuntimeConfigSettingsService` as well. The terminal
does not first pull a concrete config path from a snapshot and invoke a Core
file-settings helper for that operation.

Configured adapter IDs, enabled routes, and configured model IDs are now
projected as `ProviderConfiguredRouting` from `ProviderCatalog`. This removes
the terminal backend's direct resolved-config traversal for draft adapter
selection, route listing, and configured adapter-model presentation, while
keeping Core's authentication/configuration schema private.

Editing an existing Provider now follows the same rule. `ProviderCatalog`
projects the complete stable `ProviderConfiguredEditor` value—authentication
shape and secret source, OAuth/credential data, GitLab/Google/SAP endpoint
fields, Bedrock SigV4 values, defaults, and network timeouts—and the terminal
maps it once into its legacy draft record. The former
`ResolvedProviderConfig`-based draft constructor is deleted.

The configured local-model chooser is now `ProviderCatalog`-owned too. Its
Runtime adapter preserves the existing no-network behavior: it derives only
enabled configured routes, includes a valid configured default model, retains
native-compaction policy, and applies the current catalog decoration before
returning stable domain models to the terminal.

The no-network route/default projection is now concrete Runtime code rather
than a Core snapshot policy: `agena_runtime::configured_local_models` owns the
enabled-adapter filtering, route deduplication, default-adapter fallback, and
native-compaction projection, while
`configured_enabled_adapter_ids` supplies the stable adapter list needed by
the Core catalog-decoration adapter. Core retains only provider lookup and
catalog decoration; both configured-local and live model-listing paths use the
Runtime adapter-ID projection rather than traversing `ResolvedProviderConfig`
in Core. The last Core catalog-ID alias was removed as well: provider and
configuration model-listing paths call
`agena_provider::normalized_catalog_model_id` directly, so catalog identifier
normalization has one owner and no Core or Runtime compatibility facade.

| Consumer | Current direct Core responsibility | Required next extraction |
| --- | --- | --- |
| `apps/agena` | App Server startup retains its `RuntimeBootstrapResult` for the complete RPC-server lifetime, explicitly shuts it down after serving, and consumes application/session/provider ports; embedded TUI startup constructs its backend from `RuntimeApplicationServices`, reads locale and presentation preferences from the Runtime configuration projection, uses Runtime-owned tracing/filter/database helpers, and shuts the bootstrap result down after terminal restoration | move the concrete bootstrap implementation into `agena-runtime` |
| `agena-cli` | CLI launch intent now retains only raw `Vec<String>` `--set` expressions—never `ConfigOverrideArgument` or `LoadConfigRequest`—and its schema-neutral `OutputFormat` plus local browser OAuth callback listener are Runtime-owned rather than Core config/auth implementations. Its helpers no longer return or hold a concrete `AgenaRuntime`: they retain `RuntimeBootstrapResult`, short-lived service operations explicitly shut it down, while the long-lived MCP stdio backend retains the result until serving ends and then shuts it down; permission persistence obtains the initialized database from that result, `config resolve`/`validate` and diagnostics consume the Runtime-owned complete read-only configuration-resolution document, agent listing and credential login/logout/callback completion consume Application projections/use cases, session/event/MCP/provider command paths are port-based, and local `apply` invokes `RuntimeToolExecutionService` | move the temporary concrete bootstrap implementation from the terminal application adapter |
| `agena-studio-server` | a temporary Core bootstrap adapter is now its only direct Core use: raw override expressions and the database URL enter `RuntimeBootstrapRequest`, while `RuntimeBootstrapResult` supplies lifecycle shutdown and the one composition handoff; long-lived Studio state stores `Application` and consumes only its diagnostics/workspace-root use cases, never `RuntimeApplicationServices` or `RuntimeStatusService` | move the concrete bootstrap implementation into `agena-runtime` |
| `agena-e2e` | both DSV4F harnesses retain `RuntimeBootstrapResult`; the larger Tool API suite initializes sessions and runs model turns (including pending interactive replies) through Runtime execution/query ports, reads its provider Tool API declaration surface through `RuntimeToolExecutionService`, its streaming checkpoint assertion through `RuntimeEventStreamService`, and its `tasks.run` child completion/transcript plus all operation/history assertions through Runtime query/execution-control ports and projected transcript helpers | move the temporary Core bootstrap implementation into `agena-runtime`, then remove the E2E Core dependency |

The terminal App Server and embedded TUI requests now follow the same input
rule: they carry raw override expressions only, while their runtime uses
`RuntimeBootstrapRequest` rather than constructing lower-level composition
input. Runtime reconstructs the Runtime-owned `LoadConfigRequest` from the
bootstrap request before the early legacy-schema loader seam. Core's schema
adapter then produces the
Runtime-owned tracing value, filter, and database
connection behavior required before composition. Locale and presentation
preferences come from the Runtime configuration projection. CLI itself never
exposes that Core input type.
The App Server backend similarly no longer stores an `AgenaRuntime` or
obtains a `SessionManager`: JSON-RPC create/submit/reply/list/read/cancel
operations consume the Runtime application service bundle, and assistant text
is derived from the stable projected transcript. Its concrete bootstrap call
is now the remaining terminal App Server Core seam.
The TUI live-event path uses the same direction for message parts: Core
adapts a checkpointed `MessagePart` into a Runtime projected part, then the
Application mapping turns that projection into the public API resource. This
repairs the former missing mapper without making `agena-application` depend
on Core aggregates.

The shared prerequisite is now the **concrete bootstrap implementation**, not
a new CLI/Studio/E2E wrapper. The public `RuntimeBootstrapRequest` and
`RuntimeBootstrapResult` already provide the correct Runtime-owned input,
capability bundle, database handle, and lifecycle surface; all four consumers
already use those values. The remaining temporary adapter is exactly
the Runtime bootstrap closure → `AgenaRuntime::new`: it invokes Runtime's
bootstrap-request adapter, runs the legacy loader/schema resolution, builds
`AgenaRuntime`, installs the Core-backed plugin-host client, and supplies the
Runtime-owned result envelope. The public `from_bootstrap_request` helper is
retained only as a temporary direct Core composition seam for internal callers;
it is not used by process entrypoints. Move that composition in dependency order while retaining the
existing typed `DatabaseCompositionInputs`, `PluginCompositionInputs`,
`SessionCompositionInputs`, and `ToolCompositionInputs` boundaries. Only then
can the four normal manifest edges fall without recreating Core facade
constructors in each consumer.

The bootstrap envelope is now Runtime-composed as well: `compose_runtime_bootstrap`
normalizes the request, invokes the temporary concrete adapter through a typed
`RuntimeBootstrapComposition`, and constructs the stable
`RuntimeBootstrapResult`. Core supplies only the concrete `AgenaRuntime`
capability/lifecycle values; it no longer constructs the consumer-facing result
directly. This is an intermediate dependency-order cut, not permission to
declare the concrete Core snapshot builder moved; the next required step remains
moving that builder and its loader/session/plugin adapters into Runtime.

The bootstrap-request extraction itself is complete and must not be reopened.
`RuntimeBootstrapRequest` carries `workspace_root`, raw `--set` expressions,
database URL/connection, schema-initialization choice, and the tracing reload
handle; it intentionally contains no Core schema type. Runtime normalizes the
workspace root and parses the raw
expressions into `ConfigOverride` and materializes `LoadConfigRequest`; the
Runtime tracing preflight is carried through `RuntimeCompositionConfig`, so
bootstrap no longer performs a duplicate full schema load before snapshot
construction. The
temporary Core adapter only crosses into the legacy schema loader. CLI/Studio
retain raw expressions, E2E
uses the same request with an empty list, and consumers retain only
`RuntimeBootstrapResult`. The next move is to relocate/replace the concrete
loader/snapshot/plugin/session composition—not to add another consumer wrapper
or copy `ConfigOverride`/`RawConfig` into Runtime.

The schema-neutral environment seam has already moved: `ConfigEnvironment`
and `ProcessEnvironment` now live in `agena-runtime`; Core's legacy loader
consumes them through a crate-private import and no longer exposes a public
compatibility re-export. This is intentionally smaller than
moving the loader itself, but removes a Core-owned bootstrap input without
pulling `RawConfig` or override schema into Runtime.
The legacy provider-registry, adapter-model, and credential helpers are also
crate-private within Core now; no new consumer may obtain those concrete
configuration adapters through `agena::config::*`.
**Raw-loader extraction audit (2026-07-23):** the legacy `RawConfig` parser
and resolver are a single 1.7k-line closure, not a set of independent public
values. Its remaining Core-specific edges are limited to bundled static-entry
construction, a narrow permission-validation adapter, and settings-editor
layer adaptation. `ConfigError` and `parse_numeric` now live in
`agena-runtime/src/config_error.rs`; Core imports them only within its legacy
configuration module. Runtime also owns the schema-to-settings-service error
classification; Core supplies only its schema validator callback. The
Runtime file reader/parser now owns optional JSON document loading and the
associated read/parse errors; Core retains only unsupported-field checks and
schema deserialization. Runtime also owns the applied-layer provenance order
and descriptions, plus the retired `AGENA_MODE` rejection; the Core loader now
supplies only source-presence facts.
The permissive boolean parser for environment-backed configuration overrides and
the settings-error conversion helpers are Runtime-owned alongside
`parse_numeric`; the shared optional-string normalization helper used by
provider/environment resolution and the optional-value merge rule are
Runtime-owned there as well. The process-level `AppError` conversion is kept
at the Core boundary. The parser no longer constructs an
`Agent` merely to validate permission input. Extract or replace those adapters first,
then move the raw parser and loader together. Do not copy `RawConfig` into
Runtime or introduce a factory that delegates back to Core.
The companion default/global/project configuration path policy now lives in
Runtime as well. `default_workspace_root` and `project_config_path` are
Runtime-private loader/composition helpers; only the CLI-required
`default_config_path` remains public, so Core cannot retain another
bootstrap-policy sidecar.

**API-server runtime-composition cutover (complete):** replaced
`AppState::new(AgenaRuntime, DatabaseConnection)` with an
`AppState::from_application(Application)` constructor. The terminal app and
Studio composition roots must build `Application` from
`RuntimeApplicationServices` plus concrete storage repositories before they
construct the API router. Router contract tests may construct Core runtime only
as dev fixtures, then make the same application composition explicit. The API
state has no production `AgenaRuntime`/`SessionManager` import and no local
runtime-to-application repository factory. The file-settings REST adapter now
uses the runtime-owned `RuntimeConfigSettingsService`; request/response values
and effective-document path projection live in `agena-runtime`, while Core is
only the concrete adapter that validates and writes its schema-specific file.
The process metric counters/snapshot likewise live in `agena-runtime`, while
`agena-application` maps them into its transport-neutral metrics resource for
the API endpoint. The API-server package therefore has no normal Core
dependency or direct Runtime metrics call; only its contract-test fixture keeps
a dev-only Core dependency, and metadata now reports four normal direct Core
consumers.

Application and API-server error contracts no longer retain `AppError` as a
public transport value. Core adapter failures are collapsed to the
application-owned internal-error variant at the last concrete adapter call;
the API layer maps only application/transport error categories. This removes
one non-transcript Core type from presentation boundaries while preserving the
existing HTTP-500 behavior and diagnostic message.

The remaining message-lifecycle value move has one explicit storage
prerequisite: core `MessageMetadata` still derives SeaORM's JSON-query mapping
trait. It cannot be copied into `agena-domain` while leaving the same
persistence behavior attached to the core version—neither a duplicate model
nor an orphan trait implementation is valid. `MessageUsage` has already
moved: `CompletionUsage` is canonical and its SQLite wrapper owns the ORM
adapter. Move metadata JSON decode/encode into `agena-storage-sqlite` with the
remaining stable metadata contract, then delete the Core definition and its
SeaORM derive together.
That write-side move must use a storage-owned transaction-bound projection
writer: projection rebuild applies message rows, part rows, and its watermark
atomically. A repository that opens or retains an independent SQLite
connection would silently break that invariant; `DatabaseTransaction` itself
must remain hidden behind the storage transaction contract.
`agena-storage::MessageProjectionMessageWrite`,
`MessageProjectionPartWrite`, and their generic
`MessageProjectionTransactionWriter<Transaction>` now establish that contract
without leaking SeaORM. `agena-storage-sqlite` now implements that contract
as `SeaMessageProjectionTransactionWriter`: it accepts the caller's active
transaction, preserves immutable message and part/operation identities, and
has rollback regressions proving it does not open an independent connection.
`SessionHistoryStore` now receives that writer through composition; both
incremental projection synchronization and full history rebuild route user,
assistant, system-notice, tool-result, and checkpoint message/part upserts
through the writer using the same transaction that updates the projection
watermark. The previous Core SeaORM message/part upsert helpers are now
focused legacy regression-test implementations only. Terminalization and
watermark/clear SQL now likewise cross the writer: the SQLite adapter has a
direct regression proving terminal message/part state and its watermark commit
together. Aggregate decoding and persisted message metadata remain Core-owned
until their stable storage contract moves. Usage is no longer a Core wrapper:
`CompletionUsage` is canonical and SQLite owns its transparent SeaORM JSON
adapter. Checkpoint state and part-count updates plus tool-result/issued
message touches already round-trip through the writer. Delete the test helpers
with the remaining metadata adapter move rather than restoring them to the
production rebuild path.
The executable architecture gate now locks this boundary: the SQLite writer
must implement message/part upserts, terminalization, clear, and watermark;
`SessionHistoryStore` must compose it, while the former Core write helpers
remain test-only. This keeps the next full-read-model work from silently
reintroducing a production Core projection write path.
The corresponding read-side contract is now established in runtime as
`SessionProjectedMessage` and `SessionProjectedMessagePart`: it carries stable
routing/lifecycle fields, opaque persisted metadata/usage JSON, and typed
current part details. Core's query adapter serializes or projects its remaining
aggregates only at that boundary. The complete consumer cutover is now done:
`parts=summary`, `parts=full`, list/get-message, list-parts, and get-part all
use `SessionQueryService`, including the stable part-to-session ownership
query. Application dispatch has no `SessionManager` acquisition for transcript
reads. Its runtime-projection value performs assistant multi-round merging,
ordering, pagination, part counting, and API detail mapping directly; no Core
message aggregate is reconstructed. The architecture checker locks this rule.

**Full-detail extraction order (do not collapse these into raw JSON):**

1. Keep `SessionProjectedMessage` metadata/usage opaque until the SQLite JSON
   codec owns the persisted wrapper conversion; Application may decode only
   its API-owned `MessageMetadata` / `MessageUsage` resources at the wire
   boundary.
2. Project the simple `text`, `reasoning`, and `error` variants from their
   domain-owned values into API detail resources. These do not justify a Core
   message aggregate dependency.
3. Extract attachment payload/identity values and the typed interactive
   request payloads required by API detail. Preserve the plugin-SDK attachment
   bridge; do not clone attachment source enums into Application.
4. Extract operation detail as an explicit runtime/API contract: invocation,
   visible output, blocks, artifacts, attachments, structured result, and
   request/reply state remain typed. Core `OperationPart` /
   `ToolResultEnvelope` JSON is persistence input, not an API compatibility
   contract.
5. Switch assistant-round merge to stable projected message/part values, then
   delete Application's `agena::message::{Message, MessagePart, PartContent}`
   imports. Only after that can the Core activity projection entities and their
   test-only conversion helpers be removed.

Usage audit: the deleted Core `MessageUsage` alias and provider-owned
`CompletionUsage` had the same six accounting fields
(input/output/reasoning/cache-write/cache-read/cost). This migration is now
complete at the value/persistence boundary:
`CompletionUsage` is the canonical value and owns its arithmetic; the Core
compatibility module is deleted, with no duplicate struct or conversion
implementation. The SeaORM `FromJsonQueryResult` adapter is the
transparent `agena-storage-sqlite::PersistedCompletionUsage` wrapper, preserving
the existing JSON shape without adding ORM semantics to the provider value.
The architecture checker rejects restoring either a Core usage struct or a
Core ORM adapter.

The simple-detail sub-slice is now implemented: runtime
`SessionProjectedPartDetail` carries typed text/reasoning/error values, and
Application prefers those values when constructing full part detail. Attachment
has crossed the same boundary using the canonical plugin-host SDK
`AttachmentPart` already shared by runtime and Core; there is no new
Application-local attachment enum. Permission/user-input request and reply
values are already domain-owned, so runtime projects them as typed request
variants too. Operation now has the corresponding explicit runtime projection:
its invocation, model output, exhaustive blocks, artifacts, attachments, tool
output/result, structured payloads, errors, metadata, raw diagnostics, and
lifecycle cross as named values rather than Core `OperationPart` JSON. The
Application now performs assistant-round merging and API mapping directly over
its runtime projection value; it no longer reconstructs Core `Message`,
`MessagePart`, `PartContent`, or operation aggregates for transcript reads.
`Opaque` is retained only for absent/legacy persisted content and becomes an
absent API detail, never a generic JSON passthrough.

The typed-input work required before moving those builders is now explicit:

The provider/catalog, plugin/MCP/LSP, session/tool, and database builder input
shapes are now represented by runtime-owned generic contracts. The remaining
work is to move their concrete implementations without pulling
`ConfigResolution`, SeaORM, `ProviderRegistry`, or other legacy-core types into
`agena-runtime`; the final builder still needs an explicit schema-initializer
ownership rule and reload/shutdown choreography.

The first typed builder-input contract is now present:
`agena_runtime::ModelCatalogCompositionInputs` carries the provider map,
configuration path, plugin handle, and optional database handle into the core
catalog builder. The concrete SeaORM/provider construction remains in core for
now, but its input shape is runtime-owned and the architecture checker verifies
both the contract fields and the core consumer. This is the first step toward
moving the catalog builder without recreating a core-local aggregate input.
Plugin-host composition now uses the same boundary through
`agena_runtime::PluginCompositionInputs`, carrying plugin configuration,
workspace identity, previous-host/config reuse state, and the optional MCP
manager. `agena_runtime::compose_plugin_host` owns the concrete host build
configuration, previous-plugin transport reuse, and `PluginHost::new` call;
Core retains only static registration assembly (which binds Core tool
implementations) and active-host lifecycle installation. The architecture
guard prevents the Core construction adapter from returning.
Provider-list plugin dispatch has crossed the same host boundary:
`agena_runtime::dispatch_provider_list_patch` owns the no-plugin fast path and
plugin hook call. Runtime now also owns the remove-before-add patch application
ordering through `ProviderListPatchTarget`, plus the projection from concrete
provider IDs into host-facing descriptors; Core only implements that adapter for
its concrete provider registry. This is an incremental composition move, not
evidence that the concrete provider registry itself has left Core.
Optional LSP composition has crossed the boundary completely:
`agena_runtime::compose_lsp_services` owns plugin configuration parsing,
enablement, registry creation, and registration-task retention. Core supplies
only workspace and process identity at the snapshot call site; the obsolete
`LspCompositionInputs` staging contract and Core LSP builders are deleted.
Session composition now has the corresponding
`agena_runtime::SessionCompositionInputs` boundary for existing-manager reuse,
database, provider/plugin/agent/LSP handles, workspace identity, and session
configuration. `SessionManager` and `ToolExecutor` construction remain
core-bound because their behavior still depends on core permission and tool
contracts; the runtime-owned input shape is the staging boundary for that
future move.
The nested executor construction now uses
`agena_runtime::ToolCompositionInputs` for plugin/agent/LSP handles, workspace,
tool-presentation policy, and optional session-manager reuse. This removes the
last untyped positional argument list from the session builder's executor
factory while preserving the concrete tool implementation in core.
Database startup now crosses the corresponding
`agena_runtime::DatabaseCompositionInputs` boundary for connection reuse,
database URL, schema-initialization policy, and tracing configuration. Runtime
opens SeaORM connections, resolves the storage URL/path, prepares parent
directories, and initializes the schema; Core maps only the resulting Runtime
error into its application boundary.
Model-catalog cache freshness now uses the runtime-owned
`agena_runtime::ModelCatalogRuntimeConfig`; the SQLite infrastructure adapter,
not a core store, owns the concrete cache persistence.
The optional-database decision and missing-database diagnostic now also live on
`ModelCatalogService::compose_default_optional`; Core snapshot code delegates
that policy and keeps only provider-registry construction.

The concrete-implementation audit confirmed that the cache adapter belongs in
a dedicated infrastructure crate rather than backend-neutral `agena-storage`.
`agena-storage-sqlite` now owns it using provider-owned catalog definitions and
local SQL, without core entities or core error types. The shared schema
initializer is now storage-owned too; the remaining ownership work is limited
to the other SeaORM adapters and their concrete composition.

The follow-up catalog-value migration is now complete for
`CatalogModelRecord`: the stable API-facing record lives in `agena-provider`,
application/API DTOs import it directly, and the core model-catalog module no
longer re-exports it. The remaining separation is the core-only
`CatalogModelDefinition` source-ranking document from the persistence adapter;
that prerequisite still blocks relocating the SeaORM store. The provider crate
also owns a serde round-trip contract test for the record's wire shape.

The source-ranking audit confirms `CatalogDefinitionSourcePriority` is already
marked `serde(skip)` inside `CatalogModelDefinition`, so it is not part of the
persisted JSON/SQLite definition format. Its remaining uses are limited to
core merge/curation decisions (`sources`, `merge`, and `curate`); the next
definition migration can therefore replace this private field with a core-only
ranking sidecar without a user-data migration. The stable serialized
definition can then move behind a provider/storage contract before the SeaORM
store relocation.
The first physical step of that split is now complete: the ranking value lives
in the core-only `model_catalog/ranking.rs` module, while stable catalog data
remains in `types.rs`. Architecture checks require this sidecar location and
its non-serialized contract.
The persistence edge is now explicit as well: `CatalogModelDefinition` exposes
`from_persisted_json`/`to_persisted_json` helpers, and the SeaORM adapter uses
those helpers instead of performing definition serde inline. This keeps the
ranking sidecar out of the storage wire shape and gives the eventual
provider/storage contract a single replacement seam.

Although the input shapes are runtime-owned, the builders remain concrete core
adapters until their implementations can consume runtime-neutral provider,
storage, session, and tool ports. This distinction keeps the typed-input phase
honest and prevents a contract-only change from being mistaken for monolith
deletion.

The background-task contract portion has now been extracted as well:
`agena-runtime` owns `RuntimeBackgroundTask`, its kind/origin/status values,
start/outcome values, and control errors as stable service contracts. Its
registration spec, completion state, registry state, and generic registry
algorithm are Runtime-private composition machinery. Core no longer owns a
background-task registry implementation or compatibility re-export; it only
instantiates the private Runtime registry with `AppError` in concrete
composition. API-server REST handlers consume task snapshots through the
Runtime control service; the core runtime module retains only concrete reload
and service-composition details.
Reload cause/report values are now extracted too: `RuntimeReloadCause` and
`RuntimeReloadReport` live in `agena-runtime`; the generic polling, watch-path
diffing, and shutdown loop now live in `agena-runtime::run_reload_watch_loop`,
while core retains only the concrete reload callback and cause construction.
The generic snapshot holder and shutdown/task-control primitives are also now
owned by `agena-runtime`; core supplies only its concrete `RuntimeSnapshot`
facade and composition logic.
One-shot shutdown and plugin-notification tasks now use the runtime-owned
`spawn_detached` helper as well; core runtime code no longer calls Tokio's
`Handle::spawn` directly. Long-lived work remains covered by abortable guards
or task-control registration rather than this detached path.
The concrete runtime coordination aggregate is now runtime-owned through
`RuntimeControlState<S, E>`, so core no longer directly composes the snapshot
store, reload gate, background-task registry, and task-control fields in
`AgenaRuntimeInner`.
The same aggregate retains tracing filter reload state; generic connection
reuse/initialization (`connect_or_initialize`) and optional asynchronous
service gating (`build_optional`) are runtime-owned helpers used by builder
composition.
Snapshot identity metadata (`SnapshotMetadata`) is now runtime-owned as well;
the legacy snapshot combines it with its core-owned services and configuration.
The reload/session-GC scheduling policy and default intervals are now exposed
as `agena_runtime::RuntimeSchedulingPolicy` rather than hard-coded in core;
`agena_runtime::compose_scheduler` also owns the in-memory scheduler startup
around Core's concrete delivery sink. The remaining reload/session-GC adapter
and snapshot methods stay concrete until the builder move.
Reload watch inputs are likewise represented by the runtime-owned immutable
`WatchPathSet`; core only translates resolved configuration into that set.
The architecture checker now also asserts that core `runtime/reload.rs` calls
`agena_runtime::run_reload_watch_loop` and contains no direct watch-stamp or
path-diff implementation, preventing this extracted loop from regressing into
the legacy runtime module.
The snapshot's task-state wrapper is now runtime-owned as
`RuntimeTaskState`, leaving core with only the configuration-to-state mapping.
Reload serialization is likewise owned by `agena-runtime::ReloadGate`; core no
longer stores a bare Tokio mutex for snapshot reload choreography.
The registry registration input is now the private
`RuntimeBackgroundTaskSpec`; core retains only the concrete `AppError` binding
and callback wiring used to instantiate the Runtime-owned generic registry.
Terminal state transitions use the private `RuntimeBackgroundTaskCompletion`
value; only error creation remains core-bound. The registry's records,
ordering, cancellation-token and deduplication indexes now live in the private
`RuntimeBackgroundTaskState`; locking and async work dispatch are also
implemented by the private generic registry. The registry algorithm is
crate-private rather than a public `agena_runtime` capability, so legacy core
and upper layers cannot own task scheduling, deduplication, cancellation, or
history trimming logic.
The obsolete `crates/agena/src/runtime/background_tasks.rs` implementation
module has been deleted; `AgenaRuntimeInner` stores the runtime-owned generic
registry directly.
The shared periodic wait/shutdown primitive used by reload and session-GC loops
is also runtime-owned as `wait_for_tick_or_shutdown`.
The session-GC janitor now uses the runtime-owned generic `run_periodic`
runner; core supplies only the interval lookup and cache-prune callback.
The generic `AbortOnDrop` task guard used by the event bridge is runtime-owned;
the runtime-owned `spawn_abortable` helper now composes task spawning with that
guard. The event-subscription receive loop, lag handling, plugin broadcast, and
abort lifecycle now live in `agena-runtime::spawn_event_forwarder`; core retains
only the EventBus/EventKind subscription adapter and event-to-envelope mapping.
The runtime janitor uses the periodic helper and the reload loop uses the
runtime-owned watch-loop helper; both now let
`agena-runtime::TaskControl` retains their guards for the lifetime of the
runtime instead of leaving core with a guard collection or dropping detached
`JoinHandle`s immediately. Core now only owns the task-control handle and
shutdown signal; `TaskControl::shutdown` also drops the retained guards so
workers are aborted immediately even if another task still holds the control
handle. `TaskControl::spawn` now combines worker spawning and retention, so
core no longer manually composes `spawn_abortable` with guard storage.
The lower-level guard-retention method is crate-private; consumers receive
only the higher-level runtime lifecycle operation.
The runtime unit suite covers this contract, including the shutdown-time abort
invariant and detached one-shot notification completion; the current
`agena-runtime` suite has 25 passing tests. The
architecture checker currently has 39 passing tests, including the workspace
facade-regression and core-runtime lifecycle scans.
The process-global active plugin-host slot is now runtime-owned as well;
provider utilities and snapshot construction call `agena-runtime` directly,
and the legacy core `plugin_slot` module has been deleted.
The remaining core `store` shim has now been deleted too; `AgenaRuntimeInner`
uses `agena_runtime::SnapshotStore<RuntimeSnapshot>` and `TaskControl` directly.
Filesystem stamp capture and change diffing now live beside it in
`agena-runtime`; core reload code only translates detected paths into reload
tasks.
The immutable watch-path set now also owns generic insertion, sorting, and
deduplication in `agena-runtime`; snapshot configuration code supplies only
the resolved config/plugin candidates.
Snapshot-scoped service-handle retention is likewise Runtime-owned through the
crate-private generic `RuntimeServiceBundle`; core supplies concrete provider,
catalog, plugin, session, MCP, LSP, and agent handle types while Runtime
retains the event-bridge, LSP-registration, and plugin-shutdown lifecycle
guards. The
plugin-shutdown guard construction itself now lives in
`agena-runtime::plugin_shutdown_guard`; core only supplies the plugin host.
Configuration notification dispatch also uses
`agena-runtime::dispatch_config_if_nonempty`, leaving core with serialization
only.
LSP registration now uses `spawn_abortable` and is cancelled with its snapshot
instead of leaving a detached registration task behind.
The generic `RuntimeSnapshotState<Resolution, Services>` now owns snapshot
metadata, immutable resolution/service storage, and runtime task state; core's
`RuntimeSnapshot` is the concrete facade that supplies those two type
parameters and exposes domain-specific queries.
The facade audit also removed unused Core-only queries (`configured_agents`,
`model_catalog_snapshot`, and the provider-model convenience wrappers), leaving
only accessors with active composition or service-port consumers.
`agena-application` dispatch mapping for background-task kind/origin/status now
consumes the runtime contract directly instead of reaching through the core
runtime facade. The complete operational-status projection (runtime metadata,
provider and catalog state, background tasks, cache telemetry, scheduler,
MCP/LSP, skills, agents, and plugin UI) now crosses the typed
`RuntimeStatusService` port as well; Application maps that stable projection
to API DTOs without calling `AgenaRuntime::current_snapshot`. The remaining
Application core-runtime handle exists only for the legacy session-manager
adapter and is the next composition consumer to remove.
Read-only plugin-host inspection now follows the same rule through
`PluginRuntimeService`: API routes obtain plugin status, UI catalog, tool
registry events, inspection records, and logs without traversing a Core
snapshot. Plugin UI tool/command invocation remains a concrete session
permission and execution boundary and is intentionally not represented as a
read-only plugin port.
The REST health and readiness probes now obtain generation and load time from
the same status port rather than reading the Core snapshot directly.
Settings routes now use `RuntimeConfigurationService` for the configuration
path, existence flag, and a named effective-configuration JSON document; this
preserves the settings API's path-based extensibility while keeping Core's
resolved configuration type out of the API server.
Background-task listing/cancellation, scheduled runtime reload, and immediate
configuration reload now likewise use `RuntimeControlService`; Core's
`AppError` is converted once by its adapter instead of crossing API handlers.
Authenticated plugin callback JSON-RPC dispatch is now a `PluginRuntimeService`
operation too, so callback-token validation, stream-event ingestion, and host
callback dispatch no longer expose a plugin host or Core snapshot to the API
server.
Plugin UI action resolution, registered-tool lookup, command dispatch, and
session-scoped UI tool invocation now use the runtime plugin and session-tool
ports. The remaining plugin UI command permission calculation still requires
the concrete session executor and is now encapsulated by
`SessionPluginCommandService`; API routes no longer inspect agent policy,
construct permission checks, or invoke a session executor themselves.
API session state and command routes now likewise use the Application session
port bundle for submit/continue/compact/fork/reply/rewind/import plus tree and
export queries; they project responses from the stable session ID outcome.
The remaining direct manager access in that module is limited to event paging
and SSE subscription/ancestor invalidation, whose event-stream boundary has
not yet been extracted.
The `after_seq` event page and SSE initial backfill now already read the
session-scoped range directly through the injected `EventStore`; only the
live bus/lineage handling remains on the event-stream critical path. The
storage contract now also owns a descending `range_before` cursor query, so
the legacy cursor page preserves newest-first selection and final ascending
presentation without loading a full session event history through Core.
SSE now subscribes through Application's injected `EventBus` and asks
`SessionQueryService` for descendant lineage, eliminating the final concrete
`SessionManager` use from API session routes while preserving descendant
projection invalidation semantics.
API message list and single-message routes now dispatch through Application's
typed query boundary as well. Part-specific routes remain on the concrete
message-presentation path inside Application, but their API routes now use the
same typed query boundary (`ListMessageParts` / `GetMessagePart`) and no
longer obtain a concrete session manager.
Usage statistics now use `SessionQueryService::usage_stats`; with the session
commands, event stream, and message routes above, API server source no longer
obtains a concrete `SessionManager`. Provider credential inspection, API-key
updates, browser/device login, polling, deletion, refresh, reload, and
readback now cross Application authentication use cases; API and CLI
authentication routes neither inspect a runtime snapshot nor construct Core's
credential store or `AuthManager`, and neither receives the Runtime
authentication port.
Marketplace background work now uses `RuntimeControlService::start_background_task`,
so API server routes do not reach the concrete runtime even to register task
work. `Application` consequently extracts its typed runtime ports and the
optional session-manager adapter during construction rather than storing or
exposing `AgenaRuntime`; the remaining Application Core retention is only the
explicit message-materialization adapter.
The core runtime module no longer re-exports the background-task or reload
contract values; its remaining public surface is limited to concrete
`AgenaRuntime`, `RuntimeSnapshot`, and composition APIs.
The architecture checker now rejects `agena-runtime` dependencies on both
legacy package spellings (`agena-core` and `agena`), keeping the extracted
runtime layer from regressing into a core facade.
Its source-level tests also assert that the deleted core runtime shims
(`background_tasks.rs`, `store.rs`, and `plugin_slot.rs`) do not reappear.
The checker additionally scans `crates/` and `apps/` Rust sources to reject
reintroduction of migrated primitives through `agena::runtime::*` facade paths.
It also rejects direct detached-task and runtime-mutex primitives, including
both `tokio::spawn` and `Handle::spawn`, in `crates/agena/src/runtime`; those
operations must use the runtime-owned task control and lifecycle APIs.
The checker also scans every `agena-runtime/src` Rust source file for legacy
core package/path references, complementing the Cargo dependency-edge guard.
The tracing filter reload handle is also runtime-owned; core's runtime config
now consumes `agena_runtime::TracingFilterReloadHandle` without defining the
type alias itself.

Concrete-composition inventory after the runtime extraction:

The direct-consumer audit confirms that no workspace Rust source imports the
migrated background-task, reload, snapshot-store, task-control, watch-path, or
abort-guard values through `agena::runtime::*`; those consumers use
`agena-runtime` directly. This is now enforced by the architecture checker,
so the remaining core runtime references below are concrete composition rather
than compatibility-facade leakage.

- `crates/agena/src/runtime/snapshot/{mod,builders}.rs` still wires provider
  registries, the Runtime model-catalog service handle, plugin hosts, LSP/MCP
  services, session managers, and tool executors; these constructors are
  core-bound and are the
  next substantial move into `agena-runtime`. Generic watch-path set
  insertion/deduplication, plugin-derived reload-watch path composition, and
  cancellable registration batching have already moved; core now supplies
  only resolved configuration values and
  LSP-specific registration mappings. Snapshot handle retention and lifecycle
  guard storage now use the runtime-owned generic service bundle.
- The session-cache janitor's periodic scheduling and shutdown choreography
  now live in `agena-runtime::run_session_maintenance`; Core supplies only the
  concrete snapshot interval and `SessionManager::prune_cache` callback. The
  former `crates/agena/src/runtime/janitor.rs` adapter is deleted, keeping
  this maintenance loop free of Core session types while the remaining
  reload/event-host adapters continue their staged migration.
- `crates/agena/src/runtime/host_client/` still adapts the legacy
  `AgenaRuntime`/`RuntimeSnapshot` services to plugin-host callbacks; it can
  move only after the runtime composition API exposes those service handles.
- `crates/agena/src/event/bridge.rs` still depends on the core event bus and
  core `EventKind`; it is now a narrow subscription/projection adapter over
  `agena-runtime::spawn_event_forwarder`, while event filtering and the event
  payload projection remain core-bound. The runtime module no longer owns this
  concrete adapter.
- `crates/agena/src/runtime/builder.rs` still owns configuration loading,
  snapshot swapping, and the concrete `AgenaRuntime` handle. Database bootstrap
  composition and its lock/task/plugin primitives now come from `agena-runtime`.

1. Complete `agena-runtime` as the only concrete composition/lifecycle crate.
2. Move remaining runtime snapshot/configuration, plugin orchestration,
   host-client/event-bridge, and concrete builder composition out of old
   `crates/agena`; the registry/scheduler implementation is already extracted.
3. Delete `crates/agena` entirely.
4. Preserve `apps/agena` as the sole `agena` package after deletion and remove
   every remaining workspace alias/reference to `agena-core`.
5. Update root default members and all package references.
6. Delete deprecated aliases such as the old `tool_protocol` re-export.

Exit criteria:

```text
crates/agena no longer exists.
package agena denotes the final app only.
No old agena::* facade import remains.
```

### Phase 8 — Move and redesign the TUI

**Current status: source-disposition closure and unified functional
verification passed on the current worktree.** The terminal-runtime and presentation-value slices already
moved into `agena-tui`; the App retains only residual feature implementations
whose concrete effects prevent a complete opaque TUI slice. Do not move them
until one patch can delete their App owner without reintroducing API-server or
Runtime dependencies into the TUI crate.

**Fast completion rule for the remaining inventory:** select the next TUI
candidate by deletion, not by type size. A candidate is eligible only if this
batch can give TUI the authoritative display-only state, semantic action,
reducer/effect vocabulary, and read-only view while deleting the corresponding
App state/reducer/renderer owner. The App must retain Runtime/Application
queries and commands, persistence, filesystem/process work, schema
validation, concrete drafts, and localized process failures. Provider Studio,
plugin-schema editing, and run options are not eligible merely because they
contain focus or selection enums: their current values still combine concrete
configuration, validation, or persistence behavior. Re-audit them only when a
complete opaque projection can replace that whole App owner. This prevents a
cosmetic move from delaying the actual migration.

**User Input overlay reducer cutover (source train):**
`agena_tui::user_input::UserInputPresentation` now owns the opaque
display-question/request projection, all answer drafts, question/option/review
navigation, custom-text editor, answer synchronization, and the complete
question-flow reducer. It emits only `Close`, `Submit`, and `Cancel` intents.
TUI also owns the full read-only view: localized labels, markdown and terminal
sanitization, timeout display, choice/preview/custom-input panels, review
decision list, and Ratatui dialog composition. The App maps the Domain request
once into the opaque presentation and retains only request-aware validation,
`UserInputReply` construction, and Runtime submission/cancel effects. The App
`QuestionFlowState`, custom-editor/review state, duplicate reducers, two User
Input render methods, and all User Input rendering helpers are deleted. The
architecture checker rejects restoration of either the old reducer or renderer
owner, making this a complete State/Action/Effect/View vertical slice rather
than an adapter wrapper.

**Current candidate audit (source-train closure):** the session-lineage builder
is now a completed TUI presentation leaf: TUI owns scalar node tree ordering,
relation/depth/leaf classification, summary, and the navigation reducer; App
only projects `SessionResource`, localizes rows, stores active lineage state,
and opens the selected session. Permission Studio sections/actions still drive a concrete permission draft,
editor, and persistence lifecycle. Settings Studio likewise combines real
configuration edits/reload outcomes; Agent Studio combines source-file/profile
inspection, editor submission, and persistence; Permission Rule Studio combines
path browsing, draft validation, and rule persistence; and the remaining Plugin
Config focus/JSON-schema state participates in concrete validation/editing.
None is a standalone Phase 8 slice today. Keep their effects in App until a
complete opaque presentation state, reducer, view, and App effect adapter can
replace the old owner in one patch; this queue intentionally skips cosmetic
enum moves or a renderer-only split.

**Phase 8 source-disposition closure (2026-07-24):** the retention decisions
above, together with the completed TUI State/Action/Effect/View slices below,
close the current source queue. Transcript, Composer, Settings Studio, Agent
Studio, Permission Studio, Permission Rule Studio, Provider Studio, Plugin
Config, and run options remain App-owned only because each still combines a
concrete filesystem, process, configuration, draft-validation, editor, or
persistence effect with its presentation state. Do not create an alias,
forwarding reducer, renderer shell, or compatibility facade to make one appear
migrated. Reopen this queue only for a newly identified complete opaque slice
that removes the former App owner; otherwise proceed to the single functional
stabilization batch after all plan truth-maintenance edits are closed.

**Post-Timeline/Permission Prompt retention audit (2026-07-24):** the remaining
Transcript and Composer aggregates are not next candidates merely because
they contain presentation fields. `TranscriptState` still atomically combines
API/runtime message materialization, session loading/pagination, refresh and
execution coordination, rendered-node hit testing/selection normalization,
history loading, clipboard effects, and draft restoration. Composer likewise
combines its editor/draft persistence, staged filesystem attachments, external
editing, temporary-file cleanup, and session submission. TUI already owns the
independent viewport/pointer/scrollbar and item-selection reducers; do not
create a second presentation aggregate or move a renderer shell while those
concrete effects remain in the same App owner. Re-open either only when an
opaque display model plus complete reducer/view/effect adapter can delete the
whole corresponding App owner in one patch.

The model-catalog display payload is no longer a pending boundary. Its former
generic row action no longer carries `CatalogModelResource`; the TUI owns the
opaque key, label, subtitle, and localized detail-line projection together
with its query/page/loading/navigation reducer. App performs the one-way DTO
projection only when a page returns and retains the concrete query, refresh,
and search-editor effects. This deliberately leaves no API resource in TUI
presentation state and no App-local catalog detail renderer to migrate later.

**Completed full slice — session navigation:** lineage, rewind-message, and
child-session pickers now share `agena_tui::session_navigation`. TUI owns the
opaque row, search/selection state, keyboard/paste reducer, complete dialog
rendering, and semantic `Open`/`Rewind` activation effects. The App retains Runtime queries,
`SessionResource`/`MessageResource` projection, localized row text, and the
opaque-key map to concrete open-session or rewind-confirm effects. The former
generic `PickerValue::Session` and `PickerValue::Message` payload variants and
their `SearchPickerItem` branches are deleted, along with the corresponding
generic picker kinds for all three response paths. This is a unified vertical
slice rather than a move of only one value type.

**Completed full slice — generic selection picker:** the remaining provider,
agent, session-agent, and inspector picker paths now share
`agena_tui::selection_picker`. TUI owns opaque rows, filtering, selection,
always-visible and current-prefix presentation policy, keyboard/paste
reduction, complete dialog rendering, and close/opaque-key activation effects. The App retains Runtime
loading, localized row projection, Provider Studio and Agent Studio routing,
agent override persistence, and inspector semantics through a key-to-concrete
action map. The old App `PickerItem`, `PickerValue`, `PickerKind`, overlay
metadata, generic picker route, and its `SearchPickerItem` implementation are
deleted together; this leaves no generic concrete payload channel in the App
selection UI.

The architecture checker now records the portion already true in the extracted
crate: `agena-tui` owns the `KeyAction`/`KeyContext` action vocabulary, the
`ProtocolTransactionState` terminal transaction state machine, the
`TerminalLifecycle` state holder, and the transcript viewport reducer/view in
`crates/agena-tui/src/transcript.rs`. The transcript slice is now a concrete
State/Action/Effect/View boundary: `TranscriptViewport` reduces
`TranscriptAction` into `TranscriptEffect`, and `project_view` produces the
read-only `TranscriptView`. The remaining feature-level split is still open;
`apps/agena/src/app/app_types.rs` and the app transcript/session/composer paths
still own the broader runtime-bound state and effects.
The locale-selection and Fluent presentation slice is now TUI-owned too:
`agena-tui::i18n` contains the resolver, supported-locale policy, formatting
macro, and every `.ftl` resource. The final app imports those presentation
values directly and no longer has an `i18n` module or locale assets. This is a
pure presentation move; app-specific message/status wording remains in the
app until its feature slices move with their renderers.
The terminal capability slice now has the same ownership rule:
`agena_tui::terminal_capabilities` owns endpoint-support/source/path/provider
evidence, the canonical capability aggregate, terminal diagnostics, and the
projection to lifecycle modes. The final app imports these types directly and
only gathers process-specific helper availability plus composes diagnostics;
the architecture checker rejects restoring an App-local duplicate or facade.
It also scans every `agena-tui/src` Rust file for legacy core/runtime,
application, SeaORM, provider-registry, and session-manager references, so the
crate's dependency-light boundary cannot be bypassed through an unlisted source
import.

The first composer interaction slice is now TUI-owned as well:
`agena_tui::composer::ComposerItemSelection` reduces semantic
`ComposerItemAction` values into `ComposerItemEffect` values for selection,
navigation, open, and delete intent. The final app supplies the concrete effect
adapter that opens a staged path or removes an editor element, while attachment
preparation, temporary-file cleanup, draft persistence, external editing, and
session submission remain explicitly App-owned. The architecture checker
requires this reducer and rejects restoration of an App-local
`Option<usize>` selection field. This is intentionally only the pure
composer-item interaction slice; it does not misclassify the runtime-bound
draft aggregate as a TUI value.

The transcript pointer-value slice is TUI-owned too:
`agena_tui::transcript` defines the terminal-cell position, committed text
range, drag gesture, character-versus-semantic-unit policy, and the pure
per-line cell-range projection. It now also owns scrollbar geometry, host-edge
placement, and pointer-line-to-scroll conversion. The App retains rendered-node
hit testing, selection normalization against those nodes, actual transcript
cursor relocation/history loading, and clipboard effects. The architecture
checker requires the canonical TUI values and rejects the former App-local
definitions. This moves shared presentation mechanics without moving App-owned
transcript data or renderer behavior into TUI.

Main-surface focus now has the same owner:
`agena_tui::main_focus::Focus` defines the Sessions/Transcript/Composer
presentation identity, its stable host label, and the policy that cycles only
the visible Transcript/Composer panes while preserving legacy Sessions restore
behavior. The App imports this value directly for Route-specific input, help,
renderer, paste, and host-command adapters; its former local enum and generic
pane-cycle helper are deleted. The architecture checker rejects restoring an
App-local focus state or pane-navigation policy.

The session-list scope selector is TUI-owned as well:
`agena_tui::session_view::SessionViewMode` owns the All/Roots/Subtree
presentation vocabulary, cycle policy, and localized labels (including the
optional subtree root identity). The App directly imports that value in its
key/command, picker, and list adapters, then maps it to concrete Runtime query
ports; it no longer defines or extends a local session-view enum. This keeps
the presentation choice separate from the application-owned query lifecycle.

The main session-list vertical slice is now TUI-owned too:
`agena_tui::session_list` owns the API-independent row projection, newest-first
tree/root filtering, local query retention, selection, semantic navigation
actions, reducer effects, and a read-only list view. The App maps each
`SessionResource` to a display row and retains only the Runtime request's
in-flight scope/loading lifecycle plus the concrete open-session effect. The
former App `SessionListState`, API-record list, hierarchy/filter helpers, and
selection implementation are deleted. The architecture checker requires the
TUI State/Action/Effect/View owner and rejects restoration of those App-local
presentation algorithms; it does not move Runtime session listing or session
opening into TUI.

The session-model chooser presentation is TUI-owned too:
`agena_tui::model_chooser` defines the provider/adapter/model display identity,
picker row/current-marker behavior, picker state alias, open-purpose value,
construction policy, keyboard/paste reducer, selection-preserving query refresh,
and complete dialog rendering. The App still reads the concrete configured-model
catalog, projects it into those rows, converts the selected identity once into
its Domain `ModelRef`, and persists the Runtime override or provider default.
The prior App picker metadata, configuration/reducer/renderer owner, choice-row
type, and `SearchPickerItem` implementation are deleted; the checker rejects
restoring them or bypassing the TUI selection effect.

**Implemented in the active batch, pending final verification — full Choice
presentation slice:** `agena_tui::choice` now owns the Choice row/custom-input
representation, searchable/select-only presentation configuration, current-row
marker and query-row policy, single/multiple selection reduction, keyboard and
paste handling, and typed `Clear`/`Custom`/`Item` selection effects. The App
passes localized display strings and an opaque current value, retains only a
`ChoiceOverlayAction`, and maps the typed effect to concrete settings,
permission, provider, or session persistence/routing work. The former App
`ChoiceOverlayMeta`, `ChoiceCustomValue`, searchable-picker alias/config,
style enum, and choice input/current/query reducer helpers are deleted rather
than retained as a compatibility layer. TUI has no Runtime, Application,
provider, storage, or API resource dependency in this slice.

The session timeline picker is TUI-owned too: `agena_tui::timeline` contains
the display row, overlay presentation state, searchable-picker contract,
terminal sanitization, preview/dialog renderer, and pure linked-message
navigation effect. The App remains responsible for the concrete Runtime
timeline query and event-to-display projection, then adapts only `OpenMessage`
to its transcript navigation effect. The App-local timeline row, picker
metadata/alias, picker-item implementation, and Timeline renderer are deleted.

The prompt-history picker is TUI-owned too: `agena_tui::prompt_history` owns
the display row, picker alias, keyboard reducer, complete dialog rendering,
and explicit `UseText` effect.
The App retains history persistence and the concrete composer-draft replacement
effect, so navigation, filtering, and close actions cannot alter a live draft.

Slash-command and file-mention suggestion dialogs now follow that same full
presentation boundary: `agena_tui::slash_commands` and
`agena_tui::file_mentions` own their display rows, keyboard reduction, and
complete dialog rendering. The App retains the built-in/plugin command catalog
or workspace filesystem search, plus opaque-key resolution into the concrete
command or attachment effect. The former App-local composer picker renderers
are deleted; no command catalog object or filesystem path enters TUI state.
The former App row, picker alias, reducer, and local outcome enum are deleted.

Slash-command suggestion presentation is TUI-owned too:
`agena_tui::slash_commands` owns the display-only row, picker metadata/state,
search-picker rendering contract, and keyboard reducer that emits stable-key
`Fill`, `Accept`, and dismiss intents. The App continues to read the built-in
and plugin command catalogs, but keeps their concrete completion behavior in a
separate App-owned action map keyed by the TUI row key. It maps that action to
composer replacement and optional submission only at the effect boundary. The
former App row, picker metadata/state, value enum, and picker implementation
are deleted, so neither `CommandSpec` nor a plugin catalog object crosses into
the TUI presentation owner.

File-mention suggestion presentation is TUI-owned too:
`agena_tui::file_mentions` owns the display-only row, picker metadata/state,
rendering contract, input/navigation reducer, and stable-key refresh/select
intents. The App remains the owner of workspace filesystem search and
attachment staging. It converts each search result into a display row while
retaining the concrete `PathBuf` in an App-owned action map, then resolves the
selected key back to that path at the staging effect boundary. The prior
App-local row, metadata/state, picker implementation, and embedded path value
are deleted; the TUI never receives filesystem or backend search objects.

The permission-rule path browser is TUI-owned at the presentation boundary:
`agena_tui::path_browser` owns its mode, custom-input and complete dialog
rendering, display row, picker state, keyboard reducer, and refresh/select
effects. The App owns the
workspace directory read, relative-path resolution, permission-rule target,
and a stable-key-to-`PathBuf` action map; it commits the selected path only at
that effect boundary. The former App picker metadata, row, custom-value and
row-rendering implementations are deleted, so TUI never receives a filesystem
object while App no longer owns picker navigation policy.

The file-attachment picker is now another complete presentation/effect slice:
`agena_tui::file_attach` owns editable path input, API-independent display rows,
custom-path and complete dialog rendering, keyboard reduction, and
close/refresh/select effects.
The rows contain only stable App action keys; they never carry a `PathBuf`.
The App owns workspace indexing/search, maps keys back to concrete paths, and
stages attachments or reports validation errors. The former App-local picker
metadata, custom-path type, and `SearchPicker<PathBuf, ...>` presentation owner
and App-local overlay renderers are deleted. The architecture checker rejects
reintroducing a filesystem path into the TUI picker while requiring the App
effect adapter.

The interactive permission prompt is now TUI-owned at its presentation
boundary as well: `agena_tui::permission_prompt` owns Allow/Deny page and
details-return vocabulary, choice counts, selected cursor, opaque overview and
details content, localized title/footer and choice labels, terminal
sanitization, complete dialog composition, back/close behavior, and the
keyboard reducer. The App maps each concrete `PermissionRequest` once into
pure display lines, retains that request for reply validation/rule editing,
maps an activated page/index to Domain `PermissionReplyKind`/`PermissionScope`,
opens the rule editor, and submits the reply to Runtime. The former App page,
details-return, decision, selection, line-rendering helpers, choice-label
builder, and Permission Prompt renderer are deleted; no Domain permission
value is imported by the TUI presentation or view.

The settings workbench now has the same split:
`agena_tui::settings_studio` owns the display-only section identifiers and
group-label policy, source rows, generic item/section hierarchy, selection
state, query navigation, and keyboard reducer. The generic action payload
remains opaque to TUI. The App
builds localized rows from concrete configuration/provider/session data, then
adapts only `Refresh` and `Activate` effects to configuration persistence,
process launch, Runtime queries, and opening concrete child workbenches. The
former App-local settings section/item/source-row/focus types and its search
and key-navigation policy are deleted, so neither a configuration value nor a
second settings-navigation reducer crosses the presentation boundary.

Agent-profile workbench presentation now follows that ownership rule too:
`agena_tui::agent_studio` owns the display row, title/footer/list state, and
close/navigation/activate reducer while carrying the selected action as an
opaque generic. The App maps Application profile data into those rows and
retains the edit dialog, configuration or Markdown persistence, runtime reload,
permission-workbench routing, and source-file opening. Its
former Agent Studio row type and list-navigation path are deleted, so the
workbench no longer has a second App-owned selection reducer.

The model-catalog workbench is now a complete TUI query/pagination/detail
slice: `agena_tui::model_catalog` owns opaque display rows (stable key, label,
subtitle, and detail lines), query, page offset/limit, loading, selection, the
read-only selected-detail projection, and the close/search/refresh/page/
navigation reducer. The App performs one localized projection from
`CatalogModelResource` at the API response boundary, executes Runtime/
Application catalog requests and refreshes, retains only the concrete search
editor/effects, and feeds display rows back through `apply_page` or
`reject_page`. The generic `ModelCatalogPresentation<CatalogModelResource>`
payload, App detail renderer, former
`ListWorkbenchState<CatalogModelResource, _>` owner, and duplicated
key/pagination state are deleted.

The permission-rule workbench now has the same presentation boundary:
`agena_tui::permission_rule_studio` owns opaque rows, title/footer, selection,
and close/navigation/activate/browse/save/delete intent reduction. The App
retains the concrete rule draft, editor, choice and path-browser routing,
validation, suspended permission prompt, and create/replace/revoke effects.
The App-local row and `ListWorkbenchState<PermissionRuleStudioItem, _>` owner
are deleted, so rule navigation no longer has an App-local reducer.

The permission-studio two-pane navigation is TUI-owned too:
`agena_tui::permission_studio` defines the navigation tree, localized labels,
page and section vocabulary, pane focus cycle, focus identity, and selectable
tree movement/normalization policy. The App retains concrete permission
configuration sections, editors, validation, and persistence; it adapts the
TUI navigation state to those effects. The former App-local navigation enums,
tree builder, normalization/movement helpers, and pane-cycle helper are
deleted, preventing a second permission-studio navigation policy.

The session-search picker presentation is TUI-owned too:
`agena_tui::session_search` owns the API-independent row projection, picker
alias, remote-pagination state, complete dialog rendering, and the
reset/next-page/success/failure state transitions. The App maps `SessionResource` into display rows, executes the
resulting Runtime query effect, applies localized footer text, and owns the
query lifecycle/error notification. The old App row type, overlay metadata,
and `SearchPickerItem` implementation are deleted; the architecture checker
requires the TUI owner and its state-transition handoff. This is a complete
presentation/pagination slice, not a transfer of Runtime query or session
persistence behavior into TUI.

The command palette is now a separate complete TUI presentation/effect slice:
`agena_tui::command_palette` owns opaque command rows, searchable text,
selection persistence, keyboard/paste reduction, complete dialog rendering,
and the `Close`/opaque-key `Activate` effects. The App builds localized rows from built-in and plugin
catalogs, keeps only a key-to-concrete-command action map, and executes a
built-in command or prepares a plugin slash command at the effect boundary.
The old generic picker no longer has command/plugin-command value variants or
their `SearchPickerItem` branches. This removes the command palette's App
display/search owner without teaching TUI about `CommandSpec`, plugin catalog
objects, or command execution.

The session-navigation picker is now a separate complete TUI
presentation/effect slice as well: `agena_tui::session_navigation` owns opaque
session/message row keys, searchable display state, selection, keyboard/paste
reduction, complete dialog rendering, and distinct `Open`/`Rewind` effects. The App projects localized
lineage, child-session, and rewind-message rows, retains a key-to-concrete
session/rewind action map, executes Runtime queries, opens a real session, or
opens the concrete rewind confirmation. The old App generic `PickerItem`
payload variants for sessions/messages and their three generic picker routes
are gone; no `SessionResource` or `MessageResource` enters the TUI crate.

The formerly generic provider/agent/inspector picker is also now a complete
TUI presentation/effect slice: `agena_tui::selection_picker` owns opaque row
keys, display/search data, always-visible/create-row and current-agent-prefix
policy, selection, keyboard/paste handling, complete dialog rendering, and
`Close`/opaque-key `Activate` effects. The App maps concrete provider and agent records to those rows and
keeps the effect map for opening or creating studios, switching a persisted
session agent, and dismissing inspector rows. Its App-local generic picker
payload enum, metadata, route, search implementation, and concrete resource
carrier are deleted rather than retained as a façade.

The plugin workbench's top-level navigation is now TUI-owned as a focused
vertical slice: `agena_tui::plugin_workbench` owns the List/Detail mode,
ordered detail-tab vocabulary, and reducer for close, open-selected, return,
tab-cycle, and detail-scroll intent. The App retains the plugin manifest and
runtime-status projection, list filtering, selected-plugin lookup, schema-aware
configuration editing, validation, persistence, reload/restart, diagnostics,
and process effects. Its former local mode/tab enums and direct page/tab
mutations are deleted; it adapts only `OpenSelected` to reset the
configuration selection and `ScrollDetail` to the concrete rendered-panel
scroll position. This is intentionally not a claim that JSON schema editing is
pure presentation: that remaining configuration feature stays App-owned until
it has its own complete State/Action/Effect boundary.

The same TUI owner now defines the plugin-list transport and configuration
filter vocabulary, labels, cycling policy, and transport matching rule. The App
maps its concrete schema/runtime configuration status into the selected TUI
filter at the projection boundary; it no longer defines a duplicate filter enum
or cycle policy. This deliberately leaves the status calculation in App: it is
derived from concrete schema validation and running-plugin diagnostics rather
than being presentation-only data.

The plugin-list presentation now also owns its query editor, display-only item
projection, visible-index calculation, and stable selected plugin key. App
retains the full plugin records and resolves that selected key only when it
needs schema/configuration data or a concrete effect. The old App query,
visible-index vector, and selected-index state are deleted, so list filtering
and selection no longer have a second App reducer.

**Implemented in the active batch, pending final verification — Plugin Config
picker sub-slice:** `agena_tui::plugin_workbench` now owns the opaque action and
value-picker rows, single/multiple selection state, initial current selection,
keyboard navigation, page navigation, space-toggle semantics, and typed
close/opaque-key activation effects used by schema configuration overlays. The
App retains its schema paths, JSON values, branch records, and persistence
operations in key-to-concrete-action/value maps, then resolves the TUI key only
at the effect boundary. The prior App-local `SearchPicker` aliases, picker
metadata, and `SearchPickerItem` implementations are deleted. This is a
complete presentation/reducer extraction for the configuration overlays; JSON
schema evaluation, draft mutation, validation, and save/reload behavior remain
correctly App-owned concrete work.

The interactive user-input overlay now has one authoritative presentation and
view owner:
`agena_tui::user_input::UserInputPresentation` holds display-only questions,
request metadata, answer drafts, the custom editor, question/option/review
selection, review scroll, keyboard reduction, localized markdown/timeout
rendering, and all dialog composition. Its `UserInputEffect` is limited to
KeepOpen/Close/Submit/Cancel. The App maps the Domain request into this opaque
presentation, performs only request-aware answer validation and Domain reply
construction, and adapts submit/cancel to its Runtime effect. Its former local
answer map, `QuestionFlowState`, custom editor, review state, reducer helpers,
and renderer/helpers are deleted rather than shadowed by compatibility values.
The architecture checker locks the full TUI owner and rejects restoration of
the former App state machine or view implementation.

Transient notification presentation is TUI-owned as well:
`agena_tui::flash` owns flash level, message, lifetime, and expiry policy. The
App only chooses localized text and presentation level in response to concrete
effects; it no longer defines an App-local flash value or five-second expiry
rule. The architecture checker rejects restoring those local definitions.

The shared help-overlay presentation identity is TUI-owned too:
`agena_tui::help` defines the contextual-help/diagnostics kind, the canonical
dialog state alias, the `ContextHelpPreset` vocabulary, and the lossless
plain-text projection used for clipboard content. It also owns the localized
contextual-help document assembly from a selected context and key-card specs.
The App remains the adapter that selects a TUI preset from its Route and Overlay
state, builds runtime diagnostics, and performs the clipboard effect. The
Sessions, Transcript, Composer, ComposerItems, PromptHistory, Suggestion,
SingleLineEditor, MultiLineEditor, SearchPicker, ChoiceList, Timeline, Permission,
ReadOnlyDetails, UserInputQuestion, UserInputEditor, UserInputReview,
UserInputDecisionReview, Confirm, Usage, BasicList, Settings, ActionPane,
PermissionRule, Provider, ProviderModel, and ModelCatalog key-card catalogs are now
physically TUI-owned. The PluginList, PluginDetail, PluginConfig, PluginActions,
PluginSelection, PluginDrilldown, and PluginDiff cards are TUI-owned too: the App
now retains only Route/Overlay-to-preset selection, runtime diagnostics, and the
clipboard effect, with no key-card fallback catalog. The checker rejects restoring the
local enum, document builder, plain-text formatter, or a second static catalog.
Route/Overlay-to-preset selection remains intentionally App-owned because it
interprets route-local and runtime-interaction state; moving that reducer is a
separate full feature slice, not a reason to duplicate the catalog.

The usage-dashboard feature now has a complete display vertical slice:
`agena_tui::usage` owns its views, sortable metric, semantic controls, cycling,
stable labels, filter/selection/scroll state, reducer effects, display-only
totals/breakdown/session-link projection, row ordering, selected-session
resolution, and ratatui renderer. The App performs the one-way projection from
the Runtime `UsageStats` query result, retains period mutation, available-filter
discovery, async loading/error lifecycle, and adapts the selected session link
to the concrete open-session effect. The old App `UsageStats` rendering,
sorting, row-count, and selected-session owners are deleted. The architecture
checker rejects restoring those App-local owners or bypassing the TUI renderer;
the larger Phase 8 feature inventory remains active.

The session-status summary is now an additional complete display leaf:
`agena_tui::session_status` owns raw-token projection, the context-window
percentage policy, compact token labels, and agent/model/usage ordering. App
passes only scalar usage fields plus already-localized model/agent labels, then
uses the returned read-only segments in its broader status context. The old
App `TokenUsageStatus`, token formatter, percentage helper call, and
session-summary builder are deleted together; Runtime no longer exports a
terminal-only context-percentage helper. This is a single authoritative display
projection, not a moved scalar type.

The optional status-line scheduler is TUI-owned too:
`agena_tui::status_line::StatusLinePresentation` reduces a presentation tick
into either `None` or `Refresh { command }`, owns command enablement, cadence,
in-flight suppression, and the completed display text, and exposes only a
read-only text projection to the renderer. The App now adapts the refresh
effect by providing its process-derived session/focus interpolation and command
execution, then returns the output to `apply_refresh`; it no longer owns a
parallel `StatusLineState` or timer policy. The architecture checker guards the
state/effect/result handoff. This remains a focused presentation slice: process
execution and application-specific interpolation are intentionally App-owned.

1. Move TUI implementation into `crates/agena-tui` without behavior change.
2. Introduce State/Action/Effect/View boundaries.
3. Migrate feature slices in this order: terminal runtime, backend,
   transcript, composer, sessions, permissions, providers, plugins, settings,
   overlays/help.
4. For every feature, remove old giant-`App` mutation paths as the new slice
   becomes authoritative.
5. Keep transcript semantic model, layout, navigation, selection, clipboard,
   and renderer separate.

Exit criteria:

```text
No distributed giant-App architecture remains.
Views are read-only.
Effects own side effects.
TUI tests can use fake application services without real DB/providers.
```

## Separate performance follow-up

Build-graph isolation, cold-start baselines, rebuild attribution, and timing
policy have moved to
[`architecture-v2-phase9-performance-plan.md`](architecture-v2-phase9-performance-plan.md).
They are intentionally outside this completed functional architecture plan.

### Phase 10 — Final cleanup

**Current status: source-cleanup disposition and unified functional
verification passed on the current worktree.** The monolith-deletion gate is satisfied and the completed
cutover removed the package/alias and normal-consumer facades. Obsolete
artifacts have been removed with their owner deletions, and the source queue
has a final static audit of documentation, scripts, aliases, dependencies, and
migration-only allowances. Repair only a concrete cleanup regression found by
the later functional batch; do not reopen cleanup to add a bridge. Build-graph
measurement is deferred evidence, not a prerequisite for this cleanup.

The generic picker cleanup also removes both obsolete App module names:
`app_search_items.rs` and `app_choice_custom_value.rs` are deleted because
neither owns generic rows, search behavior, or custom Choice input any longer.
Localized Choice text is projected into the TUI-owned presentation at its
construction boundary; concrete persistence remains in the App effect adapter.
The architecture checker rejects restoring either obsolete module, so a later
generic picker implementation cannot hide behind a former owner.

The first proven-dead cleanup pass is complete in the active source batch:
unused catalog display fallback, session-context joining, Gemini-native-tool
clone, and duplicated provider response-to-stream helpers are deleted rather
than retained behind `allow(dead_code)`. The obsolete macro forwarding helper
is deleted too. Runtime provider module allowances that only masked these old
paths, plus the no-longer-needed App workbench allowance, are removed. The
remaining source scan contains no `allow(dead_code)` directive; final
compilation remains deferred until the shared stabilization phase.

The same batch now deletes the terminal's test-only concrete event-envelope
adapter instead of preserving a second live-transcript path: production and
tests both use `RuntimePresentationEvent`. Runtime-private history adapters
have been renamed away from the removed Core owner, and the now-unconsumed
`session::project_message_part` re-export has been deleted rather than kept as
an internal compatibility path; its history-local projection helper is private.
The unused generic serialized-event projection in Application is deleted too.
The architecture checker requires the test/production projection convergence
and the absence of the old re-export, so this cleanup cannot quietly
reintroduce a compatibility path.

Final-App transcript fixtures now construct the public API's typed
`StructuredObjectResource` / `StructuredFieldResource` /
`StructuredValueResource` tree directly for Tool API wrapper input, tool
arguments, and result payloads. They no longer deserialize an untagged JSON
object into that wire resource: doing so silently produces an empty structured
object and would test the generic `tools.call` fallback instead of the intended
`fs.*` compact-rendering behavior. This is fixture truth maintenance, not a
new App-to-Runtime test boundary or a compatibility conversion.

The architecture-check unit suite now follows the same current owners as its
executable rules: CLI permission helpers compose `Application` from Runtime's
already-composed services and name no SQLite repository; the snapshot's
provider-config accessor is crate-private; API auth uses its local
Application-backed response helper; and Studio retains `Application`, not a
Runtime service bundle. These are guard assertion repairs after owner deletion,
not restorations of the old concrete paths.

Active developer documentation now follows the same cleanup rule: README and
the configuration implementation index reference `agena-cli`, `agena-runtime`,
and `agena-storage-sqlite` directly rather than the deleted `crates/agena`
monolith. Historical plan discussion remains explicitly historical; the
architecture checker rejects the old source path in active developer docs.
The active-source cleanup audit was repeated after the newest TUI vertical
slices: production `crates`/`apps` contain no `allow(dead_code)` directive,
and README, configuration documentation, CI, and scripts contain no
`crates/agena/src/` reference. This is static source evidence only; the final
locked executable audit remains deferred with the unified pipeline.

Delete all transitional artifacts:

- compatibility parsers and aliases;
- old package paths and obsolete commands;
- temporary mappers and TODOs;
- dead features and dependencies;
- migration-only `allow(dead_code)` directives;
- generic helper dumping grounds created during migration.

Regenerate workspace documentation, API documentation, development commands,
architecture checks, and release/package scripts.

## Test strategy

### Domain

Use unit, property, serialization, and invariant tests. Domain tests are
deterministic and never start a server, terminal, database, or provider.

### Application

Use fake provider, storage, tool, credential, and event-bus ports. Application
tests validate use cases without HTTP, a terminal, SQLite, or real providers.

### Implementations

Every provider/storage/tool implementation runs a shared contract suite. This
verifies behavior such as pagination, ordering, transactions, cancellation,
permissions, streaming, and error mapping across implementations.

### CLI

Use help, exit-code, stdout/stderr, and JSON snapshot tests. Test launch-mode
selection without starting a terminal.

### TUI

Test reducers, transcript semantics/layout/navigation/selection, plain-text
clipboard extraction, render snapshots, terminal protocol handling, terminal
restoration, and backend-result-to-action transitions.

### API and client

Use versioned JSON fixtures, mapper tests, HTTP/WS/SSE/JSON-RPC integration
tests, and client/server contract tests.

### E2E

Keep real-provider, MCP, plugin, nested-permission, and process tests in the
opt-in `agena-e2e` package. These are not part of the ordinary product build.

## Separate build-performance plan

Timing budgets, target-graph acceptance criteria, cold-sample protocol, and
rebuild attribution are maintained in
[`architecture-v2-phase9-performance-plan.md`](architecture-v2-phase9-performance-plan.md).

Local validation also has a storage-safety invariant. The workspace dev and
test profiles retain incremental compilation for a fast edit loop while
disabling debug information, and large local Cargo gates run through
`scripts/cargo-bounded.sh`. The runner uses the repository `target` directory,
temporarily forces `CARGO_INCREMENTAL=0` for broad workspace/feature gates, and
terminates Cargo when the directory exceeds `AGENA_MAX_TARGET_GIB` (40 GiB by
default). Normal targeted Cargo commands continue to benefit from incremental
compilation. The architecture checker prevents these safeguards from being
silently removed. This limit is operational safety, not a substitute for any
build-performance or final verification gate.

## CI requirements

The final CI must include:

```bash
cargo fmt --all --check
cargo run -p architecture-check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo machete
cargo deny check
```

Layer-specific jobs may run in parallel, but the full workspace and
architecture gate remain mandatory.

## Commit discipline

Use atomic, bisectable commits. A typical sequence is:

```text
test(architecture): capture CLI and protocol baselines
refactor(app): remove the agena-tui compatibility binary
refactor(cli): extract command parsing from the runtime core
refactor(application): move local services out of the API server
refactor(api): decouple wire contracts from runtime models
refactor(domain): extract stable domain values and events
refactor(provider): separate provider ports and adapters
refactor(storage): separate repositories from SQLite
refactor(session): isolate session orchestration
refactor(runtime): make runtime the concrete composition layer
refactor(tui): move terminal UI behind application services
refactor(tui): adopt action/effect state transitions
refactor(workspace): delete the legacy agena facade
docs(architecture): publish the final dependency contract
```

Do not combine a massive mechanical move with behavior changes. Verify moves
with Git rename detection, then make semantic changes in a following commit.

## Completion checklist

- [x] The sole terminal product binary is `agena`.
- [x] No `agena-tui` binary or duplicate TUI parser remains.
- [x] `apps/agena-cli` no longer exists; final app is `apps/agena`.
- [x] The old `crates/agena` monolith no longer exists; locked metadata, the
  architecture gate, and the final unified pipeline verify its deletion.
- [x] No compatibility facade or old `agena::*` umbrella re-export remains;
  the legacy core root has no public re-export and the architecture checker
  prevents one from returning.
- [x] CLI, TUI, and API Server share application services; normal production
  consumers no longer import `AgenaRuntime`, `RuntimeSnapshot`,
  `SessionManager`, `ProviderRegistry`, `ToolExecutor`, a database connection,
  or a Sea repository. Final-App transcript fixtures now construct public API
  `MessagePartResource`/`OperationPartResource` values directly for text,
  reasoning, activity grouping, tool summaries, attachments, interactive
  requests, and export rendering. The old Runtime message/session projection
  construction path is deleted from the App, and `agena-runtime` now keeps its
  message/session implementation modules plus `project_message_part`
  crate-private. Its `db` implementation tree is private too: the remaining
  Application list fixture seeds storage through `SessionMutationRepository`
  and `EventStore`, not Runtime CRUD/entity paths. Do not add a public test
  facade or re-export to restore either old construction path. CLI permission
  list/create/replace/revoke now constructs storage adapters only at its
  composition seam and invokes the same transport-neutral application-service
  command/query operations as API Server, preserving the CLI audit actor.
  CLI snapshot and Git status now likewise consume Application's canonical
  snapshot/Git projections rather than traversing Runtime's snapshot registry
  or duplicating git-preflight policy. REST Git/snapshot handlers now invoke
  the same `Application` use cases rather than extracting a Runtime control
  port inside the transport adapter. The final App's plugin and inspector
  adapter likewise consumes those application use cases for snapshot rows,
  commit, and pull-request operations; session-scoped snapshot
  enter/exit remains explicitly a tool-execution effect. Final-App run
  cancellation now invokes the same Application `CancelRun` command as API
  consumers rather than reaching into `SessionExecutionControl`. Other direct
  Runtime capability consumers use their explicitly stable ports; they are not
  a pending return to concrete Runtime ownership.
- [x] TUI does not depend on API Server, SQLite, or concrete adapters.
- [x] API wire contracts do not depend on legacy runtime/application, and the
  architecture checker locks the canonical `agena-core` boundary.
- [x] Client does not depend on legacy runtime/domain, and the architecture
  checker locks the canonical `agena-core` boundary.
- [x] Domain has no I/O/UI/CLI/transport/SDK dependency.
- [x] Provider contract extraction is complete for catalog/discovery, normalized
  completion values, non-streaming response, tool declarations, provider-native
  configuration values, provider wire stream values, capability/mode/model
  metadata values, and the complete auth value family. Remaining provider work
  is model-catalog fetch/ranking/persistence composition and belongs to the
  runtime/monolith-deletion phase, not contract extraction.
- [x] Runtime is the only concrete composition layer: `agena-runtime` now
  owns the independent runtime primitives, contracts, registry algorithms,
  reload/watch helpers, snapshot state/service bundles, lifecycle guards,
  control-state aggregation, connection/optional-service orchestration, and
  plugin lifecycle helpers. It also owns the complete background-process
  monitor registry (Tokio child lifecycle, line buffering, reads, stopping,
  and monitor contracts); Core consumes those values through crate-private
  imports rather than re-exporting them. Runtime also owns the generic
  session-cache state machine (TTL,
  LRU order, byte/session limits, and cache statistics); core supplies only
  the `Session` cache-entry adapter and domain-specific title update. Core
  also consumes the runtime-owned session-to-snapshot registry; its active
  snapshot projection, managed-directory scan/pruning, Git/Rift capability
  probe, and concrete Git/Rift snapshot operations are runtime-owned.
  Application services and CLI call the query operations directly; core retains
  only the snapshot tool's permission/request/response adapter, alongside the
  shell-tool request/response adapter.
  The runtime-neutral `agena_tool::ToolExecutionSummary` is now the default
  result shape for CLI MCP calls, skill prompt queries, and generic CLI tool
  rendering; detailed core execution values remain only where a caller needs
  core-only attachments or apply-patch metadata. This is the execution-result
  boundary required before moving concrete executor composition.
  Runtime also owns the Unicode-safe textual output truncation algorithm; core
  applies that policy only to its typed payload fields.
  Runtime now owns schema-neutral configuration errors, environment numeric and
  boolean parsing, settings-error conversion in both directions, and bundled
  plugin merge precedence; Core supplies only concrete schema validation and
  static plugin entries.
  The Runtime session-maintenance loop owns periodic cache cleanup and
  shutdown handling; Core supplies only its concrete session callback.
  Runtime now also owns the core-free session lifecycle request values for
  create, fork, and rewind plus agent-switch outcomes. `SessionRunOptions`
  is runtime-owned now that completion-request construction accepts projected
  provider messages and Tool API definitions; core retains only that projection
  adapter.
  The runtime-owned execution request and generic execution-reply request now
  carry those options across session orchestration; core retains only wrappers
  that contain core message parts or permission reply values.
  The user-message request container is generic and runtime-owned as well;
  core supplies only its `PartContent` specialization.
  The permission-reply request is runtime-owned because its reply value is a
  domain contract; core retains only the permission-resolution implementation.
  The pure system-prompt merge policy used before completion projection is also
  runtime-owned, including its duplicate-prefix protection.
  Concrete snapshot/provider/model-catalog/session/tool service construction,
  configuration loading, host callbacks, event adapters, and process lifecycle
  now live in Runtime. The old Core implementations were removed rather than
  re-exported, and architecture checks reject restoration of the package or
  path alias. The Core-cutover pipeline passed; another complete pipeline run
  remains pending after the later open-phase source batches finish.
- [x] TUI uses State/Action/Effect/View and transcript vertical slices. The
  first transcript state slice is now concrete: `agena-tui::transcript`
  owns the transport-neutral `TranscriptViewport`, including reset, explicit
  scroll, and follow-tail transitions, plus the `TranscriptAction` to
  `TranscriptEffect` reducer; the app transcript model reuses that type and
  reducer instead of defining the viewport locally. Remaining transcript state,
  effects, and rendering deliberately remain App-owned because they depend on
  concrete application effects; the first View
  projection is now the TUI-owned `TranscriptView` value produced by
  `project_view`, which carries the visible range and follow-tail state; the
  Effect also owns the history-edge request decision consumed by app loading.
  The composer-item selection strip now has the same State/Action/Effect split:
  TUI owns selection/navigation and returns open/delete intent, while the App
  remains the concrete attachment/editor lifecycle effect adapter. Composer
  draft persistence and message submission remain deliberate runtime-bound App
  effects.
  TUI also owns the transcript pointer coordinate/range/gesture values,
  character-versus-semantic selection policy, and scrollbar geometry / pointer
  conversion; App retains node-aware selection normalization, cursor relocation
  and history-loading effects, and clipboard effects.
  The main session-list slice is now a full TUI State/Action/Effect/View
  boundary: its compact display rows, hierarchy/query projection, selection,
  navigation actions, reload/open intents, and read-only view live in
  `agena_tui::session_list`; the App retains only concrete Runtime loading and
  session-opening effects.
  The session-model chooser similarly owns its provider/adapter/model display
  identity, selected-row effect, current marker, picker construction/reduction,
  query refresh, and dialog view in TUI; the App only maps Runtime catalog
  records in and selected identities back to the Domain `ModelRef` required by
  its persistence effects.
  TUI owns transient flash notification state and expiry policy; App supplies
  only localized effect text and level.
- [x] Architecture metadata check is enforced in CI, including repository-wide
  rejection of `include!`, `include_str!`, and `include_bytes!` source
  composition; its API/client rules use canonical `agena-core` to enforce the
  legacy-core boundary, and provider/tool/storage contract crates now have
  explicit forbidden edges for core, database, runtime, HTTP, CLI, UI, and
  cross-contract dependencies that would bypass the domain layer.
  The checker test suite now explicitly asserts every provider/tool/storage
  cross-contract prohibition and scans workspace Rust sources for migrated
  runtime primitives re-entering through core facades, so the domain-only
  contract graph and runtime migration boundaries are regression tested rather
  than represented only by the production rule table.

The model-catalog runtime-policy sub-phase is now separately evidenced:
`run_cancellable_refresh` owns cancellation/shutdown/reload ordering and
`is_stale` owns cache-age evaluation. Keep provider-composition work open until
catalog-specific fetch/ranking/persistence composition leaves core; this does
not reopen the provider-value extraction checklist already marked complete.

The provider-contract portion of the checklist is now effectively complete for
the migrated catalog/discovery, completion, auth, capability, model-mode,
configured-model, native-tool, and wire-stream values. Keep the marker until
the final monolith deletion pass updates the historical checklist wording;
there is no remaining provider-value extraction implied by the current phase.

The legacy core provider module no longer publicly re-exports concrete provider
adapters or `ProviderRegistry`; those names remain crate-private composition
details while the provider contract and runtime-facing ports stay public. The
architecture checker locks this boundary so downstream application/API crates
cannot grow a new dependency on the old umbrella's concrete implementations.
The same module now keeps its domain model-ID/model-value re-exports crate-
private as well; workspace consumers use `agena-domain` or the typed provider
ports directly rather than treating the legacy provider module as a value
facade.

The current umbrella-import audit confirms that the remaining application/API
uses are not provider-value facades: they are session execution, plugin SDK,
event-bus, message projection, and SeaORM adapter types whose owning contract
or runtime composition slice has not yet been extracted. Those imports remain
tracked as monolith-deletion work; they are not masked by new compatibility
re-exports.

Session cache defaults have also moved to the domain-owned
`SessionCacheLimits` value. Core session orchestration converts that value into
its private eviction policy, while application runtime presentation consumes
the same domain value; the old `agena::session::DEFAULT_SESSION_CACHE_*`
constant facade has been deleted.

The next session extraction audit confirms that `SessionRunOptions` is not yet
a single provider value: it combines `ModelRef`, provider mode/metadata
choices, prompt policy, tool API bindings, agent selection, and core message
projection inputs. It remains a core execution boundary until those fields are
split into their already-owned provider/tool/domain contracts; no duplicate
application-side run-options struct is introduced.

The app transcript test boundary imports `ExecutionStatus` directly from
`agena-domain`; the legacy `agena::message::ExecutionStatus` re-export is no
longer used by application code. Its message projection slice is complete:
Application maps runtime part details directly and does not import Core
`PartContent`.

The application message-to-wire projection consumes domain-owned
`ExecutionStatus` and `PartKind` directly, while its full part/detail mapping
consumes runtime projection values. Architecture checks prevent both a status/
kind facade regression and reintroduction of Core message aggregate rebuilding.

The CLI API probe/test harness follows the same boundary: its transcript and
tool-suite status values now import `ExecutionStatus` from `agena-domain`.
`agena::message` keeps only the core message-part structures that still need a
concrete runtime implementation.

The usage-value audit is complete: provider-owned `CompletionUsage` is used by
session history/cost aggregation as well as provider completions. Core names
that type directly; SQLite's transparent `PersistedCompletionUsage` wrapper
owns the SeaORM query-result adapter.

The terminal startup audit now has executable evidence as well: the
architecture checker verifies that `apps/agena/src/main.rs` delegates to the
single `agena-cli::AgenaCli` parser and does not define a second Clap parser;
the parser derive remains owned by `crates/agena-cli/src/cli/mod.rs`.

The workspace default-member boundary has also been rechecked with a plain
`cargo check`: the root manifest keeps `default-members = ["apps/agena"]`,
while Studio, examples, and E2E tooling remain outside the default build and
are reached only through explicit workspace/package commands. The executable
architecture checker now validates both the manifest declaration and that the
`agena` package resolves to `apps/agena`.

Historical timing observations and the repeatable probe contract moved to the
separate build-performance plan. This V2 document retains only the normal
incremental-profile and target-directory safety invariants.

The workspace gate has since been rerun with the lockfile enforced:
`cargo test --workspace --locked --quiet` passes across all packages,
including trybuild suites, and `cargo fmt --all --check` plus the executable
architecture checker pass as well. The pinned local `cargo-machete` and
`cargo-deny 0.19.4` checks are also green; the latter retains only the
documented non-failing third-party dependency warnings. The repository CI
workflow defines both checks explicitly: the deny job uses the pinned
`cargo-deny` action, and the unused-dependency job installs a pinned
`cargo-machete` release before running it.
The architecture checker now protects that workflow contract as well, covering
fmt, locked Clippy, locked architecture/test commands, machete installation,
and the cargo-deny action.
It also protects the timing job contract: CI must run the report-only leaf
probe and publish the `build-timing-report` artifact for later budget review.

The pinned local `cargo-machete` run is now clean as well. It initially found
nine unused manifest edges across core, API server, CLI, and the final app;
after verifying source usage, those stale dependencies were removed and a
second run reports no unused dependencies. The workspace lockfile was updated
through a normal check and the locked workspace check/Clippy gates pass again.

The pinned local `cargo-deny 0.19.4` run also exits successfully for all four
requested scopes (`bans licenses sources advisories`). Its report retains
non-failing warnings for duplicate transitive versions, the yanked
`spin 0.9.8` selected by existing SQLite/fluent dependencies, and deprecated
SPDX spellings in third-party manifests; these are dependency-maintenance
follow-ups, not deny policy failures.
The local license configuration itself is now warning-clean: unmatched
`OpenSSL` and `GPL-*` allowances were removed from `deny.toml`, leaving only
licenses actually encountered by the current graph.
The yanked `spin` path is transitive through the pinned `fluent-templates` /
`sqlx-sqlite 0.8` stack (with a separate `sqlx-sqlite 0.9` branch for Studio),
so resolving it requires an upstream dependency upgrade rather than a local
facade or manifest-edge change. The reverse dependency tree is recorded for
that follow-up.

The same checker audits the repository's active developer entrypoints
(`README.md`, CI, and dependency scripts) for deleted `apps/agena-cli` and
duplicate `agena-tui` paths. Historical migration references remain confined
to this plan and other archival documentation; active commands cannot silently
reintroduce the V1 app layout.

One unused deprecated alias has now been removed as well:
`agena-plugin-host::RegisteredTool::model_name` had no workspace callers and
only forwarded to the canonical registry identity. The method and its
compatibility comment are deleted, with an architecture assertion preventing
the legacy name from returning.

The strict lint gate is now green as well: `cargo clippy --workspace
--all-targets --locked -- -D warnings` passes after correcting test-only
constant assertions, default-struct field reassignment, post-test-module
imports, and a few needless conversions/borrows exposed by the locked
all-targets run. The affected runtime, domain, core, and architecture tests
remain green after those mechanical fixes.
The complete workspace regression gate was rerun after the fixes with
`cargo test --workspace --locked --quiet`; all package, integration, doctest,
and trybuild targets passed.

The provider-registry composition audit confirms that
`build_provider_registry_from_inputs` still deliberately consumes the core
`ResolvedProviderConfig` map: it performs concrete credential, adapter, HTTP,
and plugin-host construction, so replacing that input with a provider contract
without first extracting the adapter-construction inputs would only create a
larger facade. This remains an explicitly tracked runtime-composition slice,
not an unguarded provider dependency.

The provider tool-exposure policy has now crossed the value boundary: the
canonical `AgenaToolMode` enum lives in `agena-provider`, while core config
retains only the surrounding `AgenaToolsConfig` aggregate and imports the
provider-owned mode. Provider adapters, catalog decoration, and request
preparation therefore share one contract value without duplicating the enum.
Provider runtime modules now import that value directly rather than reopening
the core config facade; application provider-studio editing paths now do the
same, and an architecture check guards the direct-import rule across
production sources.

The aggregate `AgenaToolsConfig` has now crossed the same boundary: its
provider-native bindings and mode are defined together in `agena-provider`,
while core configuration only reuses the contract type inside its larger
resolved configuration structures.
The obsolete `agena::config` public re-exports for both provider tool values
have now been removed; config parsing uses them privately and all downstream
runtime/presentation consumers import the provider contract directly.
The public `config` umbrella is guarded as well, so the migration cannot be
reintroduced by moving the same names back into `config/mod.rs`.
`agena-provider` now owns a direct contract test for the aggregate's default
mode and serialized wire shape, so this boundary is verified without loading
core configuration.

The resolved per-model configuration has crossed that boundary too:
`ResolvedProviderModelConfig` now belongs to `agena-provider` alongside its
`ConfiguredModelDefinition`, native-tool bindings, and `AgenaToolsConfig`
members. Its compatibility defaults and rejection of obsolete native-tool
keys remain part of the provider contract and are covered by a contract-unit
test. Core configuration parses and stores the provider value directly, but
no longer defines, aliases, or publicly re-exports it; catalog and
provider-studio consumers now import it from `agena-provider` directly.
Architecture checks prohibit restoring either the former
`ProviderModelOverlay` alias or a Core `ResolvedProviderModelConfig` facade.

Provider-specific network timeout policy now follows the same ownership rule:
`ProviderNetworkConfig` and its stable request/connect defaults are defined
and tested in `agena-provider`. Core configuration consumes the contract while
parsing resolved provider settings, and Provider Studio obtains its new-draft
defaults from that direct contract import. The Core config umbrella no longer
exports the type; an architecture assertion prevents that facade from
returning.

Provider request-route configuration is contract-owned as well:
`ProviderProtocolPathsConfig`, `ProviderModelDiscoveryConfig`, and the Cline
base/protocol-path defaults now live in `agena-provider`. Core's raw parser,
registry construction, adapter-model lookup, and concrete builder import those
values directly; no Core config umbrella re-export remains. A Provider
contract test preserves the Cline path/default wire behavior, while the
architecture checker rejects a Core route-configuration definition, export,
or downstream Core-facade import.

The provider secret-source and GitLab API-access records are Provider-owned
now too. They carry only inline/environment source selection and the
provider-owned `AuthData` contract, including redacted debug behavior; Core
continues to use them inside its larger auth-schema aggregates without
defining or exporting facades. The Provider contract test asserts that inline
secrets remain redacted, and the architecture guard prevents the old Core
names from returning through either definitions, config exports, or consumer
imports.

`BedrockSigv4AuthConfig` is Provider-owned too. Its concrete access-key,
secret-key, and session-token fields are request-auth contract data, and its
redacted debug behavior is verified in `agena-provider`; Core's API-auth
schema projects directly to that value without a public config facade.

The complete credential-auth value family now belongs to `agena-provider`:
`ProviderInlineCredentialAuthConfig`, the HTTP, SAP AI Core, and GitLab
records, plus the tagged `ProviderCredentialAuthConfig` aggregate. Core still
owns the surrounding auth schema and parsing decisions, but consumes these
values directly rather than defining or re-exporting a config facade. The
Provider contract test covers issuer, base URL, protocol-path, and credential
access for the Google ADC route; the architecture guard checks all five
definitions, Core's lack of facades, and downstream source imports.

The stable notification severity used by the interaction tool has likewise
crossed into `agena-domain` as `InteractionNotificationLevel`; the core tool
input remains core-owned because it carries the concrete `ToolInput` schema,
but plugin presentation now consumes the domain value directly.

`ToolResultState` has now followed the same rule. The lifecycle enum is
domain-owned, while the core `ToolResultEnvelope` keeps the operation blocks,
attachments, managed outputs, and persistence/runtime presentation fields that
still require core types; application projection consumes the domain state
directly.
The small `ToolResultDisplay` title/summary value is domain-owned alongside
that state; only the envelope remains core-bound.
The standalone `OperationError` message/code value is domain-owned as well;
core aggregates continue to own only the structures that embed attachments,
operation blocks, or other runtime-specific payloads.

Together with `InteractionNotificationLevel`, these values close the remaining
small tool-result/interaction enums and records that had no persistence or
runtime-specific ownership reason to remain in core.

`MessageSource` is now domain-owned as well (`User`, `Assistant`, `System`).
Core message metadata uses that value while retaining only the JSON/SeaORM
metadata aggregate as its adapter; application and TUI projections no longer
depend on `agena::message::MessageSource`, and the old core re-export is
deleted. Session history, manager, processor, prompt-window, and model paths
now import the domain source directly.

The `MessageProviderState` audit found an existing provider-owned replay value
(`CompletionInputProviderState`), but the persisted activity-message entity
still requires a core-local SeaORM JSON adapter (`Into<sea_orm::Value>` and
`FromJsonQueryResult`). Directly aliasing the core type would break the active
model, so this remains a deliberate adapter boundary until a persistence
wrapper/conversion is introduced.

The conversion boundary is now explicit: core `MessageProviderState` retains
the SeaORM-compatible persisted shape, implements bidirectional `From` mapping
to `CompletionInputProviderState`, and provider wire projection uses that
mapping. The persistence wrapper remains core-owned, while replay semantics
are no longer duplicated in the provider request path.

Provider metadata ingestion in session processing now constructs the provider
`CompletionInputProviderState` first and converts it once into the persisted
core wrapper. Both directions therefore use the same typed boundary instead of
repeating provider-field assembly in separate session and wire paths.

The same ownership rule now applies to usage normalization: OpenAI,
Anthropic/Bedrock, Gemini, and compatible Chat adapters construct the
provider-owned `CompletionUsage` directly. Core's `MessageUsage` module is
deleted; SQLite alone owns the persisted wrapper, and an architecture check
prevents provider mappings from reintroducing a second aggregate.

Model-catalog refresh composition now crosses a value-only priority boundary:
`ModelCatalogService` and its live-source collector accept an optional
provider-owned `ProviderModelPriorities` value rather than
`ResolvedProviderConfig`. Snapshot composition performs the one-way conversion
from resolved adapter definitions to provider priorities through
`agena_runtime::provider_model_catalog_priorities`, and the application
refresh path now performs the same Runtime conversion directly at its concrete
composition edge. This keeps catalog
fetching/curation independent of core configuration structs while retaining the
existing provider-specific ranking policy in the Runtime composition adapter.

The final facade audit now has executable guards for the two remaining small
message boundaries that are intentionally not domain-extracted yet. Attachment
values are re-exported only from the plugin-host SDK bridge (the core module
contains no duplicate definitions), because plugin-produced attachments must
cross the host boundary unchanged. `CompletionUsage` crosses unchanged through
provider, session, and application paths; its SQLite-only
`PersistedCompletionUsage` wrapper supplies the SeaORM adapter without
creating a Core value model.
The architecture checker locks both ownership decisions so a compatibility
re-export cannot be mistaken for a new canonical contract.

The snapshot service-bundle assembly has also been reduced by one more layer:
`RuntimeSnapshot` now constructs `agena_runtime::RuntimeServiceBundle` directly,
and the legacy-core `build_runtime_services` wrapper is deleted. Runtime owns
the generic retention/lifecycle aggregate; core snapshot code supplies only the
  concrete service handles and the still-core event/session/provider adapters.
  The configured local-model route/default projection and enabled-adapter list
  no longer belong to this file: they are supplied by
  `agena_runtime::{configured_local_models,configured_enabled_adapter_ids}`;
  Core only decorates the resulting domain models against its concrete
  provider adapter and catalog service.
The optional database-to-session composition is now also kept at the snapshot
call site; the redundant core `build_session_service` passthrough was removed,
so the remaining session builder is only the actual reconfiguration/constructor
logic that must eventually receive typed runtime inputs.

The session policy projection is now another explicit value boundary:
`agena_runtime::session_build_config_from_resolved` materializes the default
selection, default agent, permission, compaction, and tool-presentation values
from `ResolvedConfig`. Core snapshot composition passes that Runtime-owned
value into its concrete `SessionManager` adapter; it no longer reconstructs the
session policy aggregate inline.

The deprecated core `tool_api` and `tool_protocol` facade files are also now
covered by the executable boundary check: neither module may reappear while
the remaining provider/tool presentation adapters continue to use their
domain-owned contracts directly.

The terminal replacement boundary is now checked independently as well:
`apps/agena-cli` is absent, `apps/agena` owns the sole `agena` binary target,
and `crates/agena-tui` has no binary target. This makes the app/package
replacement evidence explicit rather than relying only on Cargo's binary-name
count. `agena-cli` now also has parser smoke contracts proving that a bare
invocation resolves to `LaunchMode::Tui`, while an explicit subcommand remains
in `LaunchMode::Command`; the app entrypoint delegates both paths to that one
parser. The architecture checker locks both smoke contracts alongside the
single-parser source guard.

The legacy core root module has also been audited: both the application-error
root re-export and the plugin-host bridge are now deleted, so callers use
`agena::error::AppError` and `agena-plugin-host` directly. Architecture checks reject restoring
`agena_domain`, `agena_provider`, or `agena_tool` umbrella exports at
`agena::` root; remaining module-level exports are tracked as concrete
composition/adapters rather than root compatibility aliases.

The persistence audit for this phase found no database-format change requiring
a data rewrite. The activity entities still use the existing table and column
names, the `StoredRole`, `StoredExecutionStatus`, and `StoredPartKind` adapters
preserve the established SQLite numeric assignments, and the provider-state
and metadata moves preserve their existing JSON fields. The adapter boundary is
covered by round-trip and persisted-code tests. `db/schema.rs` now records
schema version 1 transactionally, upgrades legacy version 0 databases
idempotently, and rejects newer unsupported versions; future user-data changes
must add a numbered migration branch with an explicit fixture test.

The active developer-entrypoint audit also removed the last stale `agena-app`
package references from `README.md`. Its Cargo commands now target the actual
`agena` package, and the architecture checker rejects both the deleted
`apps/agena-cli` paths and the obsolete `agena-app` package name in active
README/CI/ops entrypoints. Historical plan text may still mention the temporary
name when describing an earlier migration phase; it is not an executable
developer command.

The model-catalog service constructor has now crossed the storage port: it
accepts `Arc<dyn agena_storage::ModelCatalogRepository>` plus the cache-age
value it needs, while the concrete `SeaModelCatalogRepository` is constructed
only by Runtime's catalog service from `agena-storage-sqlite`. The
service no longer names or reopens the concrete store, and the architecture
checker guards that split, including the optional-database composition helper;
an in-memory repository contract test also restores
a cached snapshot without a database. The broader shared-schema migration
remains pending.

The plugin-host bridge migration is complete at package granularity:
`agena-application` declares `agena-plugin-host` directly and all of its plugin
registry/UI/RPC types use that crate path. The core implementation has also
switched its internal imports, and the root architecture check prevents the
legacy `agena::plugin` bridge from returning.

The API-server package has now crossed the same bridge: its RPC, callback, and
plugin-UI handlers declare `agena-plugin-host` directly and no longer import
plugin types through `agena::plugin`; its package-level architecture check
guards that boundary.

The CLI package has now crossed the bridge as well: plugin manifest validation,
signature verification, hook parsing, status/log output, and presentation
configuration use a direct `agena-plugin-host` dependency. Its optional
signing feature is also wired to the host crate rather than the legacy core
feature, with a package-level architecture check guarding the boundary.

The final `apps/agena` package has now crossed the bridge too. Backend plugin
catalog/status/log access, command actions, plugin workbench schema/policy
types, theme/statusline values, and timeline event kinds all use its direct
`agena-plugin-host` dependency; the active app source tree contains no
`agena::plugin` path. A workspace-wide reference audit confirmed no remaining
consumers, and the root plugin re-export has been removed.

The root-facade audit is now executable rather than descriptive: the
architecture checker asserts that `crates/agena/src/lib.rs` contains no public
`pub use` compatibility re-export at all. This closes the remaining umbrella
facade checklist item while leaving the legacy core modules themselves in
place until the runtime-composition and monolith-deletion phases are complete.

Provider failure classification has crossed the same ownership boundary:
`ProviderErrorKind` (`ApiError` and `ContextOverflow`) now lives in
`agena-provider`. Core `AppError` carries that provider contract value, while
provider adapters, retry policy, OAuth, and compaction consumers import it
directly from the provider crate. The architecture checker rejects restoration
of the old `crate::error::ProviderErrorKind` definition or import path.

`SystemNoticeKind` has likewise moved to `agena-domain`. The history event
wrapper remains core-owned because it materializes transcript messages and
parts, but its stable notice category is now shared domain data rather than a
core history enum. The domain export and absence of a core redefinition are
covered by the architecture checker.

Agent profile scope is now domain-owned as `agena_domain::AgentScope`. The
core agent registry stores and ranks that value, but no longer defines it;
application projections and host-client parsing use the domain contract before
mapping to API wire `AgentScope`. This removes the previous application
dependency on `agena::agents::AgentScope`, with ownership and import direction
guarded by the architecture checker.

The interactive-request audit keeps one intentional split explicit. Domain
owns the stable request category, question/option payloads, and request/reply
values, while core `RequestPart`/`PendingInteractiveRequest` and
`InteractiveRequestPart` own the concrete message/session lifecycle projection.
Only the enclosing AskUser tool input remains core-owned for its model-facing
`ToolInput` contract and JSON schema. The architecture checker guards both
ownership boundaries.

The V2 entrypoint guard now covers the active README, CI and dependency-report
workflows, dependency check script, and provider probe script. Each is checked
for the deleted `apps/agena-cli` paths and obsolete
`agena-app` package name; historical migration prose remains outside this
active-command scan.

The README's pre-merge workspace verification examples now use the same
locked check, Clippy, and test commands as CI. The architecture checker asserts
those three exact command forms so local documentation cannot silently drift
back to an unpinned dependency resolution.

The CI regression matrix now runs the complete `cargo test --workspace --locked`
gate after workspace checking, before its focused CLI/Studio/example probes.
This makes the full workspace test evidence an actual CI requirement rather
than only a local execution-ledger command.

The CI `feature-checks` job now likewise covers the marketplace server
`server` feature plus all-features checks for plugin-host, Runtime, API-server,
and CLI. The architecture checker asserts each command form, so the feature
capability matrix is executable evidence rather than an aspirational checklist.
After adding those guards, the historical checkpoint passed the full locked
workspace check, strict Clippy, and workspace test gate; the existing linker
`__eh_frame` warning remains a non-failing historical timing observation. The
active source batch has not rerun these gates.

The same command was rerun against the historical checkpoint after the latest
storage/domain boundary changes (`cargo test --workspace --locked --quiet`).
All package unit/integration targets, trybuild fixtures, and doctests passed;
the cache-repository contract test also passed after correcting its timestamp
option assertion.

The runtime-composition cleanup was then verified with the same workspace gate:
the core EventBus-to-plugin adapter now lives at `event/bridge.rs` rather than
under `runtime/`, while `agena-runtime` continues to own forwarding and
abort-on-drop lifecycle behavior. The full workspace test, workspace Clippy,
and diff checks all pass after the move, including the model-catalog integration
target that uses the explicit `agena_core::error::AppError` boundary.

- [x] Full workspace format, workspace Clippy, workspace tests, protocol
  fixtures, and the existing TUI tests pass on the current worktree through the
  locked consolidated functional pipeline recorded above.
- [x] Default build excludes Studio, examples, and E2E tools (`cargo check`;
  architecture checker validates the sole default member).
- [x] Default terminal build links exactly one binary (architecture checker
  validates the sole `apps/agena` target).
- [x] Persistent user data uses an explicit SQLite `PRAGMA user_version`
  migration contract in `crates/agena-storage-sqlite/src/schema_lifecycle.rs`:
  schema version 1 is recorded transactionally, legacy version 0 databases are
  upgraded idempotently, and newer unsupported versions are rejected. The
  SQLite tests and architecture checker cover the version marker and upgrade
  contract. Every future column/data change must add its numbered migration
  branch and fixture test; that is an enforced maintenance rule, not an
  outstanding implementation gap.
- [x] Active documentation, package scripts, and developer commands describe
  V2 only. The internal provider/MCP/runtime harness is consistently named
  `agena-e2e` in the workspace, README, CI, and architecture checks. The plan's
  baseline and phase sections retain historical migration wording only where
  it is explicitly labeled as historical; active-entrypoint scans reject the
  deleted V1 paths and obsolete package names.

Checklist audit rule: an `[x]` above means a completed source-ownership or
current executable-evidence slice. Build-performance partial invariants and
their future evidence live exclusively in the separate performance plan.
