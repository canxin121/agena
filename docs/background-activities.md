# Background Activities — Unified Management

## Problem

Agena built-in tools create many kinds of long-running background work:

| Source | Example | Current state |
| ------ | ------- | ------------- |
| `shell.run background` | servers, watchers, builds | `MonitorRegistry` in runtime-tools |
| `tasks.*` | delegated subagents | plugin storage + `SubtaskStatusChangedEvent` |
| runtime tasks | marketplace sync/install/reload | `RuntimeBackgroundTaskRegistry` |
| `web.browser_*` | managed browser sessions | plugin-internal `browser_clients` map |
| `cron.*` | scheduled jobs | scheduler |

The backend, TUI, and web have **no unified way to list, inspect, follow, or
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
- `BackgroundActivityChangedEvent` — one event per mutation with a `reason`
  (`started` / `updated` / `finished` / `dismissed`)
- `BackgroundActivityFilter` — kind/status/session scoping used by queries

### 2. Backend service (`agena-runtime`)

`ActivityRegistry` — single in-memory store with bounded history.

- Sources push into it:
  - shell monitors via a new `MonitorListener` hook on `MonitorRegistry`
  - delegated tasks via bus `SubtaskStatusChangedEvent`
  - runtime tasks via a new listener on `RuntimeBackgroundTaskRegistry`
  - plugins can publish rich `PluginEvent(kind_label="activity")` payloads
- Every mutation publishes `background_activity_changed` on the runtime event
  bus (persistent), which is projected into SSE/WS and the presentation stream.
- `RuntimeActivityService` exposes `list / get / logs / stop / dismiss`.

### 3. API (`agena-api` + `agena-application` + `agena-api-server`)

- Queries: `list_activities`, `get_activity`, `activity_logs`
- Commands: `stop_activity`, `dismiss_activity`
- REST:
  - `GET  /api/v1/activities`
  - `GET  /api/v1/activities/{id}`
  - `GET  /api/v1/activities/{id}/logs?since_seq=&limit=&wait_ms=`
  - `POST /api/v1/activities/{id}/stop`
  - `DELETE /api/v1/activities/{id}`

### 4. TUI

- `/activities` command + key binding opens the Background Activities panel.
- Panel: grouped list (running first), status colors, kind icons, live duration;
  select → log tail; `s` stop, `d` dismiss, `x` clear finished, `r` refresh.
- Footer indicator shows a compact running-task pill (like Claude Code / Gemini).

### 5. Web

- `/activities` page + sidebar entry.
- List with live status badges, filter by kind/status, detail drawer with live
  log tail, stop/dismiss actions; updates arrive via the existing SSE bus.
