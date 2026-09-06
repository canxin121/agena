# Development validation

Use the Rust toolchain in `rust-toolchain.toml` and the Bun version in
`.bun-version`. Repository Python tooling requires Python 3.10 or newer; CI
uses Python 3.13. The macOS system Python 3.9 cannot run the refactor tools'
`zip(..., strict=True)` checks.

The default Cargo members cover the terminal application. Use `--workspace`
for a change that needs repository-wide validation:

```sh
cargo fmt --all -- --check
cargo test --workspace --locked --no-fail-fast
cargo clippy --workspace --all-targets --locked -- -D warnings
python3 scripts/ci/verify-runtime-capabilities.py
python3 -m unittest discover -s scripts/refactors/tests
python3 scripts/refactors/check-refactor-invariants.py --manifest scripts/refactors/failure-semantics-invariants.json
```

Validate the frontend from `packages/agena-web`:

```sh
bun install --frozen-lockfile
bun test
bun run build
bun audit
```

`build` includes import-boundary checks, settings translation checks, and Vue
type checking. The `Agena quality` workflow runs these gates on pull requests
and pushes to `master`, with the Rust suite on Linux and macOS. The existing
universal-target workflow covers the larger platform matrix separately; it is
still manually triggered. The quality workflow also runs the plugin host's
optional features independently and together:

```sh
cargo test --locked -p agena-plugin-host --features signing
cargo test --locked -p agena-plugin-host --features wasm
cargo test --locked -p agena-plugin-host --all-features
cargo clippy --locked -p agena-plugin-host --all-targets --all-features -- -D warnings
cargo run --locked -p agena-plugin-host --profile dist --example panic_isolation
```

The last command is a real executable, because Cargo's test harness forces
unwinding even when the selected profile says `panic = "abort"`. Both the
release and dist profiles must preserve the in-process plugin host's panic
isolation. The fixture shared with the ordinary integration test exercises a
synchronous panic, a panic after an async yield, and successful calls afterward.

With `signing`, a stdio plugin's SHA256 applies to the executable resolved from
its configured PATH and working directory. The host retains that absolute path
and verifies its contents before every supervised restart. The Unix process
fixtures require `/usr/bin/python3`; they create their own scripts and working
directories, and do not modify the parent process's environment.

## Stdio process supervision

Run the real subprocess regressions independently when changing the stdio
transport:

```sh
cargo test --locked -p agena-plugin-host --test stdio_lifecycle
cargo test --locked -p agena-plugin-host --test stdio_host
```

`never` leaves every exit stopped or failed. `on-failure` restarts after a
nonzero/signal exit or a protocol/pipe failure, while a clean exit remains
stopped. `always` also restarts clean exits. All restarts retain the configured
retry budget and backoff, and require successful cleanup of the old child and
its tasks. Closing cancels a pending backoff.

The supervisor owns the managed process tree and observes OS exit separately
from stdout EOF or stdin failure. A **1-second** grace handles EOF arriving
before exit and drains complete response frames after exit; a descendant that
holds stdout open cannot hide the main process's exit indefinitely. Cleanup
bounds process reaping to **3 seconds**, then task shutdown to **1 second**.
Each generation cancels and joins its pipe and host callback tasks. Request
registration and callback responses bind to that generation, so a replacement
process cannot receive an old request accidentally. Repeated close calls retain
the cleanup outcome, including failures.

Stream registration replays early events under the same lock used for normal
delivery. Up to **64 early chunks per stream** are retained, with a separate
terminal result; overflow remains a failure even if the plugin later reports
success. Duplicate active stream IDs are rejected. Stderr uses segments of at
most **16 KiB of raw bytes**, replaces invalid UTF-8 for display, preserves
complete code points across segments, and marks continuation in log fields.
Lossy UTF-8 conversion can expand a stored segment to three times that size.

The full-host fixture verifies application initialization as well as pipe
recovery. A hosted restart requests `meta/manifest` again and checks it against
the validated immutable manifest, then runs `meta/init` with a newly issued
callback authority. Each request has a **30-second** deadline. Init must return
the expected manifest and protocol version before ordinary calls and
notifications are admitted and status returns to `Running`. Failed
initialization consumes the restart attempt budget. `restart_count` counts
successful restarts that reached `Running`, not failed initialization attempts.

A reused live stdio process retains its current initialization. `can_reuse`
checks whether it is ready and still belongs to the supplied predecessor scope.
This is a planning hint: `try_rebind_host` repeats the checks under spawn
serialization before beginning the transfer. A stopped, failed, closed, or
no-longer-owned candidate is replaced with a freshly initialized transport.
Replacing a provider instance also restarts its direct and transitive hard
dependency consumers, while independent eligible plugins remain reused.
Preparation after planning must still match the validated manifest; a change
blocks that activation, and the next explicit reload can parse it afresh.

Successful handoff transfers the callback router, status registry, process-owned registrations and
future initialization inputs together under spawn serialization. The next
restart uses the successor host's version, workspace and settings. Global
dynamic tools, session tool overlays, display contributions and themes are
transferred to a new child effect scope; a process exit disposes that child's
registrations. Manifest defaults are installed before init or handoff so they
do not overwrite intentional dynamic definitions. The predecessor's shutdown
disposes its old logical resources while leaving a successfully transferred
transport to the successor.

Handoff stops admission while it cancels old callbacks and transfers resources.
Final publication checks that the process and transport remain live, so a
concurrent close cannot publish a successful binding.
Cancellation or failure during handoff terminates the partially bound process.
Cancelling a whole host build also closes transports that it already received
from the predecessor and records a failure there. This is not a transaction
that restores the old plugin set; full reload rollback and non-stdio
contribution/credential transfer remain
tracked in `docs/technical-debt-audit.md`.

`PluginEffectScope::dispose` is called on an `Arc<PluginEffectScope>`. Cleanup
runs independently of individual waiters and shares one report. A synchronous
or asynchronous disposer panic is recorded as a cleanup failure while the
remaining disposers continue. Cleanup requires a running Tokio executor; it
does not impose a deadline on arbitrary user-provided disposers.

