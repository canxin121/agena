# Agena plugin system: Cordis and DeepSeek Harness study

Status: source-level study plus Phase 1 implementation
Date: 2026-08-19

## Research baseline

This study used local shallow clones under `/tmp/agena-cordis-research`:

- `deepseek-ai/deepseek-harness` at `99f6f02fecdb7dff40c3fbc9470f5907c29f74ca` (MIT)
- `cordiverse/cordis` at `8cc9e33fab69e2d0476d126baaf2acb24e6a6ab4` (MIT)

Reviewed sources include the Harness architecture, Cordis primer, capability
seams, agent lifecycle, tool pipeline, config catalog, module graph, defensive
patterns and testing docs; Cordis core `context/service/registry/fiber/reflect/events`
and loader/include/HMR sources; and Harness scope, tools, profiles, agent
presets, host/client runners, dynamic inventory and UI extension packages.

## Conclusion

Agena should borrow Cordis semantics, not embed Cordis or port its JavaScript
implementation literally. Agena already has stronger cross-process transports,
Runtime-owned permissions, durable sessions, replayable operations, and a
closed settings AST. Replacing those with a generic in-process context would be
a regression.

The useful semantic core is:

1. dependency-driven activation instead of incidental load order;
2. one lifecycle owner for every registration/effect;
3. quiescent disposal before replacement;
4. scoped capability overlays resolved by one shared visibility algorithm;
5. explicit event composition modes;
6. stable-id profile composition and generated architecture catalogs.

These primitives must sit below Agena's existing authorization, transport and
durability boundaries.

## Why the Harness plugin system feels good

### Dependency, lifetime and inspection are one mechanism

Cordis runs a plugin in a `Fiber`. The fiber resolves declared services,
validates config, records every listener/registration as an effect, owns reverse
cleanup and exposes its state for inspection. Missing services keep a plugin
pending. Provider replacement changes the dependency epoch and reactivates the
dependent closure. Failed init unwinds effects already registered.

### A Context is a capability view

Service reads go through one resolver with isolation and interception. DeepSeek
uses this to separate capability definition, provider and consumer—for example,
local and sandboxed filesystem/subprocess providers can be swapped without
rewriting the tools consuming them. Catalog, lookup and dispatch share the same
view.

### Scopes are overlays with ownership

DeepSeek adds opaque scope keys and parent chains. Registries have a global
layer plus exact-scope overlays. Children inherit ancestor layers; nearest
same-name entries shadow. Registration through a scoped context also assigns
cleanup ownership to that scope's backing fiber. This supports global tools,
preset policy, and agent/session-local overrides without making scope an
authorization boundary.

### Events have declared composition semantics

Cordis distinguishes synchronous observe, parallel observe, serial/bail and
waterfall/around dispatch. DeepSeek's tool pipeline uses an extensible
pre-policy waterfall, monotonic deny-only guards, around-dispatch wrappers for
timeout/retry/metrics, post-processing, a definition-owned finalizer, then an
observe-only frozen result event.

### Profiles compose a real plugin tree

Ordered bundle layers plus stable-id patches produce the effective plugin tree.
Rows can be inserted, disabled, replaced or reconfigured. The resolved tree can
be dumped. The loader updates changed entries rather than treating the whole
application as unrelated new state.

### Architecture is generated and inspectable

The Harness generates/verifies config, tool and Cordis catalogs, module and
capability graphs, and event producer/consumer matrices. Host and browser both
have plugin runtimes with inventory and lifecycle state, rather than hiding
plugin state in individual UI components.

## What Agena must preserve

| Concern | Agena authority that remains | Why |
|---|---|---|
| Security | Runtime permissions and Host APIs | scope visibility is not authority |
| Isolation | static/cdylib/stdio/http/wasm transports | effect ownership does not replace process/sandbox isolation |
| Durability | session parts and structured tool/operation results | a live event bus cannot become durable truth |
| Forms | closed settings AST | generic JSON Schema would recreate Web/TUI drift |
| Operations | server-resolved results and bounded host effects | client recursion is harder to authorize and replay |
| Reload | Runtime snapshot replacement | composition must integrate with, not bypass, Runtime generations |

## Agena gaps found

1. `PluginHost::new` used alphabetical initialization, not a dependency graph.
2. Failed initialization had bespoke rollback, while normal shutdown did not
   share the same host-contribution cleanup path.
