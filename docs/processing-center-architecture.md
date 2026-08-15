# Processing center and thin-client architecture

Status: implementation RFC (phases 1–3 substantially implemented; phase 4
transport/race/restart hardening implemented, with surface and platform gates
still open)

## Outcome

Agena should have one long-lived processing center and any number of clients.
The center owns every operation whose lifetime may outlive a UI connection.
Web, TUI, IDE, and CLI clients only submit commands, read snapshots, and
subscribe to changes.

Closing a client must never cancel a run. Opening another client must show the
same running sessions and allow that client to observe, answer, steer, or
cancel them. Starting the TUI without an explicit session must show sessions
that need attention, sessions that are running, and then recently changed
sessions; it must not pretend that an empty local conversation is the current
state of the system.

## Current state

The repository already contains most of the server-side mechanics needed for
the center:

- `agena center` bootstraps one `Runtime`, `Application`, session store,
  scheduler, plugin host, and HTTP API, then retains them for the lifetime of
  the server process.
- HTTP message submission calls `SessionManager::start_registered`. Once the
  execution registry slot exists, the manager spawns a lifecycle-owner task
  and returns an accepted execution identity. Dropping the HTTP request or an
  SSE connection does not drop that task.
- Session changes are committed before they are broadcast. A reconnecting
  client establishes the subscription before reading the persisted
  snapshot/version, closing the subscribe/read race. Notifications are an
  invalidation/latency path, not the source of truth; lag forces another
  authoritative snapshot.
- The session facade already enforces cross-process single-writer leases and
  derives `Ready`, `Running`, `AwaitingUser`, `Interrupted`, and `Failed` from
  persisted parts plus the lease heartbeat.
- Web already uses REST plus SSE and therefore behaves like a center client.
- TUI now defaults to a cloneable remote `TuiBackend` backed by
  `agena-client`. `App` no longer stores a concrete `Application`; only
  explicit `tui --embedded` mode contains one behind the backend boundary.
  Session overview/snapshot/submit/continue/compact/cancel/fork/rewind,
  selection changes, and interactive replies all cross the public API.
- The center atomically publishes an identity-bearing endpoint record and the
  CLI exposes foreground plus `center start/status/stop` lifecycle operations.
  `center install/uninstall` generate user-private, shell-free launchd or
  systemd definitions with login startup and on-failure restart policy;
  `start/stop` control the installed service when present. Windows service
  integration and real-platform install smoke automation remain future work.
- The IDE `rpc-server` is a stdio-to-center bridge. It retains only
  `agena-client`, a workspace id, and public provider metadata; EOF closes the
  bridge but does not shut down Runtime or cancel center-owned work.
- Session-critical and operational one-shot CLI commands use public center
  APIs: exec/continue/resume/fork/session import-export, permissions,
  provider/plugin/auth inspection and mutation, cost/usage, Git/snapshots,
  memory, config, diagnostics, MCP status/reconnect, and patch application.
  The `agena-cli` crate no longer contains a Runtime bootstrap path.
- Git, snapshot, commit, and pull-request helpers verify the canonical CLI
  current directory against the workspace root returned by the center.
  Commit and pull-request preflight this check before any mutation.
- A center-owned operator API exposes the runtime tool catalog and invocation.
  Every invocation carries a database-backed workspace id. The center resolves
  its persistent path and requires it to be canonically equal to the composed
  Runtime executor's workspace root before tool lookup or execution. `apply`
  uses this route; `mcp-server` is a tools-only stdio-to-center bridge whose
  EOF does not shut down Runtime; MCP reconnect also executes in the center.
  Workspace resolve/create/update canonicalizes existing directories before
  lookup and persistence so filesystem aliases converge on one database id.
- MCP add/remove/enable/disable use explicit global/workspace settings-layer
  routes. The center selects the concrete workspace config path, validates the
  resulting document, and owns reload. Workspace mutations additionally use a
  canonical client/center workspace preflight.
