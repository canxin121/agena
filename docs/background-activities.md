# Background Activities — Unified Management

## Problem

Agena built-in tools create many kinds of long-running background work:

| Source | Example | Current state |
| ------ | ------- | ------------- |
| `shell.run background` | servers, watchers, builds | `MonitorRegistry` in runtime-tools |
| `tasks.*` | delegated subagents | plugin storage + `SubtaskStatusChangedEvent` |
| runtime tasks | marketplace sync/install/reload | `RuntimeBackgroundTaskRegistry` |
| `web.browser_*` | managed browser sessions | web plugin `publish_activity` + registered `Browser` source adapter |
| `cron.*` | scheduled jobs | scheduler |

The backend, TUI, and web now share one way to **list, inspect, follow, and
stop** this background work. This document describes the unified model, the
backend service, and the TUI/web surfaces.

## Design

### 1. Domain model (`agena-domain`)

- `BackgroundActivityKind` — `shell`, `task`, `runtime`, `browser`
- `BackgroundActivityStatus` — `pending`, `running`, `succeeded`, `failed`,
  `cancelled`, `stopped`
- `BackgroundActivity` — id, kind, status, title, description, command,
  workdir, session ids, timestamps, exit code, message, failure, log cursor
  (`last_seq`, `has_more`, `dropped_lines`), `cancellable`, `dismissible`
- `BackgroundActivityLogLine` / `BackgroundActivityLogRead` — unified log
  cursor protocol with `since_seq` tails
- `BackgroundActivityChangedEvent` — one event per mutation with a `reason`
  (`started` / `updated` / `finished` / `dismissed`)
- `BackgroundActivityFilter` — kind/status/session scoping used by queries

### 2. Backend service (`agena-runtime`)

`ActivityRegistry` — single in-memory store with bounded history (256 records,
active rows kept when trimming).

- Sources push into it:
  - shell monitors via a new `MonitorListener` hook on `MonitorRegistry`
  - delegated tasks via bus `SubtaskStatusChangedEvent` (bridged to `task_…`)
  - runtime tasks via a new listener on `RuntimeBackgroundTaskRegistry`
  - plugins publish rich `BackgroundActivity` records through the first-class
    `HostClient::publish_activity` capability and register an
    `ActivitySourceAdapter` per owned kind (the bundled web plugin publishes
    one `browser_...` activity per managed browser session on `browser_open`,
    updates the live record title/URL on main-frame navigations, and publishes
    a terminal `Stopped` record on `browser_close`, `browser_shutdown`, or
    plugin shutdown). For kinds with a registered adapter, log reads and stop
    requests dispatch to the adapter (`read_logs` / `stop`) instead of the
    built-in per-kind behavior — the web plugin tail streams CDP console and
    log events and closes the matching browser target on stop.
- Every mutation publishes `background_activity_changed` on the runtime event
  bus (persistent), which is projected into SSE/WS and the presentation stream
  (`RuntimePresentationEventKind::ActivityChanged`).
- `RuntimeActivityService` exposes `list / get / logs / stop / dismiss /
  clear_finished`; log reads and stop/dismiss delegate to per-kind adapters
  (monitor read/stop, subtask logs/cancel, runtime task cancel).

### 3. API (`agena-api` + `agena-application` + `agena-api-server`)

- Queries: `list_activities`, `get_activity`, `activity_logs`
- Commands: `stop_activity`, `dismiss_activity`, `clear_finished_activities`
- REST:
  - `GET    /api/v1/activities` (kind/status/session/active filters)
  - `GET    /api/v1/activities/{id}`
  - `GET    /api/v1/activities/{id}/logs?since_seq=&limit=&wait_ms=`
  - `POST   /api/v1/activities/{id}/stop`
  - `POST   /api/v1/activities/{id}/dismiss`
  - `POST   /api/v1/activities/clear-finished` (returns count)
- Wire resources `BackgroundActivityResource` / `BackgroundActivityLogResource`
  live in `agena-api` and convert from the domain model there.

### 4. TUI

- `/activities` command (aliases `/background`, `/tasks`) + palette entry opens
  the Background Activities panel.
- Panel (`agena-tui::activities` + `agena-tui-app` adapter):
  - grouped list (Active first), status colors, kind icons, live duration
  - filters: `f` toggle finished, `k` cycle kind, `t` cycle status
  - `↵` toggles the detail pane with live log tail
  - `s` stop, `d` dismiss, `x` clear finished, `r` refresh, `q`/`Esc` close
- Backend adapters (`agena-tui-backend::backend_activities`) call the
  application dispatch surface in-process.

### 5. Web

- `/activities` route + sidebar entry + command palette (`/activities`,
  `/background`, `/tasks`).
- `ActivitiesPage.vue`: live list with status badges, kind icons, duration,
  kind/status/active filters, expandable rows with log tail, stop/dismiss
  actions, clear-finished, 4s auto-refresh while mounted.
- API functions/types in `agenaApi.ts`; updates can also arrive through the
  existing SSE bus (`background_activity_changed`).