3. Shutdown did not close admission before waiting for accepted calls/streams.
4. Dynamic registries are plugin-owned globally but lack workspace/preset/
   agent/session overlay semantics.
5. Hook dispatchers encode ordering, mutation and failure handling separately;
   there is no common observe/bail/transform/around/guard vocabulary.
6. Runtime reload can reuse transports, but it does not yet restart only the
   transitive dependent closure of a changed provider.
7. Inspect surfaces lack dependency/service graphs and effect inventories.

## Phase 1 implemented in this change

### Dependency-driven activation

`ConfiguredPlugin.activation` now supports:

```json
{
  "activation": {
    "requires": ["example.provider"],
    "after": ["example.observer"]
  }
}
```

- `requires` is hard: missing, disabled, blocked, cyclic or failed providers
  keep the consumer inactive.
- `after` is a soft ordering hint; it never blocks and soft cycles fall back to
  lexical order.
- self-dependencies, duplicates and overlap are rejected.
- transitive blockers are recorded before transport startup with stable codes.
- a provider that fails initialization blocks later hard dependents in the same
  host build.
- disabled and blocked rows remain visible in plugin status/inspect surfaces;
  inspect returns required/soft dependencies plus a non-sensitive structured
  blocker even when the plugin never reached `meta/init`.
- `agena plugin validate` evaluates the same graph and reports missing,
  disabled, transitively blocked and cyclic hard dependencies before startup;
  missing soft `after` hints remain valid.

Implementation:

- `crates/agena-plugin-host/src/activation.rs`
- `crates/agena-plugin-host/src/config.rs`
- `crates/agena-plugin-host/src/host/plugin_host_build.rs`
- `crates/agena-plugin-host/src/host.rs`
- `crates/agena-plugin-host/src/host/plugin_host_core.rs`
- `crates/agena-cli/src/cli/cli_validation.rs`

Service resolution now feeds the same planner directly. Declared service
imports resolve to exactly one provider/version before activation; required
bindings become hard activation edges, optional bindings remain late-bound,
and provider epoch changes feed the restart plan. The wire call contract is
typed and transport-neutral rather than an ambient in-process lookup.

### Quiescent transport lifecycle

Every newly loaded transport is wrapped by `QuiescentTransport`:

1. accepted dispatches, notifications, host attachment, stream starts and
   stream events acquire an activity guard;
2. shutdown closes admission atomically;
3. accepted calls and streams are awaited to terminal completion;
4. `meta/shutdown` runs only after quiescence;
5. the underlying transport closes once;
6. late calls fail as disconnected.

A dropped stream consumer does not abandon plugin work: the wrapper drains the
underlying stream to its terminal result so shutdown can make a truthful
quiescence claim.

Implementation:

- `crates/agena-plugin-host/src/transport/quiescent.rs`
- `crates/agena-plugin-host/src/transport/mod.rs`
- `crates/agena-plugin-host/src/loader.rs`

### Shared host-owned effect cleanup

Failed init and normal shutdown now share the idempotent
`dispose_plugin_resources` path. It releases callback tokens, transport routes,
indices, names, hooks, dynamic tools, tool events, display contributions,
themes, recent notifications and quota state. Shutdown failures are logged and
cleanup continues.

Implementation:

- `crates/agena-plugin-host/src/host/host_handle.rs`
- `crates/agena-plugin-host/src/host/plugin_host_core.rs`

### Shared settings/operation execution and fixed client workbenches

The existing unified settings/operation refactor is completed through the
same composition boundary:

- `SettingsContract::default_value()` produces one deterministic editor and
  invocation seed;
- `SettingsContract::parse_shorthand()` owns full JSON, single-field scalar,
  multi-field `field=value`, choice, numeric and boolean parsing;
- the Host catalog publishes `accepts_empty_input` and `default_input`, so Web
  and TUI do not reimplement constraint logic;
- Application resolves an operation target exactly once: method targets cross
  the plugin transport, while tool targets use the existing session,
  permission and tool-execution path;
- the REST surface has one operation endpoint and one explicit tool endpoint;
  `/ui/actions` and `/commands` are removed;
- Web and TUI consume the final `PluginOperationResult` and only apply the
  bounded `PluginHostEffect` set;
- plugin settings have a dedicated read/save API validated by the same
  contract before Runtime reload;