- Provider API-key, browser, and device login/logout are center-owned. CLI,
  TUI, and IDE clients can exchange `AGENA_CENTER_PASSWORD` for an in-memory
  bearer token or use `AGENA_CENTER_TOKEN`; endpoint records never contain a
  secret. MCP manual bearer and OAuth credential lifecycles are also
  center-owned: PKCE sessions remain in bounded center memory and no token or
  client credential is returned in a DTO or written to normal config.
  Password-authenticated clients retain the password only in a zeroizing
  process-memory secret and coalesce concurrent 401 responses into one bearer
  re-exchange plus one request replay. Static bearer tokens are never treated
  as passwords. Real-platform user-service smoke automation and the full
  Web/TUI surface process matrix remain open.

The target architecture should therefore promote the existing server runtime
to the center instead of introducing another execution engine or another
conversation protocol.

## Target topology

```text
                         one ownership boundary
  Web ───── REST/SSE ─┐
  TUI ───── REST/SSE ─┼──> Agena processing center
  VS Code ─ REST/WS ──┤      ├─ Runtime / provider clients
  CLI ───── REST ─────┘      ├─ SessionManager + execution registry
                              ├─ scheduler / plugin host / MCP / LSP
                              ├─ background-operation coordinator
                              └─ SQLite session + configuration stores
```

Only the center may construct the execution runtime. A client must not open
the chat database, acquire a session lease, start the scheduler, load runtime
plugins, or own model/tool tasks.

Local file presentation is not an exception to the ownership rule. A client
may read a file locally for terminal rendering only when the center explicitly
identifies the same local workspace. All agent tool execution and all writes
still go through the center.

## Ownership and lifecycle invariants

1. **Accepted means detached from the request.** The center must durably create
   the run marker and execution identity before returning an accepted command.
   Connection cancellation never maps to run cancellation. Cancellation is an
   explicit command carrying the current `execution_id`.
2. **One execution owner.** The center's execution registry is the fast
   in-memory authority. The persisted session lease is the crash and
   cross-process authority. A second center cannot steal a fresh lease.
3. **Persistence before notification.** Clients recover from missed or lagged
   notifications by reading a new snapshot. No correctness decision depends
   on receiving every SSE/WS frame.
4. **Interactive work is session state.** Permission and user-input requests
   are persisted session parts. They can be answered by a different client
   from the one that submitted the run.
5. **Optimistic multi-client mutation.** Metadata and interaction commands use
   session versions or stable request/execution identities. Stale commands
   fail with a conflict and the client refreshes; last-response-wins behavior
   must not silently overwrite another client.
6. **Center shutdown is explicit.** Client exit never calls
   `RuntimeBootstrapResult::shutdown`. Center shutdown stops accepting work,
   drains or terminalizes owned executions according to policy, flushes
   state, and only then shuts down Runtime.

## Public session overview

`lifecycle_state` is not enough for a client home screen: it only says whether
the session record was created. The shared session resource also needs the
processing state derived by the store:

- `awaiting_user`: highest attention priority;
- `running`: the center owns a fresh execution lease;
- `interrupted`: an in-flight marker has no fresh owner and needs recovery;
- `failed`: latest usable state failed;
- `creating`: session construction has not finished;
- `ready`: no foreground work is in flight (shown as recently finished on the
  home surface).

The first implementation exposes this as `SessionResource.state`. The store
also provides a batch state projection, so list/overview surfaces do not load
one complete transcript per row. Clients sort the fetched overview by the
priority above and then by `updated_at` descending. The storage derivation, not
an HTTP request's local loading flag, is the authority.

The explicit overview query returns all attention/running sessions plus a
bounded recent tail:

```text
GET /api/v1/sessions/overview?workspace_id=…&recent_limit=50

{
  "attention": [SessionResource…],
  "running": [SessionResource…],
  "recent": [SessionResource…],
  "generated_at": "…"
}
```

A session appears in exactly one group. `recent` excludes subagents by default
but the active parent view still reflects active descendant work. The endpoint
needs a single store/repository operation rather than client-side N+1 reads.

The pending-interaction wire discriminator is `kind: permission | user_input`.
A user-input request's own subtype is serialized separately as `input_kind`
(for example `ask_user` or `review`); reusing `kind` for both values makes a
strict client unable to decode the flattened resource.

## Client transport contract

