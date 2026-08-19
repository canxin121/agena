# Plugin services, scopes, effects, and typed pipelines

This guide describes the Cordis-inspired composition primitives implemented by
Agena's plugin host. They sit below Runtime authorization and transport
isolation; none of them grants permissions.

## Service-driven activation

A configured plugin can export and import stable service seams:

```json
{
  "plugins": {
    "list": {
      "example.storage.sqlite": {
        "package": { "kind": "static" },
        "activation": {
          "services": {
            "exports": [
              { "id": "example.storage", "api_version": 1 }
            ]
          }
        }
      },
      "example.memory": {
        "package": { "kind": "static" },
        "activation": {
          "services": {
            "imports": [
              {
                "id": "example.storage",
                "api_version": 1,
                "optional": false
              }
            ]
          }
        }
      }
    }
  }
}
```

The Host resolves the import before starting either plugin:

- exactly one compatible enabled provider binds automatically;
- multiple compatible providers require an explicit `provider` id;
- a required missing/disabled/incompatible provider blocks the consumer;
- an optional missing/failed provider does not block the consumer;
- required bindings become hard activation edges;
- optional bindings become soft ordering/rebinding hints;
- service cycles participate in the same hard-cycle detector as explicit
  `activation.requires` edges.

Example explicit provider selection:

```json
{
  "id": "example.storage",
  "api_version": 1,
  "provider": "example.storage.sqlite"
}
```

`agena plugin validate` runs the same resolver used during Host construction,
so ambiguity and version errors are reported before transport startup.

## Recursive activation epochs

Every configured plugin receives a deterministic activation epoch. It includes:

- its canonical configuration;
- hard dependency ids;
- the recursively computed epoch of every hard dependency;
- pre-start blocker state for blocked rows.

A provider's provider changing therefore invalidates the complete affected
consumer closure. Optional providers are intentionally excluded from the hard
epoch and remain late-bound. An unchanged external transport can be reused only
when both its own configured row and recursive activation epoch are unchanged.

The epoch is exposed in plugin inspect and the architecture catalog.

## Effect scopes

Every loaded plugin instance owns a `PluginEffectScope`. Agena currently places
at least these effects in the scope:

1. host-owned contributions/resources;
2. the quiescent plugin transport.

Effects are released in reverse acquisition order, so transport admission is
closed and accepted work reaches a terminal state before host contributions are
removed. Scope disposal is shared and idempotent: concurrent callers and early
release handles run a disposer at most once.

Effects can own child scopes:

```rust,ignore
let session_scope = plugin_scope.child("session.42")?;
let registration = session_scope.own(
    "prompt.section",
    "session-specific instructions",
    move || async move {
        remove_prompt_section().await;
        Ok(())
    },
)?;
```

Failed effects remain in the inventory with a safe diagnostic. Normal unload,
failed initialization cleanup, and future differential restart paths must use
this ownership boundary rather than independently retaining/removing registry
entries.

## Scoped registries

`ScopedRegistry<K, V>` provides one global layer plus parent-linked overlays:

```text
runtime generation
  -> workspace
     -> agent preset
        -> session / agent execution
```

Resolution rules:

1. exact scope;
2. nearest parent to farthest parent;
3. global layer.

Materializing all visible entries applies the reverse order: global first,
ancestors farthest-to-nearest, exact scope last. Therefore a nearer same-key
entry shadows a farther one.

Important invariants:

- reads never create an overlay;
- duplicate keys fail only inside the same exact layer;
- registering an entry requires a `PluginEffectScope` owner;
- owner disposal removes the entry automatically;
- empty overlays are reclaimed;
- parent cycles are rejected;
- scope changes visibility/composition, never authority.

This registry is the migration target for tools, prompt sections, operations,
policies, activity providers, and client slots.

## Typed event pipelines

Agena now has explicit live event composition primitives:

| Mode | Semantics |
|---|---|
| `observe` | ordered observation; failures are collected |
| `parallel_observe` | concurrent observation; all handlers are awaited |
| `bail` | first non-empty answer in priority order |
| `transform` | serial value transformation |
| `guard` | monotonic allow/deny; first denial is final |
| `around` | middleware wrapping a terminal dispatch through `next.run()` |

Every handler registration is effect-owned. Priority is deterministic; lower
priority runs first and registration id breaks ties.

Bail/transform pipelines declare whether failures are contained or abort the
pipeline. Guard errors can be contained or fail closed. A guard cannot re-allow
a decision after another guard denied it.

Around example:

```rust,ignore
pipeline.register(&plugin_scope, 10, "metrics", |request, next| async move {
    let started = Instant::now();
    let result = next.run(request).await;
    record_duration(started.elapsed());
    result
})?;
```

