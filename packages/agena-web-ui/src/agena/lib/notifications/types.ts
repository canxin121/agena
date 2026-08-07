// Wire types for the unified notification surface (`/api/v1/notifications*`).
// Mirrors `agena-api::resource::notification` (Phase 3) so the web client
// consumes the same stable contract as any other client.

export type NotificationSeverity = 'info' | 'success' | 'warning' | 'error'

export type NotificationSource = 'runtime' | 'app' | 'plugin' | 'background' | 'frontend'

export type NotificationSurface =
  | 'banner'
  | 'toast'
  | 'composer_chip'
  | 'composer_footer'
  | 'status_line'
  | 'terminal_title'
  | 'terminal_progress'
  | 'terminal_bell'
  | 'activities_panel'
  | 'history_search'
  | 'permission_dialog'
  | 'input_prompt'
  | 'settings'
  | 'plan_panel'
  | 'background_task'
  | 'log'

export type NotificationControl = 'dismiss' | 'copy' | 'pin'

export type NotificationState = 'idle' | 'running' | 'awaiting' | 'blocked' | 'finished' | 'failed' | 'cancelled'

export type RunNotificationState =
  'queued' | 'running' | 'paused' | 'awaiting_input' | 'blocked' | 'finished' | 'failed' | 'cancelled'

export type NotificationKind =
  | { kind: 'notice'; code: string }
  | { kind: 'progress'; current: number | null; total: number | null }
  | { kind: 'status'; state: NotificationState }
  | { kind: 'model_status'; model: string; thinking: string | null; speed: string | null }
  | { kind: 'plan_progress'; current: number; total: number }
  | { kind: 'run_state'; state: RunNotificationState }
  | { kind: 'command_execution'; command: string; stream: string | null; exit_code: number | null }
  | { kind: 'tool_call'; call_id: string; name: string }
  | { kind: 'background_activity'; activity_id: string }
  | { kind: 'permission_request'; request_id: string }
  | { kind: 'user_input_request'; request_id: string }
  | { kind: 'history_search'; query: string; current: number; total: number }
  | { kind: 'terminal_title'; title: string }
  | { kind: 'terminal_notify'; text: string }
  | {
      kind: 'usage_update'
      current_tokens: number
      projected_tokens: number | null
      context_window: number | null
    }
  | { kind: 'custom'; plugin_id: string; code: string; data?: unknown }

export type NotificationScope =
  | 'global'
  | { session: number }
  | { workspace: number }
  | { tool_call: string }
  | { provider: string }
  | { plugin: string }
  | { background_task: string }

export type NotificationActionTarget =
  | { target: 'recovery'; directive: string }
  | { target: 'command'; command: string; input?: unknown }
  | { target: 'navigate'; route: string }
  | { target: 'copy'; text: string }

export interface NotificationAction {
  id: string
  label: string
  target: NotificationActionTarget
}

export interface NotificationResource {
  id: string
  kind: NotificationKind
  severity: NotificationSeverity
  scope: NotificationScope
  surface: NotificationSurface
  source: NotificationSource
  summary: string
  detail?: string | null
  control: NotificationControl
  actions: NotificationAction[]
  priority: number
  dedup_key?: string | null
  created_at_ms: number
  expires_at_ms?: number | null
  dismissed: boolean
}

export interface PaginatedNotificationsResponse {
  items: NotificationResource[]
  page: {
    next_cursor: string | null
    has_more: boolean
    returned: number
  }
}

/** SSE `data:` payloads on `/api/v1/notifications/stream`. */
export interface NotificationStreamLaggedPayload {
  skipped: number
}

export interface NotificationStreamResumedPayload {
  up_to_ms: number
}

export interface NotificationStreamClosedPayload {
  reason: string
}

export interface LocalNotificationOptions {
  detail?: string | null
  surface?: NotificationSurface
  dedupKey?: string
  priority?: number
  ttlMs?: number
}

/** Handle passed to page composables for emitting local notifications. */
export interface NotificationsHandle {
  error(message: string, options?: LocalNotificationOptions): void
  notice(message: string, options?: LocalNotificationOptions): void
  success(message: string, options?: LocalNotificationOptions): void
  toast(severity: NotificationSeverity, message: string, ttlMs?: number): void
  clearBanner(): void
}