The existing `agena-api`, `agena-api-server`, and `agena-client` crates remain
the only wire contract.

A thin session client needs these operations:

- health/readiness and center identity;
- resolve/list workspaces;
- overview/list/get/create/rename/delete sessions;
- get an execution snapshot and ordered parts snapshot;
- submit, continue, compact, fork, rewind, steer, cancel;
- reply to permission and user-input requests;
- mark an interaction presented;
- list/stop background activities;
- subscribe by global/workspace/session scope.

Operational clients additionally use typed settings-layer, provider/MCP
credential, operator-tool, Git/snapshot, diagnostics, usage, and plugin APIs.
Those operations are not permission to construct a client-local Runtime.

The reconnect sequence is always:

1. connect and validate protocol/center identity;
2. subscribe (so mutations during the read are queued);
3. read overview or session snapshot with its version;
4. consume notifications;
5. on lag, reconnect, or version discontinuity, discard incremental
   assumptions and repeat from step 2.

The current HTTP server already follows this ordering for the session-specific
change stream. The Rust client should expose the same snapshot-plus-stream
operation so TUI code cannot accidentally reverse it.

## Operator tool bridge and workspace boundary

Sessionless administrative clients use two public routes:

```text
GET  /api/v1/operator/tools
POST /api/v1/operator/tools/invoke
```

The outer server authentication middleware protects both routes whenever UI
password authentication is enabled. Only the center retains
`RuntimeToolExecutionService`, the concrete tool executor, and the monotonic
operator call-id counter. Clients send a database-backed `workspace_id`, a
public tool name, and structured input. Closing `agena mcp-server`, including
EOF before MCP initialization, drops only that stdio bridge; it never invokes
center shutdown.

The application resolves that id through the workspace repository, then
canonicalizes both its persistent path and the workspace root captured by the
composed Runtime tool executor. An unknown id returns not-found; a mismatched
root returns conflict before tool-name parsing or executor dispatch. Thus the
generic route, `agena apply`, and `agena mcp-server --workspace` all share a
server-side authoritative workspace boundary rather than relying only on a
client preflight.

This is deliberately a single-workspace executor contract, not a claim that
one center already supports multiple workspaces or tenants. Before exposing a
multi-workspace center, tool resolution and filesystem effects must use a
per-scope executor, and authenticated principals must be bound to explicit
workspace grants. Possession of a center-wide bearer token alone is not that
authorization model.

MCP settings and credentials do not use the generic operator route. A
settings-layer request names only `global` or `workspace`; the center maps that
choice to its composed global or project config path, validates writes, and
performs requested reloads. Credential routes return only server, credential
kind, store, and action. Manual bearer values go directly to the center's
keyring or explicitly selected private file store. OAuth discovery, dynamic
client registration, PKCE verifier, token exchange, optional revocation, and
credential persistence all remain center-owned; the thin CLI only opens the
authorization URL and listens on the validated loopback callback. Pending
OAuth flows expire after ten minutes and are bounded per center process.

## Center discovery and process management

The first supported transport is loopback HTTP. It keeps Web, TUI, CLI, and IDE
on the same tested protocol and leaves a path to remote authenticated use.

Resolution order for clients:

1. explicit `--center <URL>`;
2. `AGENA_CENTER_URL`;
3. the endpoint record written by the local center;
4. the default `http://127.0.0.1:3210`.

The endpoint record belongs under the Agena state directory and contains a
schema version, URL, center UUID, PID, start time, and protocol version. It is
discovery metadata, not proof that a process is alive; clients must call the
health endpoint and compare the center identity.

Password-derived authentication is also process-lifetime-aware. `AgenaClient`
keeps the password in a shared zeroizing memory secret and attaches the current
bearer dynamically to every REST request and SSE handshake. If a protected
request receives HTTP 401 (for example after the center restarts and loses its
in-memory session table), a generation check plus one async refresh mutex lets
exactly one clone exchange the password. Every waiter then retries once with
the new bearer. Because authentication middleware returns 401 before route
dispatch, this replay cannot duplicate an accepted mutation; there is no
unbounded retry loop. An explicitly supplied bearer remains static and fails
normally when rejected.

The CLI surface should converge on:

```text
agena center           # foreground service, suitable for systemd/launchd
agena center start     # install/start or spawn the user service
agena center status
agena center stop
agena tui --center …
agena exec --center …
```

`agena server` remains a compatibility alias for `agena center`. Background
spawning must use an explicit executable path, log path, PID/endpoint record,
and readiness handshake. It must not use shell interpolation. Process records
are removed only after identity validation, so a stale PID cannot stop an
unrelated process.

## TUI migration

The TUI uses a transport-neutral backend port rather than mixing concrete
`Application` calls with ad-hoc HTTP requests.

The migration boundary is the existing `app_backend` module:

1. Define a cloneable `TuiBackend` port using only `agena-api` resource and
   command/query types plus transport-neutral subscription events.
2. Move the current `Application` calls behind `EmbeddedTuiBackend`. This is a
   mechanical compatibility adapter and keeps current tests usable.
3. Implement `RemoteTuiBackend` with `agena-client`.
4. Change `App` to retain the port, workspace descriptor, and client-local
   presentation services only. It must no longer retain `Application`.
5. Move synchronous provider/plugin/config projections required by the TUI to
   existing or new API resources. Until each projection exists, that feature
   is explicitly unavailable in remote mode; the client must not bootstrap a
   second runtime as a fallback.
6. Switch `launch/tui.rs` to center discovery plus `RemoteTuiBackend`, then
   remove the embedded runtime bootstrap from the default path. Keep an
   explicit development-only embedded mode until parity tests pass.

Steps 1–6 and the first remote parity slice are implemented. Reconnect uses a
subscribe-before-snapshot pair and repeats that pair after lag or transport
closure. Provider/config/plugin studios still follow through the same port as
their typed center APIs are completed; remote mode never bootstraps a fallback
Runtime.

## Web behavior

Web is already a thin center client. Its overview presentation and reconnect
hardening now:

- show `SessionResource.state` in the session list;
- order attention and running sessions before recent sessions;
- keep the overview visible when no session route is selected;
- refresh overview state from a workspace stream, not only the currently
  selected session stream;
- preserve selected session by ID without making selection a prerequisite for
  observing other running sessions.

## Recovery semantics

Client disconnect requires no recovery action. Center process failure does:

- stale execution leases are reaped;
- in-flight markers without a fresh lease become `interrupted` and are
  reconciled to a terminal failure according to the existing store policy;
- scheduled/background work uses its durable operation/delivery state;
- a restarted center publishes the reconciled snapshot before accepting a
  conflicting continuation.

Automatically replaying an arbitrary foreground model call after center crash
is out of scope until provider idempotency and tool side-effect replay are
proven. Showing `interrupted` and offering an explicit continue/retry action is
safer and matches the current durable model.

## Delivery phases and gates

### Phase 1: shared overview truth

- expose authoritative processing state in `SessionResource`;
- show and prioritize it in Web and TUI session lists;
- open the TUI on the running/recent session view when no session is explicit;
- add API and presentation tests for all state values and default navigation.

Gate: two independently created client presentations given the same store
snapshot produce the same status and ordering.

Implementation status: the shared state projection, storage/SQLite parity,
overview endpoint, and Web/TUI presentation paths are implemented. The full
cross-process gate remains part of the automated E2E backlog.

### Phase 2: center identity and client session port

- add center identity/discovery metadata and CLI resolution;
- complete the high-level `agena-client` session API;
- introduce `TuiBackend`, embedded adapter, and remote session adapter;
- test snapshot-before-stream reconnect and stream lag recovery.

Gate: a TUI submits a run through the server, exits, a second TUI connects,
observes the same active execution, and receives its terminal state.

Implementation status: center identity/discovery, the public session client,
remote-default TUI backend, subscribe-before-snapshot reconnect, and the thin
IDE bridge are implemented. Isolated fake-provider tests now exercise two real
HTTP clients: the submitting client's SSE connection is dropped, a second
client observes the same running execution, and the center completes it. The
exact two-TUI process-level execution gate remains open.

### Phase 3: remote-default TUI and CLI

