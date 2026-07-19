# Agena Architecture V2 Refactor Plan

**Status:** approved implementation plan; not yet started.

**Scope:** a deliberate, source-breaking refactor of the Rust workspace. The
goal is a maintainable, testable, and fast incremental-build architecture, not
an incremental cleanup of the current `agena-cli`/`agena` layout.

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

## Current diagnosis

The workspace has two overlapping structural problems.

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
agena-api             -X-> agena-runtime, agena-application, concrete adapters
agena-client          -X-> agena-domain, agena-runtime, agena-application
```

Only app packages may construct concrete runtime implementations or perform
process-wide initialization.

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

This is a composition layer, not a second monolith. It reads resolved
configuration, builds storage/provider/tool/plugin implementations, creates
application services, manages reload/background tasks/shutdown, and returns a
typed application handle to presentation layers.

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

1. Make full workspace Clippy strict and green.
2. Capture CLI help, exit-code, stdout/stderr, and JSON golden tests.
3. Capture API request/response/notification fixtures and protocol tests.
4. Add TUI characterization tests for startup, terminal restore, transcript
   navigation, mouse/text selection, copy, images, formulas, and layout.
5. Add a metadata-based architecture checker with forbidden-edge assertions.
6. Record build timings for the normal edit loop and final app build.

Exit criteria:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

all pass, and the baseline timings/target graph are committed as documentation
or reproducible benchmark instructions.

### Phase 1 — Remove the duplicate terminal entrypoint

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

Create `agena-domain` and migrate in dependency-light slices:

1. IDs and newtypes.
2. Roles and model value types.
3. Message value types.
4. Permission value types.
5. Event envelopes and filters.
6. Execution preferences and pure agent configuration.

Each move updates all callers and deletes the original module immediately;
there is no old-core re-export.

Exit criteria:

```text
agena-domain has no I/O, async runtime, DB, transport, UI, CLI, or SDK deps.
```

### Phase 6 — Extract ports and infrastructure implementations

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

1. Complete `agena-runtime` as the only concrete composition/lifecycle crate.
2. Move remaining runtime, plugin orchestration, registry, reload, and
   background-task code out of old `crates/agena`.
3. Delete `crates/agena` entirely.
4. Rename `apps/agena` package from temporary `agena-app` to `agena`.
5. Update root default members and all package references.
6. Delete deprecated aliases such as the old `tool_protocol` re-export.

Exit criteria:

```text
crates/agena no longer exists.
package agena denotes the final app only.
No old agena::* facade import remains.
```

### Phase 8 — Move and redesign the TUI

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

### Phase 9 — Build graph and feature optimization

1. Rename `tools/agena-cli-test-tools` to `tools/agena-e2e` because it tests
   providers/MCP/plugins/runtime rather than CLI ownership.
2. Isolate expensive provider SDKs and other heavy dependencies in leaf
   adapters.
3. Add feature checks only where they represent real product capabilities.
4. Make CI test layers in parallel and retain a full workspace gate.
5. Add timing checks and target-graph assertions to prevent regressions.

Exit criteria:

```text
TUI work does not rebuild concrete provider adapters.
API client work does not rebuild runtime.
Default build does not build Studio, examples, or E2E tools.
Default terminal build links exactly one agena binary.
```

### Phase 10 — Final cleanup

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

## Build performance acceptance criteria

Measure on a fixed development machine, Rust toolchain, and Cargo profile.
The target graph matters as much as elapsed time.

| Scenario | Target budget |
| --- | ---: |
| No-change `cargo check -p agena-tui` | <= 1 second |
| TUI leaf change, `cargo check -p agena-tui` | <= 15 seconds |
| CLI leaf change, `cargo check -p agena-cli` | <= 10 seconds |
| TUI leaf change, final `agena` build | <= 30 seconds |
| No-change root `cargo build` | <= 2 seconds |
| Default final terminal link count | exactly 1 |

Additionally, a TUI-only change may rebuild `agena-tui` and final
`apps/agena`, but must not rebuild AWS/Google provider adapters, SQLite
implementation, API server, or remote client. Full-workspace cold builds may
remain slower; they are CI/release work, not the edit loop.

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
perf(build): enforce leaf-oriented incremental builds
docs(architecture): publish the final dependency contract
```

Do not combine a massive mechanical move with behavior changes. Verify moves
with Git rename detection, then make semantic changes in a following commit.

## Completion checklist

- [ ] The sole terminal product binary is `agena`.
- [ ] No `agena-tui` binary or duplicate TUI parser remains.
- [ ] `apps/agena-cli` no longer exists; final app is `apps/agena`.
- [ ] The old `crates/agena` monolith no longer exists.
- [ ] No compatibility facade or old `agena::*` umbrella re-export remains.
- [ ] CLI, TUI, and API Server share application services.
- [ ] TUI does not depend on API Server, SQLite, or concrete adapters.
- [ ] API wire contracts do not depend on runtime/application.
- [ ] Client does not depend on runtime/domain.
- [ ] Domain has no I/O/UI/CLI/transport/SDK dependency.
- [ ] Provider, storage, and tool contracts are independent of implementations.
- [ ] Runtime is the only concrete composition layer.
- [ ] TUI uses State/Action/Effect/View and transcript vertical slices.
- [ ] Architecture metadata check is enforced in CI.
- [ ] Full workspace format, Clippy, tests, protocol fixtures, and TUI tests pass.
- [ ] Default build excludes Studio, examples, and E2E tools.
- [ ] Default terminal build links exactly one binary and meets timing budgets.
- [ ] Persistent user data changes, if any, use explicit tested migrations.
- [ ] Documentation, package scripts, and developer commands describe V2 only.