- the Web workbench is host-owned (`Overview`, `Settings`, `Operations`,
  `Tools`, `Logs`, `Diagnostics`) and cannot be replaced by plugin-authored
  views or controls;
- the TUI retains its keyboard editor through an internal adapter generated
  only from the closed SettingsContract AST, never from plugin-authored JSON
  Schema.

Implementation:

- `crates/agena-plugin-contracts/src/lib.rs`
- `crates/agena-application/src/application_plugins.rs`
- `crates/agena-api-server/src/rest/plugins.rs`
- `crates/agena-tui-plugin-workbench/src/model/workbench_schema_validation/schema_materialization.rs`
- `crates/agena-tui-app/src/app_backend/plugin_effects.rs`
- `packages/agena-web/src/lib/pluginOperations.ts`
- `packages/agena-web/src/components/settings/PluginContractEditor.vue`
- `packages/agena-web/src/components/settings/PluginsPanel.vue`

## Additional Cordis semantics implemented

### Host-resolved service graph

Configured plugin instances may now export/import a stable service id and API
major. The activation planner resolves one provider, requires explicit
selection when providers are ambiguous, reports version incompatibility before
startup, and turns required bindings into hard dependency edges. Optional
bindings are late-bound soft edges. The resolved provider is visible in inspect
and the architecture catalog.

### Recursive activation epochs

Transport reuse is no longer based on direct configuration equality alone.
Each plugin has a canonical recursive epoch that includes every hard dependency
epoch, so provider changes propagate through the complete dependent closure.
Optional providers do not cause eager restarts.

### Effect scopes

Every loaded plugin owns an asynchronous effect scope. Host resources and the
quiescent transport are disposed in reverse order, exactly once, and child
scopes are awaited. The complete inventory and failures remain inspectable.

### Scoped registries

A generic global-plus-overlay registry now supplies parent inheritance, nearest
shadowing, cycle rejection, no layer creation on read, exact-layer duplicate
checks, owner-bound registration and automatic empty-layer reclamation.

### Typed event pipelines

Lifecycle-owned observe, parallel-observe, bail, transform, monotonic guard and
around-middleware pipelines now provide a common composition vocabulary and
explicit failure policies. Method-backed plugin operations already execute
through the typed around pipeline, and middleware registration is owned by the
registering plugin's effect scope.

### Ordered profiles and explicit reload decisions

`PluginsConfig.profiles` now resolves stable-id overlays into the existing
`plugins.list` map before transport startup. Layers can upsert/remove/disable,
merge config, replace activation, and bind a service provider. Provenance and a
stable diff are retained. A separate reload plan classifies every row as
add/reuse/restart/remove/disabled/blocked with explicit reasons and drives
transport reuse.

### Live architecture catalog

`GET /api/v1/plugins/architecture` exposes plugin nodes, explicit/service edges,
activation epochs, blockers, effect ownership, reload decisions, profile provenance and profile changes. The Web workbench renders
resolved services and effect state rather than hiding them in raw logs.

See `docs/plugin-services-scopes-and-pipelines.md` for the author/operator
contract and examples.

## Target architecture

### 1. Prepared transport and service graph

Split load into:

- **prepare**: construct/verify transport, attach host, fetch and validate the
  immutable manifest, but do not call `meta/init`;
- **activate**: resolve service imports, compute graph order, then initialize.

Add wire-safe contracts in `agena-plugin-contracts`:

```rust
PluginServiceExport { id, api_version, visibility }
PluginServiceImport { id, api_version, optional, provider }
```

The Host resolves exactly one provider per import. Provider replacement restarts
only the transitive required-dependent closure. Optional imports use explicit
late-binding semantics rather than ambient lookup.

### 2. One effect scope per plugin instance

Add `PluginEffectScope` with lifecycle state, admission/cancellation, reverse
sync/async disposers, child scopes, active-call counters, labels/source
locations and shared idempotent `dispose()` completion. `QuiescentTransport` is
the first part of this boundary. Host registries should next return effect
handles registered into the owner scope instead of relying on map-wide retain.

### 3. Scoped registries

Use one generic global-plus-overlay registry for tools, prompt sections,
operations, policies, activity providers and client slots:

```text
runtime generation -> workspace -> agent preset -> session/agent execution
```