Global tool, display, and theme registration acquires an owner lease and admits
its disposer before publishing registry changes. Each value retains an exact
registration identity and a weak reference to its owning effect scope. The
registry lock serializes the value and ownership change; replacement or explicit
removal retires the actual previous owner's effect handle. An already-running
disposer checks the registration identity under that same registry lock before
removing anything. A rejected registration preserves both the visible value and
its existing cleanup. A closed logical effect scope is not implicitly recreated
by registration.

Global keys remain namespaced by plugin and use the latest registration. Removing
that registration leaves the key absent until another registration or manifest
restore. Manifest defaults have individual logical-scope ownership; initialization
overrides retain their process ownership, and supervised restart assigns restored
defaults to the logical scope. Handoff snapshots select values currently owned by
the exact process scope, including global tools, display and themes. A cloned
`PluginToolRegistry` is an independent value snapshot whose mutations cannot
release effects in the live registry. Tool registration and removal listeners run
after the registry write lock is released.

Run the ownership and handoff regressions with:

```sh
cargo test --locked -p agena-plugin-host --lib host::process_scope::ownership_tests
```

`ScopedRegistry` also serializes owner admission, value publication, and retirement
of the old cleanup handle under its registry lock. Every visible entry already
contains its effect handle. Duplicate registrations, closed owners, and exhausted
registry generations return errors while preserving entries, scope metadata, and
the next generation. Replacements and explicit owned removals require the exact
same effect scope, including when two scopes belong to the same plugin. This
preserves scoped duplicate isolation and ancestor fallback behavior.

Registration generations advance with checked arithmetic; exhaustion returns
`ScopedRegistryError::GenerationExhausted` before publication. Parent-cycle
validation also precedes metadata changes. `clear_scope_tree` atomically removes
the current subtree; later registrations can recreate a cleared layer. Test the
registry's behavior, including concurrent mutation and cleanup, with:

```sh
cargo test --locked -p agena-plugin-host --lib scoped_registry::
```

Hook catalogs and transport routing bindings use the same exact registration
ownership as display and theme contributions. Both registration APIs return a
`Result`; a closed owner returns `HostUnavailable` and preserves the previous
binding. The build publishes activation only after manifest ownership, hooks,
and transport registration all succeed. Transport values are cloned out of the
registry before awaiting plugin code; removing a binding does not itself close
the transport.

Concurrent first use of a plugin shares one logical scope, and registration does
not recreate a closed scope. Fallback resource cleanup acquires both asynchronous
metadata locks before checking the current scope's exact identity. It holds that
generation read lock through the remaining synchronous removals, preventing a
successor from being published midway through cleanup. Cancelling while waiting
for those locks leaves fallback metadata intact and permits a later retry. Scope
disposal precedes fallback, so this does not restore a running plugin after
cancellation or make the entire shutdown atomic. Run the regressions with:

```sh
cargo test --locked -p agena-plugin-host --lib host::host_handle::lifecycle_tests
```

Every `ScopedHostClient` method now admits the callback and acquires a lease
against its exact current scope under the generation read lock. This includes
local catalog operations and logging, as well as calls forwarded to the runtime.
Quiesce closes admission even while the lifecycle state still reads `Active`.
Stopped or stale callbacks return `HostUnavailable`; the void logging method
silently discards them. A lease covers both host-client lock waits and the host
operation, and scope cancellation drops the callback future before releasing
that lease. A callback that requests its own disposal can therefore be cancelled
without waiting indefinitely for its own lease.

Admitted callbacks retain their original registration owner through a task-local
binding. The binding identifies both the host allocation and the effect scope,
so work on another host with the same plugin id uses that host's owner. Stdio
callbacks and process-contribution handoff use the same binding while retaining
their process child ownership. Test these boundaries with:

```sh
cargo test --locked -p agena-plugin-host --lib host::host_scoped_client::lifecycle_tests
```

The shared callback entry also validates privileged context before any local
catalog operation or runtime forwarding. Session/call/workspace/tool fields must
match a live authority issued to that plugin generation; forged, altered, expired,
or cross-plugin authority returns `PolicyDenied` before changing catalogs, events,
notifications, or effects. Plugin-only global calls remain valid, with identity
attributed by the scoped client.

Use the SDK's `run_in_isolated_host_callback_context` for independent requests and
fully validated or newly issued authority. It installs exactly the supplied
fields, so missing fields cannot inherit privileges from an enclosing operation.
The existing `run_in_host_callback_context` intentionally patches the enclosing
context and remains available for internal inheritance. The host's authorized
calls, stream invocation branches, scoped callback entry, and raw RPC entry use
the isolated form. Raw RPC rejects malformed non-null context as `InvalidParams`,
including arrays and incorrectly typed fields. Missing, null, and empty-object
context retain plugin-only semantics. Run these regressions with:

```sh
cargo test --locked -p agena-plugin-host --lib host::host_scoped_client::authority_tests
```

The SDK HTTP driver treats each incoming request as an independent context,
including requests that omit `Request.context`. A streaming invocation retains
the incoming plugin identity and authority token for the plugin task and every
chunk, success, error, or lost-terminal callback. Its session/call/workspace/tool
fields continue to come from `ToolInvokeInput`. Actual loopback HTTP tests cover
all terminal paths, concurrent streams, and enclosing middleware context:

```sh
cargo test --locked -p agena-plugin-sdk --features http --lib drivers::http::stream_context_tests
```

These SDK tests inspect emitted packets and bearer headers. The host stream
entry separately validates the live authority before dispatching to a transport,
and HTTP ingress requires an object-shaped context. The HTTP transport binds
each stream to the complete context of its originating invocation. A valid
authority from another call cannot append chunks or finish that stream.

HTTP stream invocation reserves pending state before sending the request. Its
guard removes early events synchronously on cancellation, timeout, or response
failure. Only a matching pending invocation can reserve a new remote stream ID;
the response must agree with that ID, and duplicate IDs preserve the existing
consumer. Registration, early replay, and delivery share one state lock. There
are at most **128 pending invocations**, each retaining **64 early chunks** and
an independent terminal slot. A full valid chunk buffer survives an end marker;
overflow remains an error even if later events report success. Closed or unknown
streams cannot create orphan buffers. Active-stream monitors check their exact
channel identity before removing a registration. Run the registry/authority and
actual SDK HTTP/runtime-bridge regressions with:

```sh
cargo test --locked -p agena-plugin-host --lib -- host::host_handle::stream_authority_tests:: transport::http::streams::tests::
cargo test --locked -p agena-runtime-plugins --lib plugin_runtime_service::http_stream_tests
```

The HTTP fixture uses the real bridge behind a test Axum route; it does not test
the application's `ServerError` envelope. It includes 128 cancelled initial
requests followed by a successful stream, and forces callbacks to arrive before
the initial HTTP response without production hooks or sleeps.

HTTP callback credentials now identify an exact host and logical effect scope.
Admission rechecks the credential and current scope together, and holds a scope
lease across the inner-client lock wait and callback operation. Cancellation
drops the callback future before owned cleanup runs. Dynamic registrations retain
the admitted owner if a successor scope is installed during the callback. A weak
route registry shared by one reload lineage lets the published bridge route a
candidate credential to its issuing host before that candidate is published.
Independent runtime lineages cannot route each other's credentials; dropping a
host removes its routes.

Retained SDK HTTP plugins use a stable `HostClient` proxy. The HTTP control method
`meta/host.rebind` compares `expected` with the current callback binding and
atomically replaces the URL/token pair with `next`, without repeating plugin
initialization. Both bindings must be JSON objects and unknown fields are
rejected. Enabling and disabling callbacks switches between the remote client
and the supplied local fallback, whose full HostClient API is forwarded. The
control operation does not consume an ordinary dispatch slot, so 64 occupied
slots cannot starve handoff. This is an HTTP extension to protocol version 2;
older servers that lack it cannot perform live handoff.

Fresh HTTP transports also fence remote initialization. Before `meta/init`, the
host requires `meta/http.state` with HTTP instance extension `version: 1` and an
optional opaque `revision`. Global RPC protocol version remains **2**. A server
without this extension, with an incompatible version, or with a mismatched
response ID is rejected before initialization can retire its running instance.
Each transport sends a unique `x-agena-plugin-instance` owner. Initialization
also sends `x-agena-expected-revision`, using `-` when no instance has existed.
The SDK compares that revision under its lifecycle lock. Concurrent candidates
that observed the same revision have one winner. Instance headers must contain
one value of 1–128 ASCII letters, digits, `-`, `_`, or `.`; they are lifecycle
identifiers and do not replace HTTP endpoint authentication.

`http::router` accepts a factory that returns a fresh Rust plugin object:
`router(MyPlugin::default, host)` replaces `router(MyPlugin::default(), host)`.
Captured construction inputs can use `router(move || MyPlugin::new(inputs.clone()), host)`.
The factory runs once to obtain the manifest and first object, then for each
subsequent initialization. `export_http!` evaluates its plugin expression inside
that factory, including the entrypoint generated by `#[agena_plugin(export = http)]`.
Constructors must provide fresh instance state and an identical manifest; share
only resources whose lifetime intentionally spans instances. Keep constructors
brief and put asynchronous setup in `Plugin::init`.

Each remote instance owns its dispatcher, callback client and HostClient proxy.
A new object gives macro-generated PluginSettings a new OnceLock, so settings
reload and retry after failed initialization do not repeat writes to the old
object's settings. Factory panics and changed manifests are rejected before
retiring the predecessor. The SDK validates the returned InitOutcome manifest
and protocol before admitting calls; an incompatible outcome enters cleanup.
Successful shutdown releases the dispatcher and object while retaining the
owner/revision needed to reject stale requests and make repeated shutdown a
no-op. Rebind preserves the current object and settings without calling the
factory or initializing again.

The SDK validates initialization object shape, RPC version and callback binding
before retiring the predecessor. InitContext rejects unknown fields, including
the removed `config` field. Initial bindings and rebinds require HTTP(S) URLs;
a token without a URL, an empty token or an invalid Authorization header is
rejected. Ordinary calls, shutdown and rebind require the current instance
owner. A delayed shutdown from an older transport cannot stop its successor.
Headerless SDK callers remain usable only while the current instance is also
headerless; they have no fencing between headerless generations.

Remote lifecycle operations have an SDK **30-second** deadline covering lock
waits, cleanup and plugin code, independently of the host's request timeout.
They bypass the 64 ordinary dispatch permits. Retirement cancels admitted work
and waits for the actual stream plugin tasks to drop before calling shutdown.
Each new initialization receives a new HostClient proxy; old retained proxies
are stopped, while rebind keeps the current instance's proxy stable. Cancelled
or failed init/shutdown retains cleanup responsibility and schedules one
serialized cleanup attempt with its own **30-second** deadline, including its
lock wait. A successor must complete any remaining cleanup before initializing.
Failure keeps the instance stopped; it does not restore the predecessor.
Both outbound plugin RPC and SDK host callbacks validate the response ID.
These asynchronous deadlines require cooperative execution. They cannot
preempt blocking Rust constructors, destructors or arbitrary detached work.

The host checks remote support and ownership before quiescing the predecessor.
Handoff has a 30-second deadline and cancels old callback/outbound request
futures, interrupts streams, and transfers owned tools, session overrides,
display contributions and themes to the successor. Failure or cancellation
after transfer begins closes local admission and retires partial bindings.
It does not restore the disposed predecessor. A terminal stream error bypasses
an unread full chunk channel in the lifecycle wrapper; successful streams still
deliver all queued chunks before completion. Run the added regressions with:

```sh
cargo test --locked -p agena-plugin-host --lib host::callback_rpc::tests
cargo test --locked -p agena-plugin-sdk --features http --lib drivers::http::handoff_tests
cargo test --locked -p agena-plugin-sdk --features http --lib drivers::http::instance::tests
cargo test --locked -p agena-plugin-sdk --features http --test http_plugin_lifecycle
cargo test --locked -p agena-runtime-plugins --lib plugin_runtime_service::http_handoff_tests
```