These primitives are intended to replace hook-specific ad hoc loops, starting
with tool execution policy/dispatch and then chat/provider/session hooks.

## Architecture catalog

The live catalog is available at:

```text
GET /api/v1/plugins/architecture
```

It contains:

- every configured plugin and current status;
- activation epoch and blocker;
- declared service exports/imports;
- explicit dependency edges;
- required and optional service edges;
- every currently owned effect and terminal state.

Plugin-specific inspect exposes the same service bindings, epoch, blocker, and
effect inventory. The Web Plugin Workbench renders these fields under Overview
and Diagnostics.

## Current boundary and next migration

Service declarations are currently configuration-owned because stdio/HTTP/
cdylib transports still return their immutable manifest during initialization.
The next loader step is a true prepare/activate split:

```text
construct and verify transport
-> attach Host API
-> fetch immutable manifest without initialization
-> resolve manifest service imports/providers
-> activate in graph order
```

Once every transport supports that protocol, service exports/imports should
move into the manifest while the resolver and activation semantics remain
unchanged.

## Ordered profile layers

`PluginsConfig.profiles` applies ordered stable-id patches to `plugins.list`
before any transport starts. The resolved map remains the only Host leaf model:

```json
{
  "plugins": {
    "list": {
      "example.storage.sqlite": {
        "package": { "kind": "static" },
        "activation": {
          "services": {
            "exports": [{ "id": "example.storage", "api_version": 1 }]
          }
        }
      },
      "example.memory": {
        "package": { "kind": "static" },
        "activation": {
          "services": {
            "imports": [{
              "id": "example.storage",
              "api_version": 1
            }]
          }
        }
      }
    },
    "profiles": [
      {
        "id": "workspace",
        "patches": {
          "example.memory": {
            "action": "bind_service_provider",
            "service_id": "example.storage",
            "api_version": 1,
            "provider": "example.storage.sqlite"
          }
        }
      }
    ]
  }
}
```

Available patch actions are:

- `upsert`: insert or completely replace one configured plugin row;
- `remove`: remove one row;
- `set_enabled`: enable or disable one row;
- `merge_config`: recursively merge object config and replace all other values;
- `replace_activation`: replace explicit/service dependencies;
- `bind_service_provider`: select one provider for a declared service import.

Layer ids must be unique. Patches targeting a missing row fail unless they use
`upsert`. The resolver emits final activation state, recursive epochs, per-row
provenance, and a stable added/removed/enabled/disabled/reconfigured diff. Host
construction consumes the resolved tree directly; disabled rows never start a
transport.

## Differential reload plan

The Host stores an explicit reload decision for every plugin row:

```text
add | reuse | restart | remove | disabled | blocked
```

Reasons include configuration changes, recursive dependency epoch changes,
blocker changes, enable/disable transitions, and in-process static-plugin
identity. External transports are reusable only for an unchanged `reuse`
decision. Static plugin instances always restart because they bind to the new
Host/Runtime generation.

Reload decisions are included in `GET /api/v1/plugins/architecture`, alongside
profile provenance and profile changes.

## Operation around middleware

User-facing method operations now cross a real `PluginAroundPipeline` before
transport dispatch. Host components and trusted plugins can register middleware
through `PluginHost::register_operation_middleware`:

```rust,ignore
host.register_operation_middleware(
    "example.metrics",
    10,
    "operation latency",
    |dispatch, next| async move {
        let started = Instant::now();
        let result = next.run(dispatch).await;
        record_duration(started.elapsed());
        result
    },
)?;
```

The middleware preserves `PluginError` rather than stringifying it. Its handler
registration is owned by the registering plugin's effect scope and therefore
cannot survive plugin unload. Tool-backed operations continue through Runtime's
normal permission/tool path; method-backed operations use this Host pipeline.

## Contract hardening added during Cordis/Harness convergence

### Service methods are now a closed RPC catalog

A service export is no longer only `service id + API version`. Every export must
list at least one `PluginServiceMethod`, and each method owns a bounded
`SettingsContract` for both input and output. The Host validates input before
crossing the provider transport and validates provider output before returning
to the consumer. Unknown methods never reach the provider.

The SDK exposes `service_method_for::<Input, Output>("method")` so ordinary Rust
`JsonSchema` types compile through the same constrained contract used by plugin
settings and operations. Explicit machine-oriented escape hatches remain
possible through `PluginServiceMethod::bounded_json`, with byte/depth limits.

This follows Cordis' declared service seams while preserving Agena's stronger
cross-process and permission boundaries: no ambient service lookup and no
stringly-typed unvalidated RPC surface.

### EffectScope is the generation owner