Rules: reads never create layers; ancestors apply farthest to nearest; nearest
same-name entry shadows; duplicates within an exact layer fail; registration
returns one idempotent undo; empty overlays are reclaimed; catalog, lookup,
presentation and dispatch use the same resolver. Scope controls composition,
never permissions.

### 4. Typed pipelines

Migrate hook dispatch incrementally to explicit modes:

- `Observe`: contained/logged failures;
- `ParallelObserve`: concurrent, aggregate failures;
- `Bail`: first answer;
- `Transform`: ordered value changes;
- `Around`: middleware with `next`;
- `Guard`: monotonic deny-only result.

Every event contract declares mode, live/durable status, scope subject,
cancellation, failure containment and mutation rights. Tool execution should be
first because it already has the strongest authority boundary.

### 5. Profiles and differential reload

Compose stable plugin rows as:

```text
base bundle -> selected bundles -> workspace overlay -> user overlay -> CLI overlay
```

Support insert/replace/disable/reconfigure by stable id, provider binding, graph
dump/diff, and restart of only the changed dependent closure. `plugins.list`
remains the resolved leaf format; profiles must not create a second config model.

### 6. Generated catalogs and client plugins

CI should generate/verify plugin dependency and service graphs, event matrices,
tool/operation/settings catalogs, Host API permission matrices, effect inventory
schemas and package graphs. Web/TUI client plugins may target only host-defined
slots, have explicit lifecycle/error boundaries, and perform protected work only
through Runtime APIs.

## Explicit non-goals

- no JavaScript proxy service locator in Rust;
- no ambient undeclared service lookup;
- no scope-based authority grants;
- no generic JSON Schema settings;
- no client-recursive tool/operation effects;
- no effect system as a substitute for transport/sandbox isolation;
- no default model-authored arbitrary browser/host code execution;
- no transport reuse when dependency/provider epoch changed.

## Acceptance invariants

1. A plugin cannot initialize before all required providers are active.
2. Missing/disabled/cyclic/version-incompatible/failed dependencies are explicit
   inspectable states.
3. Every contribution has one owner and one idempotent undo.
4. Failed init and normal unload execute the same cleanup path.
5. Shutdown closes admission before waiting; late calls cannot enter.
6. Disposal completion means accepted work and child effects are quiescent.
7. Catalog, lookup, model presentation and dispatch resolve the same scoped view.
8. Scope controls visibility only; Runtime controls authority.
9. Anything model-visible is durable or deterministically reconstructable.
10. Reload restarts only the affected dependent closure unless a broader Runtime
    change is independently required.

## Phase 1.5 implemented: owned scopes and capability overlays

### `PluginEffectScope`

`crates/agena-plugin-host/src/effect_scope.rs` now provides the host-side
lifecycle primitive corresponding to the useful part of a Cordis Fiber:

- explicit `Pending -> Starting -> Active -> Stopping -> Stopped/Failed/Blocked`
  state;
- atomic admission closure and a cancellation token;
- accepted-work leases and truthful quiescence waiting;
- reverse-order asynchronous disposers;
- disposer failure accumulation without abandoning remaining cleanup;
- concurrent and repeated `dispose()` calls sharing one completion;
- child scopes owned as ordinary effects;
- inspectable effect id, kind, label, terminal state and failure;
- no authorization behavior.

Loaded plugins now place transport shutdown and host-resource cleanup under the
same scope. The scope is projected through plugin inspection, including after
normal shutdown. This consolidates order and ownership without weakening the
existing `QuiescentTransport` boundary.

### `ScopedRegistry`

`crates/agena-plugin-host/src/scoped_registry.rs` is the shared visibility and
ownership algorithm used by scoped capability registries:

- a separate global layer plus exact-scope overlays;
- an explicit parent graph with cycle rejection;
- global, then ancestors farthest-to-nearest, then exact scope;
- nearest same-key shadowing;
- duplicate rejection inside one exact layer;
- reads that never allocate or mutate layers;
- generation-safe, idempotent registration tokens and exact owned replacement;
- owner-wide reverse-generation cleanup;
- automatic empty-overlay reclamation;
- identical lookup and visible-catalog resolution.