Production runtime composition now starts a dedicated loopback callback listener
before initializing plugins. Runtime snapshots in one reload lineage share its
lifetime. A preconstructed `PluginCallbackDispatcher` uses the same weak,
authenticated host/scope registry as the existing bridge, so first-init and
unpublished candidate callbacks do not depend on the application's later public
listener. Callback bearers authenticate independently of UI sessions. Missing,
incorrect or UI-session credentials receive JSON-RPC 401 responses; authenticated
requests without callback context receive JSON-RPC 400 responses, preserving the
request ID. Malformed HTTP/body extractor errors remain a separate review area.

Each candidate HostClient captures its complete ConfigResolution envelope before
init. `read_config` therefore sees that candidate during bootstrap, restart and
rebind. Runtime-dependent methods report HostUnavailable until publication;
a weak binding and a generation check become ready with the snapshot swap.
After publication they use live process services, rather than retaining an
immutable service graph for every retired generation. Runtime clones share one
internal owner, so weak clients survive replacement of an outer Arc while any
ordinary runtime handle remains. The tool-registry listener emits synchronously
through a weak LiveSignalHub and does not retain the runtime.

Snapshot cleanup now covers every service build step after PluginHost returns.
Failed/cancelled provider composition shuts down initialized plugins and releases
an unpublished listener. The process-visible host slot updates only after a
completed snapshot is published. Maintenance loops still require normal runtime
shutdown; active operations and returned/detached resources have separate
lifetimes. The ownership regression explicitly stops maintenance, shuts down the
plugin host and releases runtime handles while retaining its callback adapters.

Run real runtime/application regressions with:

```sh
cargo test --locked -p agena-runtime --lib runtime::callback_tests
cargo test --locked -p agena-runtime --lib reload_shutdown_tests
cargo test --locked -p agena --test http_plugin_callbacks
```

These tests use temporary configurations, in-memory databases or isolated state
directories, and ephemeral loopback listeners. They exercise first-init config,
unchanged-plugin rebind, changed-plugin restart before publication, explicit
service readiness, build failure/cancellation cleanup, runtime release, and an
actual UI-password-protected application bootstrap and management-API reload.
The SDK macro lifecycle fixtures separately cover factory-created PluginSettings
objects. SDK/global protocol and instance-extension versions are unchanged.

The advertised callback URL currently uses 127.0.0.1; remote HTTP plugins need a
separate bind/advertised-address design. The previous public `/plugin-rpc` route
retains its UI middleware and REST error mapping; SDK initialization uses the
new dedicated endpoint. Whole-snapshot rollback remains open: handoff can retire
an old plugin before a later service fails. SessionManager reconfiguration is
prepared privately and applied only at admitted publication, but a failed
successor does not restore its retired predecessor.

Plugins request a runtime reload through `HostClient::request_config_reload()`
(`host/config.reload.request`) and query it later through
`HostClient::config_reload_status(HostConfigReloadStatusRequest { task_id })`
(`host/config.reload.status`). These are additive optional host capabilities;
the global RPC protocol remains 2 and the HTTP instance extension remains 1.
The HTTP callback client, stdio callback client, stable HTTP proxy and scoped
HostClient all forward the two methods. The proxy and scoped client now cover
all **60 HostClient methods**.

Acceptance has this JSON shape:

```json
{"task_id":"rtask_...","started":true}
```

A later status response has a separate shape:

```json
{"task_id":"rtask_...","state":{"status":"succeeded"}}
```

The states are `running`, `succeeded`, `failed` (with a `problem`), and `cancelled`.
`running` includes waiting for the originating call to finish. Requests/results
reject unknown fields; zero-field states use empty struct variants so Serde also
rejects extra fields there. Unknown or expired task IDs return an error. History
is bounded; missing history never implies success.

The host binds a completion signal to its originating request-response call.
Admitted callbacks restore it from the validated authority record; nested calls
share it, and a native stream keeps it until terminal result forwarding. Hooks
without session/tool context get a neutral authority with no added privileges.
Background callbacks without a host invocation get their own callback-lifetime
signal. The signal retains no plugin, host or runtime. Returning, dropping or
cancelling the origin releases it; the accepted runtime task runs outside plugin
callback task-local contexts. The shutdown regression accepts a request first,
then verifies that runtime shutdown cancels it while its origin remains pending.
Registry admission and shutdown now share one lock, with deterministic and
concurrent admission regressions. Direct reload also observes shutdown while
waiting for ReloadGate and composing its candidate. Final synchronous snapshot,
session configuration and process-host publication shares TaskControl's lifecycle
mutex with shutdown: shutdown first rejects publication; publication already
admitted finishes before shutdown returns.

Return from the originating call after acceptance. Polling until success inside
that call would wait on its own completion barrier. Repeated requests from the
same origin return the same active task ID with `started: false`; different
origins can queue separate serialized reloads within the shared active-task cap.
An independent reload can still interrupt an active call. Accepted work survives
cancellation of its origin;
completed writes are not undone. One-way notification delivery and arbitrary
remote/detached work have separate lifetimes.

The synchronous SDK `reload_config()` remains optional for other hosts. Agena
Runtime rejects it before mutation, with guidance to use request/status. Bundled
settings writes now return `reload_task` and the text `Runtime reload queued.`;
they leave the completed `reload` report unset. A later build failure is recorded
as `failed`, without fabricating a completed-generation result or restoring an
already retired predecessor.

Runtime admits at most **64 active managed background tasks** across plugin
reloads, catalog refreshes and marketplace work. Waiting tasks consume a slot.
Active deduplication runs before capacity rejection, so a duplicate can return
the existing task at capacity. Shutdown closes admission even for duplicates.
The history trimming target remains **64 total records**, preserving running
tasks; it is not an additional allowance of 64 terminal records. An expired
record remains an error when queried.