`PluginEffectScope` now provides the single generation-safe ownership boundary
for registrations:

- effect registration is synchronous, so a resource cannot become visible
  without an owner;
- disposal is asynchronous and reverse ordered;
- admission closes before disposal and accepted leases reach quiescence first;
- explicit removal `release()` marks a registration terminal without rerunning
  its disposer, preventing an old registration from deleting a replacement;
- concurrent disposal shares one completion report;
- completed effects leave the live disposal stack permanently and remain
  inspectable as terminal inventory.

### ScopedRegistry and EventPipeline share effect ownership

The operation registry now uses the same exact visibility algorithm for catalog
and lookup: global -> ancestors -> exact scope, nearest shadow wins, reads never
create overlays. Each registration belongs to an exact plugin generation and is
removed by its effect scope.

Typed event pipelines use the same ownership mechanism. `tool.before` is a
Transform+Bail pipeline, `tool.after` is a Transform pipeline, and operation
invocation supports Around middleware. Handler priority/registration order and
owner are exported in the runtime architecture catalog.

### Reload explanations include service binding changes

The loader already refused to reuse a transport when its resolved service
provider epoch changed. The public reload plan now records
`service_binding_changed` as a restart reason as well, so inspect/UI output
matches the actual loader decision rather than reporting a misleading reuse.

Required service bindings participate in that epoch and record the direct
provider ids under `triggered_by`. Optional imports deliberately do not: the
new Host swaps the resolved binding table atomically, so an optional provider
can appear, disappear, or update without restarting the consumer. This is the
Cordis-style late-binding behavior we want for optional capabilities, while a
required service remains a hard activation/lifecycle dependency.

Profile provenance is also field-addressed now. Each effective-tree mutation
records stable changed paths such as `/config/model` or
`/activation/requires`, rather than only saying that a profile performed a
generic patch. Workbench and CLI architecture output can therefore explain
both *which layer* changed a plugin and *what that layer changed*.

### Typed settings are the only structural source of truth

Bundled plugins no longer publish settings by first constructing a second JSON
Schema tree. `#[agena_plugin(settings = Config, settings_default = ...)]`
compiles the Rust `JsonSchema` type directly into Agena's closed
`SettingsContract` AST. Rust type/field docs survive that compilation and type
or field identifiers are humanized for renderers (`McpConfig` → `MCP Config`,
`api_url` → `API URL`).

Plugins that need richer labels use `settings_metadata = ...`. This decorator
runs *after* the typed contract is compiled and accepts contract paths such as
`/fetch/request/timeout_secs`; it can change only `title` and `description`.
Unknown paths fail manifest construction, and the decorator cannot add fields,
change node kinds, defaults, constraints, or validation semantics. Structure
therefore has one owner while presentation remains customizable.

An intentionally empty editable object uses `EmptyPluginSettings`, which is
different from omitting settings entirely. Arbitrary JSON is never inferred
implicitly: a field must opt into `bounded_json_schema`, which compiles to the
explicit bounded JSON node carrying Host-enforced byte and depth limits. LSP
initialization options use this seam; ordinary config remains fully typed.

### Callback context is an ephemeral capability, not plugin-supplied metadata

`HostCallbackContext` still crosses stdio/HTTP so plugin SDKs can echo the
current session/call/workspace/tool context back to Host APIs, but privileged
fields are no longer trusted from the wire. Before every context-bearing
Host→Plugin call, `HostHandle` mints an opaque `authority_token` bound to the
plugin id, exact effect-scope generation, and the complete trusted context.

The token exists only for the originating call. The callback boundary rejects
privileged context without a token, tokens issued to another plugin, modified
context fields, stale plugin generations, and tokens replayed after the call
ends. Native tool streams retain the same authority lease in a Host-owned
terminal-forwarding task until the transport reports stream end; a caller may
stop reading chunks without prematurely revoking the still-running plugin's
context or keeping the authority alive after terminal completion. Streaming
and non-streaming tools therefore have the same context-isolation semantics.
Plugin identity continues to come from transport attribution, never from the
callback body.

### Shared typed service endpoints remove provider/consumer drift

`#[service(...)]` is now available on ordinary plugin methods. The inline form
declares the versioned endpoint next to the handler:

```rust,ignore
#[service("workspace.search", version = 1, method = "query")]
async fn query(&self, input: &SearchRequest) -> Result<SearchResponse> { ... }
```

The macro emits both the manifest `PluginServiceMethod` contract and the typed
`Plugin::service_invoke` dispatcher. Methods targeting the same service/version
are merged into one export; duplicate method ids, missing versions, and invalid
method signatures fail at compile time.