- route every execution-affecting TUI action through the center;
- route one-shot CLI commands through the center by default;
- stop bootstrapping Runtime in client launch paths;
- retain explicit embedded mode only for recovery/development.

Gate: process inspection shows exactly one scheduler, plugin host, execution
registry, and provider runtime while Web, TUI, and CLI are used concurrently.

Implementation status: default TUI, IDE, and CLI client paths do not bootstrap
Runtime. Real isolated-center CLI/RPC/operator/MCP-control smoke suites have
verified that client exit leaves the same center identity alive, patch writes
occur in the center workspace, mismatched workspaces fail before mutation,
MCP stdio discovery is center-backed, and workspace-layer MCP edits are
center-owned. Generic operator invocations also require a database-backed
workspace id which the center canonically binds to its composed Runtime root
before dispatch. A real subprocess E2E now holds a fake-provider execution
open while a Web-equivalent HTTP/SSE attachment, default remote TUI in a PTY,
IDE JSON-RPC bridge, MCP stdio bridge, and one-shot CLI coexist. An opt-in
Runtime composition audit records exactly the center PID as owner of Runtime,
provider clients, scheduler, plugin host, execution registry, and session DB;
the live SQLite lease table has one owner, every thin client runs under a
pre-composition Runtime-forbidden guard, and their disconnect leaves the same
execution running. The exact browser/TUI rendered-surface handoff tests remain
open; per-workspace executors and auth-principal grants are still required for
a true multi-workspace center.

### Phase 4: service lifecycle and multi-client hardening

- implement `center start/status/stop/install/uninstall` and OS user-service
  integration;
- add global/workspace overview subscriptions;
- enforce version/idempotency contracts on every shared mutation;
- exercise two-client races for rename, permission reply, cancel, and send.

Gate: end-to-end restart/disconnect/race tests pass on SQLite and no client
process holds a session lease.

Implementation status: detached and launchd/systemd-backed lifecycle,
identity-safe records, password/token client auth, workspace streams, snapshot
convergence, and atomic permission reply semantics exist. Fake-provider HTTP
tests cover cross-client user-input and permission replies, different-answer
and allow/deny races (one durable reply and one continuation), disconnect, and
cancel versus natural completion. A deterministic real-HTTP SSE overflow test
proves current snapshot + older queued patch + lag convergence. A real center
subprocess test kills a provider-blocked center, ages its durable lease,
restarts with a new identity, observes `Interrupted` rather than `Running`,
reconciles the old run as `failed/process_restart`, and completes one explicit
continuation. Both launchd and systemd definitions are generated and
syntax/escaping-tested on macOS, while actual systemd/launchd install and
service-manager crash-restart smoke remains platform CI work. Windows service
integration and the complete Web/TUI surface-level suite remain unfinished.
Client-level real HTTP tests cover concurrent refresh, mutation replay, SSE
reconnect, static-bearer non-refresh, and Debug redaction. A real center
subprocess test also kills and restarts a password-protected center on the same
endpoint and proves the original client reauthenticates without reconstruction.

## Required end-to-end tests

1. Submit from Web, close Web, observe completion from TUI.
2. Submit from TUI, exit TUI during model streaming, observe completion from a
   new TUI and Web.
3. Disconnect while waiting for permission, answer from another client, and
   complete the same execution. (Covered through independent HTTP clients.)
4. Let an SSE client lag, force a snapshot reload, and converge without lost or
   duplicated parts. (Covered through a forced real-HTTP queue overflow.)
5. Race two permission replies; exactly one consumes the request and the loser
   observes the idempotent terminal snapshot or a conflict. (Covered.)
6. Race cancel against natural completion; both clients converge on one
   terminal state. (Covered.)
7. Kill the center during a run, restart it, and show/reconcile `interrupted`
   without claiming the run is still healthy. (Covered by a real subprocess
   and file-backed SQLite test.)
8. Start Web, TUI, CLI, IDE RPC, and MCP bridge together and assert that only
   the center owns Runtime, provider clients, scheduler, plugin host,
   execution registry, session DB, and the execution lease. (Covered by a
   real subprocess/PTY test with an opt-in composition audit and live SQLite
   lease assertion; rendered Web↔TUI observation remains in items 1–2.)