Registration and closing registry admission share one mutex. Missing executor
and capacity failures occur before a task record is created. Already-cancelled
work is checked before even invoking its factory. Factory/future panic records
a scrubbed `Failed` task, while abort or executor teardown finalizes it as
`Cancelled`; terminal state, slot release and dedupe cleanup happen together.
Observers run outside the mutex and start/finish callback panics are isolated.
Maintenance `TaskControl` also rejects new work after shutdown. A typed
`AppError::Cancelled` returned by the work branch remains `Cancelled`, even when
shutdown arrives after the registry polled its cancellation branch in that same
turn. Ordinary errors remain `Failed`; catalog cancellation is not recorded as
a refresh failure.

Valid typed RPC errors preserve their failure identity, kind, public problem,
retry/recovery and diagnostics through SDK HTTP/stdio callbacks and host
transport decoding. Incompatible or untyped RPC data remains a generic error,
with raw data retained only as diagnostics. Plugin reload overload returns
`HostUnavailable`, with `Backoff` retry and `Retry` recovery.

`ModelCatalogRuntimeService::start_model_catalog_refresh` now returns
`RuntimeBackgroundTaskControlError` directly; the former message-only
`ModelCatalogRefreshError` is removed. Application owns the shared task-control
error projection. REST reload/catalog/marketplace admission maps shutdown,
capacity and missing executor to **503**; unknown cancellation IDs map to
**404**, and terminal or noncancellable tasks to **409**. Actual router tests
verify reload/catalog capacity and shutdown responses retain the shared
`problem` envelope and `Backoff` semantics.

Direct reload uses `RuntimeControlServiceError::Shutdown` to preserve the
stopped-runtime failure through Application, REST settings and Provider Studio
context wrappers. It projects to `DependencyUnavailable`, HTTP **503**,
`Backoff` and `Retry`. Other operation failures retain their diagnostics and
internal-failure classification. Settings edits happen before reload: a rejected
reload does not undo the saved file or create a completed reload report.

A candidate built against an existing SessionManager owns a prepared replacement
configuration. Only final admitted publication applies it to the shared manager.
Dropping a candidate releases the prepared providers, processor and executor;
it does not install its PluginHost through live sessions. Runtime callback
fixtures disable automatic maintenance directly, without shutting down the
runtime and then expecting reload to remain available. Production construction
always enables automatic maintenance.

If a reload has already published its snapshot, failure to enqueue the optional
model-catalog refresh is recorded in catalog `last_failure`. The completed
reload remains successful. This does not claim that the refresh ran; an explicit
refresh or a subsequent reload can retry it. There is no deferred automatic retry.

Runtime shutdown closes managed background admission and orders its first
TaskControl transition against the synchronous publication mutex before selecting
the session notification host. Only that first caller schedules active
`session.end` notifications. Repeated calls and concurrent first calls do not
schedule more broadcasts. If no Tokio executor is available, the first call
still closes control and emits one diagnostic; later shutdown calls do not retry
notification scheduling.

`SessionManager::broadcast_active_session_end(...)` constructs an owned future
synchronously. It captures the current PluginHost and execution registry before
queueing, then reads active session IDs asynchronously and uses the captured host
throughout that batch. Reconfiguration before the first poll or while a previous
notification waits cannot switch its host. The existing `.await` call form works;
the future does not need to retain the entire SessionManager.

Shutdown remains synchronous and does not await the detached notification batch.
Active membership is read when that task runs, and the existing five-second
per-plugin deadline is not a total budget for all sessions. One-way notification
status does not provide a durable acknowledgement from arbitrary remote plugin
work. This also does not close or cancel every retained SessionManager execution
API. Tests explicitly cancel their held session executions during teardown.
Notification drain ownership, session execution admission/cancellation after
shutdown, retained-generation delivery and total resource budgets remain open.

The notification regressions create persisted sessions in in-memory SQLite and
hold real registered execution in a local SDK HTTP prompt hook before any model
request. They cover repeated and 16-thread concurrent shutdown, no-executor
diagnostic order, the real lifecycle mutex, and reconfiguration before/within a
notification batch. Run them with:

```sh
cargo test --locked -p agena-runtime --lib shutdown_notification_tests
cargo test --locked -p agena-runtime-session
```

The actual application HTTP callback fixture waits for the accepted REST reload
by task ID via `/api/v1/runtime/tasks`, then asserts terminal success before the
next SDK reload. Candidate configuration and callback credentials may be visible
before publication binds runtime services, so neither config visibility nor old
token rejection is a completion barrier. A failed or cancelled task fails the
test immediately; the mutation is not retried to hide readiness errors.

Provider client identity belongs to a runtime snapshot and the adapters or
authentication handles constructed from it. Each snapshot owns an immutable
`ProviderClientIdentity` resolved from `ProviderClientVersions`. Candidates use
their own identity before publication; retained provider registries keep their
identity after a replacement is published. Failed or cancelled candidates cannot
change the User-Agent of live requests or another Runtime. Saved model discovery
retains one snapshot for adapter configuration, network settings and identity.

Provider construction passes the identity explicitly, including MCP/LSP setup,
OpenAI/Anthropic/Gemini adapters and nested GitLab providers. Internal
`AdapterBuildContext` groups the HTTP client, environment, config path and
identity. `AuthManager` and `ManagedCredential` accept `.with_client_identity(...)`
and preserve the selected identity through OAuth refresh. OAuth applies identity
per request on a shared connection pool. Explicit defaults support standalone
construction; production Runtime composition supplies its resolved identity.
Rust callers of the removed global setter or no-argument User-Agent helpers must
retain and pass the selected identity instead. No config field or wire schema
changes are required. Plugin `InitContext.agena_version` is the actual Agena
package build version and is independent of configured provider CLI versions.

The identity regressions issue local HTTP discovery requests for unpublished,
failed and cancelled candidates, candidate/retained registries, and five adapter
protocols across two simultaneous runtimes. Separate checks verify Codex query
and header agreement, Gemini request-model updates and custom User-Agent
preservation, and built OAuth requests. They do not execute remote OAuth grants
or freeze credential-store contents. Multiple runtimes still share the weak
process PluginHost slot; explicit identity does not make that slot independent.

```sh
cargo test --locked -p agena-runtime --lib client_identity_tests
cargo test --locked -p agena-runtime-provider --lib client_identity_tests
cargo test --locked -p agena-runtime-provider-adapters --lib client_identity_tests
```