For shared APIs, `PluginServiceEndpoint` is the preferred source of truth. An
API crate declares service id, API version, method id, request type, and response
type exactly once:

```rust,ignore
plugin_service_endpoint! {
    pub SearchQuery {
        service: "workspace.search",
        version: 1,
        method: "query",
        input: SearchRequest,
        output: SearchResponse,
    }
}
```

Providers use `#[service(SearchQuery)]`, consumers use
`PluginServiceClient::endpoint::<SearchQuery>(host)`, and manifests use
`SearchQuery::required_import()` or `optional_import()`. Rust then checks that
the provider handler request/response matches the endpoint while Host still
enforces the generated input/output contracts across every transport. This is
the same contract feeding provider metadata, consumer injection, and runtime
dispatch rather than three repeated string/version declarations. Endpoint API
versions must be positive; zero versions and provider handler type drift are
compile-fail fixtures rather than runtime validation cases.

### Session scope is both visibility and lifetime ownership

Dynamic plugin tools now use the same scoped-overlay semantics as operation
registrations. A tool registered from a Host callback carrying session authority
is owned by the plugin generation in the opaque `session:<id>` layer; a tool
registered during init or another sessionless context remains plugin-global.

The shared `ScopedRegistry` now supports generation-exact `replace_owned` and
`remove_owned`. Updating a registration swaps visibility atomically and releases
the old effect without running its stale disposer, so an old handle cannot later
delete its replacement. Cross-owner replacement is still rejected. Closing a
scope recursively closes all descendant overlays and releases the effect handles
that owned their registrations.

`ToolSessionContext` carries an optional runtime-only session identity.
`SessionExecutionContext` binds the owning Session id after creation/hydration
without persisting it. `ToolExecutor` selects that scope once while building its
catalog and uses the same view for discovery, capability filtering, permission
resolution, identity resolution, and execution.

This deliberately preserves a stable model turn: if tool A registers tool B
while a turn is running, the already-created executor keeps its immutable tool
snapshot. A subsequent turn in the same session sees B; another session and a
global caller do not. Registry change events carry the optional scope and the
plugin-visible `host.tool.list` response filters `last_event` with the same
visibility rule, avoiding a metadata side-channel. Operator architecture
inspection still sees every registration and its explicit scope.

After every `session.end` hook has had a chance to perform plugin-specific
cleanup, Host closes the `session:<id>` scope automatically for dynamic tools and
operations. Plugins therefore cannot accidentally leak a session-temporary
capability until their own unload; the scope lifetime, rather than plugin author
discipline, owns teardown.

`GET /api/v1/plugins/architecture` exposes `tool_registrations` alongside
`operation_registrations`, including key, owner, registration generation, and
global/scoped layer. The Web workbench renders these facts as “Scoped tool
registrations”; inspection does not create or mutate scope layers.

Default `tool.before` / `tool.after` hooks all register at priority `0` and use
the deterministic plugin activation/registration order as their tie-breaker.
Activation index is never converted into a hidden pipeline priority. Explicit
typed pipelines still sort higher numeric priorities first and then by stable
registration id.

### Detached background work must drop foreground authority

A short-lived callback token is intentionally unsuitable for detached work.
The bundled Tasks plugin previously carried its foreground session/call/tool
context into a spawned task; under the hardened authority model that token would
correctly expire when the launching tool call returned.

Tasks now separates foreground and durable state instead of extending the token.
Foreground persistence uses the real current authority. Spawned task fibers run
with plugin identity only; every `RunSubtaskRequest` already carries its parent
session explicitly. Task registry state lives in plugin-private Global storage
under `parent_session_id/task_id` keys, so terminal persistence also needs no
ambient session authority. This keeps background work least-privileged and
generation-bound without introducing replayable long-lived session tokens.

### Public plugin catalogs have stable wire shapes

Architecture and neutral surface catalogs always serialize their collection
fields as arrays, including when empty. Presence of a plugin contribution no
longer changes whether `plugins`, `dependencies`, `effects`, `pipelines`,
`tool_registrations`, `operation_registrations`, `operations`, `display`, or
`themes` exists on the wire. This makes offline contract probes, thin clients,
and alternate renderers deterministic rather than state-shape dependent.

### Removed parallel authoring paths stay removed

The `#[agena_plugin]` macro no longer accepts `settings_builder = ...` or
plugin-level `exports(...)`. The first would recreate a second settings
structure source next to typed settings; the second would recreate a service
manifest catalog separate from the handler that actually dispatches it.
Compile-fail tests preserve these removals. Settings come from `settings = Type`
plus optional defaults/presentation metadata, and provider services come from
method-level `#[service]` handlers (preferably a shared `PluginServiceEndpoint`).
