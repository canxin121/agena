// AUTO-GENERATED FILE - DO NOT EDIT.
//
// TypeScript mirror of Agena's backend state types. Regenerate with:
//
// cargo run -p agena-web-types > packages/agena-web/src/generated/agenaState.ts
//
// Source of truth: crates/agena-domain, crates/agena-api, crates/agena-notification.
// `cargo test -p agena-web-types` fails whenever this file and the backend
// definitions diverge, so the frontend cannot keep a second, silently drifting
// copy of a state type. Literal arrays carry the order of the Rust JSON schema.

/** JSON payload carried opaquely through the wire contract. */
export type JsonValue = unknown

/** The single derived processing state of a session. */
export const SESSION_STATE_KINDS = ['creating', 'ready', 'running', 'awaiting_interaction', 'failed'] as const
export type SessionStateKind = (typeof SESSION_STATE_KINDS)[number]

/** Kinds whose session is executing model work. */
export const SESSION_STATE_BUSY_KINDS = ['running'] as const

/** Kinds that always need user attention. */
export const SESSION_STATE_ATTENTION_KINDS = ['awaiting_interaction', 'failed'] as const

/** Kinds whose session terminally failed. */
export const SESSION_STATE_FAILED_KINDS = ['failed'] as const

/** Visibility/readiness status of a persisted session. */
export const SESSION_LIFECYCLE_STATES = ['creating', 'ready', 'failed'] as const
export type SessionLifecycleState = (typeof SESSION_LIFECYCLE_STATES)[number]

/** Persistent state of a session's execution workflow. */
export const WORKFLOW_STATES = ['quiescent', 'tool_pending', 'awaiting_interaction'] as const
export type WorkflowState = (typeof WORKFLOW_STATES)[number]

/** Active phase of a session execution. */
export const EXECUTION_PHASES = ['starting', 'preparing_model', 'streaming_model', 'executing_tools', 'awaiting_interaction', 'cancelling'] as const
export type ExecutionPhase = (typeof EXECUTION_PHASES)[number]

/** Execution access mode of a session. */
export const EXECUTION_ACCESSES = ['inherit', 'read_only'] as const
export type ExecutionAccess = (typeof EXECUTION_ACCESSES)[number]

/** Lifecycle status of a delegated subtask. */
export const SUBTASK_STATUSES = ['created', 'running', 'completed', 'failed', 'cancelled', 'timed_out', 'interrupted'] as const
export type SubtaskStatus = (typeof SUBTASK_STATUSES)[number]

/** Statuses where a delegated subtask can no longer progress. */
export const SUBTASK_TERMINAL_STATUSES = ['completed', 'failed', 'cancelled', 'timed_out', 'interrupted'] as const

/** Domain meaning of a session's immutable parent edge. */
export const SESSION_RELATION_KINDS = ['root', 'child', 'fork', 'rewind', 'subagent'] as const
export type SessionRelationKind = (typeof SESSION_RELATION_KINDS)[number]

/** Execution state of a message, operation part, or interactive request. */
export const EXECUTION_STATUSES = ['pending', 'in_progress', 'completed', 'failed', 'cancelled', 'policy_denied', 'user_declined', 'capability_unavailable', 'tool_unavailable'] as const
export type ExecutionStatus = (typeof EXECUTION_STATUSES)[number]

/** Statuses where an operation part is still in flight. */
export const PART_EXECUTION_ACTIVE_STATUSES = ['pending', 'in_progress'] as const

/** Statuses where an operation part reached a terminal outcome. */
export const PART_EXECUTION_TERMINAL_STATUSES = ['completed', 'failed', 'cancelled', 'policy_denied', 'user_declined', 'capability_unavailable', 'tool_unavailable'] as const

/** Kind of background activity. */
export const BACKGROUND_ACTIVITY_KINDS = ['shell', 'monitor', 'task', 'cron', 'runtime', 'browser'] as const
export type BackgroundActivityKind = (typeof BACKGROUND_ACTIVITY_KINDS)[number]

/** Lifecycle status shared by every background activity source. */
export const BACKGROUND_ACTIVITY_STATUSES = ['running', 'succeeded', 'failed', 'pending', 'waiting', 'paused', 'cancelled', 'stopped'] as const
export type BackgroundActivityStatus = (typeof BACKGROUND_ACTIVITY_STATUSES)[number]

/** Statuses where a background activity is still live. */
export const BACKGROUND_ACTIVITY_ACTIVE_STATUSES = ['running', 'pending', 'waiting', 'paused'] as const

/** Statuses where a background activity finished for good. */
export const BACKGROUND_ACTIVITY_TERMINAL_STATUSES = ['succeeded', 'failed', 'cancelled', 'stopped'] as const

/** Severity of a notification. */
export const NOTIFICATION_SEVERITIES = ['info', 'success', 'warning', 'error'] as const
export type NotificationSeverity = (typeof NOTIFICATION_SEVERITIES)[number]

/** Source of a notification. */
export const NOTIFICATION_SOURCES = ['runtime', 'app', 'plugin', 'background', 'frontend'] as const
export type NotificationSource = (typeof NOTIFICATION_SOURCES)[number]

/** Surface a notification is rendered on. */
export const NOTIFICATION_SURFACES = ['banner', 'toast', 'composer_chip', 'composer_footer', 'status_line', 'terminal_title', 'terminal_progress', 'terminal_bell', 'activities_panel', 'history_search', 'permission_dialog', 'input_prompt', 'settings', 'plan_panel', 'background_task', 'log'] as const
export type NotificationSurface = (typeof NOTIFICATION_SURFACES)[number]

/** Control action of a notification. */
export const NOTIFICATION_CONTROLS = ['dismiss', 'copy', 'pin'] as const
export type NotificationControl = (typeof NOTIFICATION_CONTROLS)[number]

/** State carried by a status notification. */
export const NOTIFICATION_STATES = ['idle', 'running', 'awaiting', 'blocked', 'finished', 'failed', 'cancelled'] as const
export type NotificationState = (typeof NOTIFICATION_STATES)[number]

/** State carried by a run notification. */
export const RUN_NOTIFICATION_STATES = ['queued', 'running', 'paused', 'awaiting_input', 'blocked', 'finished', 'failed', 'cancelled'] as const
export type RunNotificationState = (typeof RUN_NOTIFICATION_STATES)[number]

/** An active execution inside a session. */
export type ActiveExecutionResource = {
  execution_id: string
  phase: ExecutionPhase
}

/** Current processing state of a session. This is the single client-facing execution state. Durable session facts are derived from durable parts; the optional live execution and workflow payloads are attached by the application service when it has a full execution snapshot. */
export type SessionState = {
  kind: 'creating'
} | {
  kind: 'ready'
  data: {
    last_failure?: JsonValue
  }
} | {
  kind: 'running'
  data: {
    execution?: ActiveExecutionResource
    requests?: JsonValue[]
    workflow: WorkflowState
  }
} | {
  kind: 'awaiting_interaction'
  data: {
    execution?: ActiveExecutionResource
    requests?: JsonValue[]
    run_id?: number | null
  }
} | {
  kind: 'failed'
  data: {
    failure?: JsonValue
  }
}