The cap bounds task count, not bytes, per-origin fairness or maintenance capacity
reservations. Synchronous observers can still block indefinitely, and cancelling
a task cannot undo completed writes/publication or stop arbitrary blocking,
detached or remote work. The publication critical section contains no await and
must not reenter TaskControl. Whole-snapshot rollback, atomic reads across the
separate snapshot/session/process and credential stores, generation exhaustion,
and complete session/notification shutdown remain review boundaries.

Focused task admission and API checks:

```sh
cargo test --locked -p agena-runtime --lib background_
cargo test --locked -p agena-runtime --lib runtime::callback_tests
cargo test --locked -p agena-api-server --lib background_task
cargo test --locked -p agena-api-server --no-default-features
cargo test --locked -p agena-api-server --no-default-features --features http
```

Router tests follow their HTTP/WS/SSE feature requirements; the standalone
background-task HTTP tests also run in the HTTP-only build. The actual callback
application fixture disables public catalog fetches only in its isolated child
environment, so external startup maintenance cannot rotate its captured initial
bearer before its deliberate auth/reload checks. It preserves child logs on
failure. This does not disable catalog refreshes in the product.

Focused lifecycle/protocol checks also include:

```sh
cargo test --locked -p agena-plugin-host --lib host::call_completion::tests
cargo test --locked -p agena-plugin-sdk --features stdio --lib drivers::stdio::reload_callback_tests
```

The stdio check exercises the in-memory callback queue and response codec, not
a real stdio child process self-reload. Runtime regressions exercise real HTTP
tools and native streams, direct/chained hooks, nested HTTP-to-bundled-settings
calls, changed-settings object replacement, origin cancellation, shutdown while
waiting, malformed input rejection and actual failed task status. The actual
UI-password-protected application test also submits and queries a reload through
the SDK callback client independently of its management UI session.

Shutdown implementations must tolerate the previously documented cleanup retry.

Cancellation does not undo completed writes or automatically cancel detached or
blocking work in a remote plugin. Returned resource lifetimes, stream byte
budgets, catalog event ordering across registries, scope/effect history growth,
and whole-host rollback remain under review.

Graceful transport shutdown waits for accepted work. If it fails or exceeds
**10 seconds**, the loader attempts forced close for up to **5 seconds**.
Forced close reaches the underlying transport before waiting for wrapper calls
to settle, so pending stdio requests cannot prevent child termination. Repeated
close calls retain the cleanup result, including failure.

## Scheduler durability

Run the scheduler independently as well as in the workspace. This ensures its
process recovery tests declare their own Tokio features instead of borrowing
features enabled by another workspace member:

```sh
cargo test --locked -p agena-scheduler
```

`Scheduler` and `JobStore` persistence operations return `SchedulerResult`.
An absent job, a concurrent edit, and a failed database operation are separate
outcomes. `put` inserts new IDs only. Updates compare a read snapshot containing
the exact persisted JSON and claim owner, so prompt-only changes and pauses
invalidate stale writes. Completion merges against the current job and commits
its audit record in the same transaction. Do not append that record separately.

The existing schema version remains **1**. The `delivery_key` column identifies
one worker attempt; the stable business idempotency key remains in
`pending_delivery`. `claimed_at_ms` holds the last lease renewal, while the
attempt's original start time remains in the JSON. Workers renew every **30
seconds** and abandoned claims become eligible after **90 seconds**, followed
by the next poll. Jobs are claimed immediately before delivery, so a slow sink
does not consume leases for the rest of a due batch. A lost lease drops the
local delivery future; a later worker retries using the same business key.

This is **at-least-once** delivery. An external effect accepted before a crash
or cancellation cannot be undone by dropping a future. Sinks must deduplicate
using `JobDeliveryAttempt.delivery_key`; the runtime session sink already has
durable support for that key. Lease comparisons use the host wall clock. A
large clock adjustment or a process suspension exceeding the lease can cause
recovery, so neither leases nor a green test suite establish exactly-once
external effects. Pausing keeps an in-flight occurrence; it finishes under the
latest configuration while preventing later occurrences.

Tests cover SQLite write failures, malformed stored data, competing connections,
transaction rollback, pauses/edits during delivery, renewal under writer-lock
contention, and a subprocess exiting immediately after a durable claim. That
subprocess uses only a temporary database; it does not touch a running Agena
instance or the user's scheduler database.

## SQLite schema and failure invariants

Run the storage adapter independently when changing persistence:

```sh
cargo test --locked -p agena-storage-sqlite
```

The main store retains table-layout version **13**. `initialize_schema` creates
only an empty version-zero database. A current version marker also requires
the actual tables and required indexes to match Agena's canonical declarations;
missing objects, unsupported columns/constraints, and unexpected Agena tables
are rejected. Incompatible versions and structures are rejected before this
function changes connection pragmas. This does not add table/column migrations.

Compatible invariant-trigger corrections are applied to existing current
databases as well as fresh ones. Initialization holds the schema file lock,
takes the SQLite write lock, revalidates the layout, and replaces only changed
or missing known triggers in one transaction. An unchanged set is left alone,
so normal reopens do not invalidate the schema on every startup. The regression
fixtures retain the historical version-13 failure triggers and exercise both
reopening and concurrent connections to a temporary file database.

Creation failures require string `id`, `code`, and `user.fallback` fields.
Missing JSON properties return SQL NULL; use NULL-aware comparisons in trigger
conditions so those values cannot bypass validation. Subtask insertion and
update both enforce a non-null status and the corresponding start/finish/failure
shape. Invalid writes leave the old record and optimistic version untouched.
These are minimum database invariants; they do not validate every field of the
application `Failure` type or scan/repair already persisted business data.

