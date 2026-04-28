# agena event system & v2 API

This document describes the unified event bus and the v2 API surface added by
the event-system refactor (Steps 1–8 of `sunny-hopping-octopus.md`).

## Crate layout

| Crate | Role |
|---|---|
| `agena-event` | Generic envelope (`DomainEvent<K>`), filter, in-process bus, publisher, sequence allocator, `EventStore` trait. Payload-agnostic. |
| `agena-event-store-sea` | sea-orm-backed `EventStore` impl + migrations. **Database-agnostic** — no driver feature is enabled by default; consumers pick `sqlx-sqlite` / `sqlx-postgres` / `sqlx-mysql`. |
| `agena-api` | Wire protocol: `Command` / `Query` / `Notification` / `ClientMessage` / `ServerMessage`. No transport. |
| `agena-api-server` | axum REST + WebSocket + SSE + Unix-socket transports, all backed by the same in-process bus. |
| `agena-client` | Official Rust SDK (HTTP + WS multiplexed). |

## Event flow

```
SessionManager (core)
  ├─ event_publisher(): Arc<EventPublisher>
  └─ event_bus():       Arc<dyn EventBus<EventKind>>
        │
        ├──► agena_events table (persistent, single source of truth)
        └──► tokio::broadcast
                │
                ├─► agena-api-server: WebSocket subscribers
                ├─► agena-api-server: SSE subscribers
                └─► agena-api-server: Unix-socket subscribers
```

Every legacy `SessionEvent` and `HistoryItem` produced by core also surfaces
on the unified bus as the corresponding `EventKind` variant. The legacy
tables are still written during the cutover; readers and new clients should
target the unified path.

## Subscription model

Subscriptions use `EventFilter`:
- `scope: Scope` — `Global` / `Workspace { id }` / `Session { id }`.
- `kinds: Option<HashSet<EventKindTag>>` — `None` = all kinds.
- `since_seq_global: Option<i64>` — resume from a previous cursor; the server
  replays from `EventStore::range` first, then attaches the live broadcast.

Slow consumers receive a `Lagged(n)` notification when broadcast capacity is
exceeded; clients should re-subscribe with `since_seq_global = last_seen_seq`.

## v2 API endpoints

REST (JSON):

- `GET  /api/v2/health`
- `GET  /api/v2/sessions[/{id}[/messages]]`
- `GET  /api/v2/events?scope_kind=…&since_seq_global=…&limit=…&kinds=a,b`
- `POST /api/v2/sessions`
- `POST /api/v2/sessions/{id}/turns`
- `POST /api/v2/sessions/{id}/continue`
- `POST /api/v2/sessions/{id}/cancel`
- `POST /api/v2/sessions/{id}/permission-replies`
- `POST /api/v2/sessions/{id}/user-input-replies`

WebSocket:

- `GET  /api/v2/ws` — duplex JSON RPC. Frames: `subscribe`, `unsubscribe`,
  `command`, `query`, `ping`. Server pushes `notification` / `command_result`
  / `query_result` / `error` / `subscribed` / `unsubscribed` / `pong` /
  `hello`.

SSE:

- `GET  /api/v2/events/stream?scope_kind=…&workspace_id=…&session_id=…&kinds=a,b&since_seq_global=…`
  — push-only, no DB polling, 25-second heartbeat.

Unix socket (Linux/macOS):

- `agena_api_server::ipc::serve(path, state)` — same JSON protocol as WS,
  one frame per line. Useful for local TUI/CLI clients.

## Versioning

- `agena_api::PROTOCOL_VERSION` — bumped on incompatible WS/REST framing
  changes. Server announces it via `ServerMessage::Hello { protocol_version }`.
- Per-payload evolution is handled by `envelope_schema` in `EventMeta`. Add
  new variants (e.g. `EventKindV2(...)`) rather than mutating existing ones.

## Status / scope

This refactor delivers the unified event substrate end-to-end:
publisher → store → bus → multi-transport server → SDK + TUI subscription.

**Migrated**:
- `agena-tui` consumes live events via the unified `EventBus` (push), in
  addition to its existing 250ms safety-net poll. See
  `Backend::subscribe_session_events` and `AppMessage::SessionEventArrived`.
- `agena-studio-server` mounts both v1 (`/api/v1/*`) and v2 (`/api/v2/*`)
  routers — Studio UI can adopt v2 incrementally.
- `agena-http-api-server` likewise serves v1 + v2 from one bind address.

**Still on v1 only** (no v2 ports yet): auth flows, runtime status, provider
listing, workspace/permission-rule CRUD, message-part details. The
`dispatch` layer in `agena-api-server` returns
`BadRequest("not yet implemented in v2")` for those operations.

The legacy `agena_session_events` and `agena_session_history_events` tables
are still written and read; the
`agena_event_store_sea::DropLegacyEventTablesMigration` migration is
provided but **not** registered in the default migrator. Run it once the
last v1 reader is gone.