Every successful registration is bound directly to a `PluginEffectScope`.
If ownership registration races with disposal, the registry entry is rolled
back immediately. Exact replacement releases the old effect handle without
running a stale disposer. Dynamic tools and operations use the same resolver;
session-scoped tools are selected once when a turn's `ToolExecutor` snapshot is
built, so catalog, authorization, identity and dispatch cannot disagree.

The old prototype aliases (`PluginScopedRegistry`,
`SharedPluginScopedRegistry`, and related names) are intentionally removed.
There is one public scoped-registry vocabulary only.

## New architecture invariants

1. A plugin registration that is not owned by a `PluginEffectScope` is a bug.
2. A scope may hide or shadow a capability but may never grant permission.
3. Every exact registry layer rejects duplicate keys; replacement requires an
   explicit remove/replace operation and a new generation token.
4. Reading an absent scope must not create it or change inspection output.
5. `dispose()` completion means admission is closed, accepted work is settled,
   children are disposed and all owned effects were attempted.
6. One failed disposer must not prevent later reverse-order cleanup.
7. Old generation tokens may never remove a replacement registration.
8. Runtime catalog, model-visible presentation, lookup and actual dispatch must
   share the same scope-resolution function.
9. Client code consumes one final `PluginOperationResult`; it never follows a
   plugin-authored invocation chain.
10. Scope/service composition remains below Runtime permissions and transport
    isolation.

## Phase 1.75 implemented: typed pipelines, profiles and architecture catalog

### Typed event composition

`crates/agena-plugin-host/src/event_pipeline.rs` now provides explicit reusable
composition modes:

- ordered failure-contained `Observe`;
- concurrent failure-aggregating `ParallelObserve`;
- ordered first-answer `Bail`;
- strict serial `Transform`;
- nested `Around` middleware with `next`;
- monotonic deny-only `Guard`.

Handlers have stable owner/id/priority/registration metadata and failures retain
the owning plugin. Guard deliberately has no `Allow` variant: an extension can
abstain or deny, while a handler failure fails closed. This is the common
semantic vocabulary for upcoming hook migrations; existing hook dispatchers
must be moved one pipeline at a time, starting with tool execution, so a partial
migration never changes authorization ordering silently.

### Stable-id profile composition

`crates/agena-plugin-host/src/profile.rs` adds an Agena-native authoring layer
that resolves directly into the existing `PluginsConfig`. Ordered layers support
explicit stable-id operations:

- insert and replace;
- remove;
- enable/disable;
- replace or deep-merge plugin config;
- replace package, timeouts or activation dependencies.

Resolution records layer/operation provenance for each plugin and reuses the
same activation planner as PluginHost. A structured profile diff distinguishes
added/removed, package, enabled, config, timeout and activation changes.
`affected_required_closure()` returns the changed plugins plus all transitive
hard dependents across the before/after graphs; soft `after` hints never expand
the restart closure.

The composer is deliberately not a second runtime configuration model. Runtime
continues to consume only the resolved `PluginsConfig`.

### Generated runtime architecture catalog

`PluginHost::architecture_catalog()` projects a safe, deterministic graph of:

- configured plugin nodes;
- enabled/transport/run state;
- activation blockers;
- lifecycle scope state, active leases and owned-effect inventory;
- hard `requires` and soft `after` edges;
- whether each dependency target currently exists and is enabled.

The same catalog is exposed through Runtime and
`GET /api/v1/plugins/architecture`. The Web Plugin Workbench consumes it to show
lifecycle state, dependency edges and owned effects instead of asking users to
reconstruct the graph from logs.

### Delivery status

The concrete migrations described by this study are now part of the runtime:

1. typed Transform / Transform+Bail / Around pipelines are effect-owned and
   deterministic; default plugin hooks share one priority and therefore use
   stable registration order instead of treating activation position as a
   hidden priority;
2. dynamic tools and operations use `ScopedRegistry` atomically across catalog,
   lookup and dispatch, with session scope teardown after `session.end`;
3. transport preparation, manifest inspection, service resolution and
   `meta/init` activation are separate phases with dependency ordering;
4. profile provenance and required dependency/service epochs feed the reload
   closure and explain restart reasons through architecture inspection;
5. contracts, compile-fail probes, API fixtures and Web/TUI consumers exercise
   the same closed settings/operation/service surfaces. Generated tool docs are
   drift-tested against the live bundled manifest catalog.

Future plugin features should extend these primitives rather than create a
parallel compatibility layer or renderer-specific schema.