Part content must be valid JSON on both insertion and content-only updates.
Optional `provider_state` must be JSON or SQL NULL on both paths. Run markers
require a string `run_kind`; its values remain open for extensions. An optional
`abort_reason` must be a string or JSON null, completed markers must include
it, and failed/cancelled markers require a string. Updating either `state` or
`content` checks that shape. Duplicate top-level `run_kind`/`abort_reason` keys
are rejected: SQLite selects the first duplicate whereas serde_json selects
the last. Updating lifecycle timestamps also enforces the insertion bounds.
The historical part triggers are retained in `schema_invariants/fixtures/`
and exercised through the normal version-13 refresh path; table/index
definitions and existing rows are not rewritten.

The two storage backends share `prepare_part_update` and
`prepare_run_completion`. They validate a complete candidate before publishing
it, so an invalid text delta cannot leave an in-memory state transition behind.
Run creation/completion also validates the control fields; errors do not advance
part or session versions in the tested paths. Completion returns the revision
and timestamps actually committed. Backwards update times, invalid finish times,
and revision overflow return errors. SQLite passes its loaded row by value;
the memory backend stages a clone so validation cannot mutate the stored part.

These checks cover JSON syntax and the stated control/timestamp fields, not a
complete typed decoder for every part kind, a scan of historical corruption,
or every in-memory mutation's transaction boundary. Full state-transition and
repeated/conflicting completion semantics remain separate audit work.

The runtime configures WAL and `synchronous = NORMAL` on each file-backed pool
connection. NORMAL preserves consistency but can lose recent, unsynced commits
on a power failure; it does not provide the power-loss durability of FULL.

`run_transaction_effects` and `run_transaction_app_effects` preserve the
operation's original error even when explicit rollback also fails. SQLite can
already have rolled back a transaction after a trigger or I/O error. Cleanup
failure is logged separately with its diagnostic chain; it must not replace a
constraint code or an application-specific error variant. Queued effects run
only after commit succeeds. Real trigger and deferred-constraint fixtures
check the returned error, secondary log, skipped effects, rolled-back rows,
and subsequent connection reuse. These public helpers are separate from the
core engine/model-catalog wrappers, which still use combined diagnostics when
their rollback fails.

`is_sqlite_busy` checks primary code **5** in the low byte of a numeric SQLite
code, including **261 / 517 / 773** (recovery/snapshot/timeout). A known non-busy
code must not be overridden by text containing `database is locked`; message
fallback applies only without a usable numeric code. A real two-pool WAL
fixture verifies that `BUSY_SNAPSHOT` is **517**, not 31. A snapshot conflict
requires restarting the transaction, not retrying its statement in the same
snapshot. The write-lock helper retries lock acquisition before invoking the
operation, at most five times, with **100 / 200 / 400 / 800 / 1,600 ms**
backoffs in addition to each connection's busy timeout. It does not replay
the caller's operation or make post-commit effects durable on cancellation.

## Workbench file integrity

Run the file behavior tests independently, then the application package and
workspace gates when changing the handlers or route signatures:

```sh
cargo test --locked -p agena --bin agena server::fs::fs_core::tests
cargo test --locked -p agena --bin agena server::app::tests::fs_
cargo test --locked -p agena --bin agena server::fs::fs_http::tests
cargo test --locked -p agena
```

Chunked text reads use byte offsets. `limit=0` remains a metadata-only request,
which the frontend uses before its large-file prompt. Positive limits are
normalized to **4 bytes through 2 MiB**; the default is **256 KiB**. A successful
positive read with `hasMore` must advance `nextOffset`. A chunk may defer an
incomplete trailing UTF-8 character to the next read, but incomplete UTF-8 at
EOF is rejected. The handler checks the opened file's length and short reads
so detected concurrent truncation reports an error instead of a stuck cursor.
Separate requests still do not form a versioned file snapshot.

Upload and text write stage complete contents in the target directory and
sync the temporary file before publishing it atomically. `overwrite=false`
uses `persist_noclobber`, so a competing creator is rejected even if preflight
observed no file. Replacements preserve file mode and follow existing valid
symbolic links without replacing the link itself. Read-only files and dangling
links are rejected; the original target's OS write-access checks still apply.
Same-directory staging also requires a writable parent directory.

At most **16** blocking file workers run concurrently. Their semaphore permit
remains with the worker if the HTTP waiter is cancelled; an already started
write may still publish its complete result. This is single-file atomic
publication, not cancellation rollback or a transaction across files. Hard
link aliases retain the old inode; owner/ACL/xattr preservation and directory
sync for power-loss durability remain separate audit work.

The Unix write-failure regression runs its own test subprocess, ignores
SIGXFSZ there, and applies a **4 KiB RLIMIT_FSIZE** to make the kernel fail a
larger write partway through. The parent process and personal files are not
affected. The test checks both preservation of old contents and absence of
partial newly created targets or leftover staging files.

The upload and text-write routes use explicit body readers, avoiding Axum's
default **2 MiB** cap on `Bytes` and `Json` extractors. Upload accepts at most
**50 MiB** of actual body bytes. Text write permits **300 MiB + 64 KiB** of
encoded JSON, enough for 50 MiB of fully escaped content (`\u0000` takes six
wire bytes), with bounded overhead for the path, syntax, and whitespace.
The handler separately enforces **50 MiB of decoded UTF-8 content** before
creating directories or writing. `Content-Length` enables early rejection;
actual bytes are counted even when the header is absent or understated.

At most **two** upload/write requests poll and buffer their bodies at once.
Incoming frames are coalesced as they arrive so tiny frames cannot accumulate
per-frame storage overhead. JSON parsing runs on a blocking worker, which
keeps its request permit if the HTTP waiter is cancelled. Cancellation during
body reading releases the slot and leaves files alone; already started file
workers retain the publication semantics described above. JSON buffers and
decoded strings still consume memory: these are byte and concurrency bounds,
not a zero-copy or streaming-to-disk upload implementation. Read/queue timeouts
and fairness under slow clients remain separate work.

These two routes report body, query, and JSON extraction failures in the
Workbench `{ "error": "..." }` envelope. Size, media-type, JSON syntax, and
JSON data-shape failures retain HTTP **413 / 415 / 400 / 422**, respectively.
Other JSON mutation routes retain their default body limits. The route tests
construct the same filesystem router as the server and cover exact limits,
escaped and multibyte content, transport failures, cancellation, and file
preservation. Actual-server probes also cover HTTP/1.1 chunked transfer.

Content replacement also uses staged atomic publication. Both single-match
and bulk output are limited to **50 MiB** while generating the replacement,
including a single large capture expansion. With `isRegex=false`, replacement
text is literal, preserving the earlier `NoExpand` behavior for currency,
shell variables, and dollar signs (`$5`, `$HOME`, `$$`). Regex mode compiles
replacement syntax through `regex_automata::util::interpolate::string` and
appends captures with byte-budget checks; named/numeric references, `$$`, and
unknown or malformed references keep the library's existing grammar. No-op
replacements do not publish a new file or count as completed changes.

Full text read, raw preview, download, content-search, and transform reads share
the bounded reader. It checks metadata before and after opening, rejects
non-files, and reads at most the configured limit plus one byte even if the
opened inode grows. Full reads allow **50 MiB**; search/transform input retains
its **2 MiB** searchable-file limit. Oversized read/raw/download requests return
**413**, non-files and invalid UTF-8 text return **400**, and missing files
return **404**, all in the Workbench JSON error envelope. Raw/download preserve
arbitrary binary bytes, MIME type, cache policy, and inline/attachment headers.
On Unix, nonblocking open prevents a swapped FIFO from waiting for a writer.

These reads share the 16-worker limit and its cancellation-safe permit with
file mutations. Completed responses can retain their buffers while clients
receive them, so the worker limit is not a total-process memory budget. The
byte limit bounds reads, not an exact allocator capacity or a consistent
snapshot during external in-place writes. Slow-filesystem deadlines and the
budget for queued/completed HTTP responses remain separate work.

Participating Workbench writes, uploads, and transforms use canonical-path
mutexes and share a process-local namespace lock with delete and rename.
Transforms retain these locks across reading, generation, and publication.
The path-lock registry prunes dead entries. The full original contents are
compared again before publication; detected external edits return **409**.
These locks do not coordinate other processes, external editors, or unrelated
runtime/plugin filesystem tools. External writers can still race between the
final comparison and rename. There is no cross-file transaction, cancellation
rollback, or guarantee of metadata preservation beyond the mode/link behavior
described above.

Search results include a lowercase SHA256 `revision` of the complete original
bytes. Single-match replacements accept `expectedRevision`; bulk accepts an
`expectedRevisions` path map. Supplied revisions must be 64 hex characters and
match the selected candidates; duplicate normalized paths are rejected before
mutation. Unknown replacement fields are rejected so misspelled revision keys
cannot silently disable conflict checking. Omitting revisions intentionally
targets current contents, while the selected-match `expected` text check still
applies. The Web UI requires valid revisions from its displayed search results.

Bulk replacement stops at the first failure and keeps the failure's non-2xx
status. Its error envelope includes `details.completed` (the normal result
shape for changes already published), `details.failedPath`, and
`details.remaining` (unattempted files after the failure, excluding the failed
file). Callers must not assume a failed request left every file unchanged.
The Web UI validates this information, reports completed counts and the failed
path, invalidates cached reads, and refreshes search even after uncertain
transport failures. Auto-refresh preserves drafts created during the request;
request sequences prevent older responses from replacing newer search or
workspace state. Visible browser interaction remains unverified.

Mixed directory/file replacement scopes are capped at **4,000** candidates.
Directory expansion obeys `includeHidden` and `respectGitignore`; explicitly
selected individual files remain included independently of those discovery
filters. Responses report `truncated`, which the UI surfaces.

## Generated tool contracts

When an intentional tool schema, help text, or workspace version change makes
the generated-contract tests fail, regenerate and review both artifacts:

```sh
cargo run --locked -p agena-bundled-plugins --example generate_capability_identity_snapshot > /tmp/agena-capability-identities.json
cargo run --locked -p agena-bundled-plugins --example generate_tools_reference > /tmp/agena-tools-reference.md
```

Compare the files with `crates/agena-bundled-plugins/generated/` before replacing
them. A passing snapshot test proves agreement with the implementation; review
is still necessary to determine whether the schema change is intended.

## Dependency maintenance

`bash scripts/dependencies/check.sh check` runs the strict dependency gate;
`report` prints findings without treating each finding as a fatal error. Both
use the current `packages/agena-web` package. The scripts require `cargo-deny`,
`cargo-audit`, and `cargo-machete`; report mode also requires `cargo-upgrade`.
The Rust advisory exceptions are documented in `deny.toml` and must stay in
sync with the cargo-audit arguments.

The remaining Rust advisory exception is RSA: jsonwebtoken's rust_crypto
backend includes it, while MCP OAuth uses EdDSA exclusively. The lockfile also
includes the disabled sqlx-mysql dependency path. Reassess the exception before
enabling RSA algorithms or MySQL. Tantivy now uses the fixed LRU version via
the dependency patch in `third_party/tantivy/AGENA_PATCH.md`.

The Web package currently overrides DOMPurify to **3.4.13**. Monaco 0.55.1 pins
the vulnerable 3.2.7 release, and even Monaco 0.56.0 pins 3.4.8. The override
keeps the current editor API while bringing its HTML sanitizer to a fixed
release. Remove the override when the selected Monaco release resolves to a
safe sanitizer without it. Keep `bun.lock` committed and audit dependency
updates, including transitive dependencies.

If the advisory database cannot be refreshed, `cargo deny --offline check
advisories` checks the cached database. Record its Git revision and timestamp
alongside the result; an offline pass does not establish that newly published
advisories have been checked.

Normal builds must not modify tracked sources. The vendored fingerprint
library now uses its committed Chrome-version corpus; refreshing that corpus
requires `SPIDER_FP_REFRESH_CHROME=1` and a reviewed diff. See
`third_party/spider_fingerprint/AGENA_PATCH.md` for that patch.

Do not disable macOS compact unwind tables to silence a large debug binary's
linker warning. That flag makes even `catch_unwind` terminate the process on
the development host. The in-process plugin transport test exercises both
synchronous and asynchronous panic recovery so this remains visible in CI.
