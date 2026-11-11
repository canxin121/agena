// AUTO-GENERATED FILE - DO NOT EDIT.
//
// TypeScript mirror of Agena's backend state types. Regenerate with:
//
// cargo run -p agena-web-types > packages/agena-web/src/generated/agenaState.ts
//
// Source of truth: crates/agena-domain, crates/agena-api, crates/agena-notification.
// Regenerate this file whenever the backend state definitions change. Literal arrays carry the order of the Rust JSON schema.

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

/** Lifecycle status of an agent. */
export const AGENT_STATUSES = ['created', 'running', 'completed', 'failed', 'cancelled', 'timed_out', 'interrupted'] as const
export type AgentStatus = (typeof AGENT_STATUSES)[number]

/** Statuses where an agent can no longer progress. */
export const AGENT_TERMINAL_STATUSES = ['completed', 'failed', 'cancelled', 'timed_out', 'interrupted'] as const

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
export const BACKGROUND_ACTIVITY_KINDS = ['shell', 'monitor', 'agent', 'cron', 'runtime', 'browser'] as const
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

export type AdapterDescription = {
  kind: AdapterKind
  channel_context_version?: number | null
  config_schema: JsonValue
  description: string
  idempotent_send: boolean
  minimum_poll_seconds: number
  title: string
}

export type AdapterKind = 'channel' | 'event_source'

export type AssistantMemory = {
  body: string
  description: string
  metadata: MemoryMetadata
  name: string
  sha256: string
  truncated: boolean
}

export type AssistantPatch = {
  address_user?: string | null
  cooldown_seconds?: number | null
  daily_limit?: number | null
  instructions?: string | null
  language?: string | null
  memory_enabled?: boolean | null
  name?: string | null
  proactive_enabled?: boolean | null
  quiet_enabled?: boolean | null
  quiet_end_minute?: number | null
  quiet_start_minute?: number | null
  timezone?: string | null
  tone?: AssistantTone
}

export type AssistantPreferences = {
  address_user: string
  cooldown_seconds: number
  daily_limit: number
  instructions: string
  language: string
  memory_enabled: boolean
  name: string
  proactive_enabled: boolean
  quiet_enabled: boolean
  quiet_end_minute: number
  quiet_start_minute: number
  timezone: string
  tone: AssistantTone
}

export type AssistantProfile = {
  preferences: AssistantPreferences
  revision: number
  updated_at_ms: number
  workspace_id?: number | null
}

export type AssistantTone = 'balanced' | 'warm' | 'direct' | 'playful'

export type BindingState = 'disabled' | 'enabled' | 'paused' | 'expired' | 'faulted'

export type ChannelAccessAudit = {
  actor: string
  after: ChannelAccessPolicy
  at_ms: number
  before?: ChannelAccessPolicy
  binding_id: string
  epoch: number
  request_key: string
  revision: number
}

export type ChannelAccessPolicy = {
  allow_automated_reports?: boolean
  allow_unlisted_chat?: boolean
  grant?: ChannelGrant
  identity: ChannelIdentity
  members: ChannelMember[]
  per_user_daily_limit?: number
  service_grant?: ChannelGrant
  session_mode?: ChannelSessionMode
  thread_mode?: ChannelThreadMode
  trigger?: ChannelTrigger
}

/** Immutable capability fence stored with a session and inherited by workers, agents and forks. Current membership is rechecked by the host verifier. */
export type ChannelAuthority = {
  actor: string
  binding_id: string
  epoch: number
  expires_at_ms: number
  grant: ChannelGrant
  identity: ChannelIdentity
  memory_workspace_root: string
  private_paths: string[]
  resource: string
  revoked?: boolean
  role: ChannelRole
  session_mode: ChannelSessionMode
  thread?: string | null
}

export type ChannelEnvelope = {
  actor_verified: boolean
  addressed_to_bot: boolean
  identity: ChannelIdentity
  is_member: boolean
  verified_at_ms: number
}

export type ChannelGrant = {
  mcp_tools?: Record<string, string[]>
  network_hosts?: string[]
  private_memory?: boolean
  source_workspace?: boolean
  tools?: string[]
}

export type ChannelIdentity = {
  kind: ChannelKind
  account: string
  tenant: string
}

export type ChannelKind = 'direct' | 'group' | 'public_channel' | 'private_channel'

export type ChannelMember = {
  actor: string
  expires_at_ms?: number | null
  role: ChannelRole
}

export type ChannelOutputRoute = {
  actor: string
  epoch: number
  identity: ChannelIdentity
  resource: string
  session_id?: number | null
  thread?: string | null
}

export type ChannelRole = 'blocked' | 'member' | 'operator' | 'administrator' | 'owner'

export type ChannelSessionMode = 'per_conversation' | 'per_actor'

export type ChannelSessionRecord = {
  authority: ChannelAuthority
  binding_id: string
  created_at_ms: number
  scope_key: string
  session_id: number
}

export type ChannelThreadMode = 'exact' | 'per_thread'

export type ChannelTrigger = 'direct_or_mention' | 'every_message'

export type ExternalEvent = {
  actor: string
  bot?: boolean
  channel?: ChannelEnvelope
  data?: JsonValue
  event_type: string
  id: string
  occurred_at_ms: number
  resource: string
  text: string
  thread?: string | null
}

export type InlineBlock = {
  id: string
  program: UiProgram
  type: 'panel'
} | {
  id: string
  text: string
  type: 'paragraph'
} | {
  id: string
  text: string
  title?: string
  tone?: InlineTone
  type: 'callout'
} | {
  id: string
  items: InlineStep[]
  title?: string
  type: 'steps'
} | {
  columns: string[]
  id: string
  rows: InlineComparisonRow[]
  title?: string
  type: 'comparison'
} | {
  id: string
  items: InlineMetric[]
  title?: string
  type: 'metrics'
} | {
  columns: string[]
  id: string
  rows: string[][]
  title?: string
  type: 'table'
} | {
  id: string
  text: string
  title: string
  type: 'details'
} | {
  chart_type: InlineChartType
  id: string
  labels: string[]
  series: InlineSeries[]
  title: string
  type: 'chart'
  x_label?: string
  y_label?: string
}

export type InlineChartType = 'line' | 'bar' | 'area' | 'pie'

export type InlineComparisonRow = {
  label: string
  values: string[]
}

export type InlineIcon = 'number' | 'shield' | 'layers' | 'sparkles' | 'link' | 'check' | 'search' | 'code'

export type InlineMetric = {
  detail?: string
  label: string
  value: string
}

export type InlineSeries = {
  name: string
  values: (number | null)[]
}

export type InlineStep = {
  icon?: InlineIcon
  label?: string
  text: string
  title: string
}

export type InlineTone = 'note' | 'tip' | 'warning'

export type IntegrationBinding = {
  activated_at_ms?: number | null
  admitted: number
  channel_session_id?: number | null
  created_at_ms: number
  cursor: JsonValue
  day: string
  day_admitted: number
  epoch: number
  id: string
  last_error?: string | null
  next_poll_ms: number
  permission_fingerprint?: string
  poll_failures: number
  provider_fingerprint: string
  request_key: string
  revision: number
  source_session_id: number
  spec: IntegrationSpec
  state: BindingState
  updated_at_ms: number
  workspace_id: number
}

export type IntegrationControlAction = 'enable' | 'pause' | 'poll_now'

export type IntegrationDelivery = {
  attempts: number
  binding_id: string
  created_at_ms: number
  error?: string | null
  id: string
  next_attempt_ms: number
  remote_id?: string | null
  route?: ChannelOutputRoute
  state: string
  text: string
}

export type IntegrationEvent = {
  binding_id: string
  epoch: number
  event: ExternalEvent
  execution_generation?: number | null
  execution_id?: string | null
  note?: string | null
  received_at_ms: number
  session_id?: number | null
  state: string
}

export type IntegrationMode = 'channel' | 'notify' | 'session'

export type IntegrationProvider = {
  adapter: AdapterDescription
  fingerprint: string
  plugin_id: string
}

/** One conversation, an explicit allow-list, or every conversation the adapter reports. */
export type ResourceSelector = string | { all: boolean } | string[]

export type ConversationKind = 'direct' | 'group' | 'public_channel' | 'private_channel'

export type ConversationCounts = {
  direct: number
  group: number
  public_channel: number
  private_channel: number
}

export type ConversationEntry = {
  resource: string
  kind: ConversationKind
  thread?: string | null
  session_id?: number | null
  title?: string | null
  actor?: string | null
  role?: ChannelRole | null
  created_at_ms?: number | null
}

export type PlatformAccount = {
  binding_id: string
  tenant: string
  account: string
  state: BindingState
  selector: string
  last_error?: string | null
  conversations: ConversationEntry[]
}

export type PlatformEntry = {
  platform: string
  title?: string | null
  accounts: PlatformAccount[]
  conversation_count: number
  conversation_counts: ConversationCounts
}

export type PlatformInventory = {
  platforms: PlatformEntry[]
}

export type IntegrationSpec = {
  actors?: string[]
  channel_policy?: ChannelAccessPolicy
  config: JsonValue
  daily_limit: number
  event_types: string[]
  expires_at_ms: number
  instruction: string
  max_execution_seconds: number
  mode: IntegrationMode
  poll_seconds: number
  provider: string
  resource: ResourceSelector
  thread?: string | null
  title: string
  total_limit: number
  use_source_permissions?: boolean
}

export type MemoryKind = 'fact' | 'preference' | 'episode' | 'procedure' | 'task'

export type MemoryMetadata = {
  kind: MemoryKind
  created_at_ms: number
  id: string
  importance: number
  origin: MemoryOrigin
  provenance_verified: boolean
  revision: number
  scope: MemoryScope
  source?: MemorySource
  source_suppressed: boolean
  status: MemoryStatus
  superseded_by?: string | null
  supersedes?: string[]
  updated_at_ms: number
}

/** Who last changed these bytes; only intact Derived records may be auto-updated. */
export type MemoryOrigin = 'external' | 'explicit' | 'derived'

/** Scope remains inside its owning project or instance storage boundary. */
export type MemoryScope = {
  kind: 'workspace'
} | {
  kind: 'global'
} | {
  kind: 'session'
  instance_id: string
  session_id: number
}

/** Supplied by the runtime from its authenticated callback/event context. */
export type MemorySource = {
  call_id?: string | null
  entry_id?: number | null
  instance_id: string
  session_id: number
  task_id?: string | null
}

export type MemoryStatus = 'active' | 'archived' | 'superseded'

export type Reminder = {
  created_at_ms: number
  delivered: number
  delivery_text?: string | null
  id: string
  last_error?: string | null
  next_due_ms: number
  occurrence: number
  origin_message_id: number
  request_key: string
  reserved_at_ms?: number
  reserved_day?: string | null
  retry_at_ms: number
  revision: number
  source_session_id: number
  spec: ReminderSpec
  state: ReminderState
  updated_at_ms: number
  workspace_id?: number | null
}

export type ReminderControlAction = 'pause' | 'resume' | 'cancel'

export type ReminderKind = 'reminder' | 'check_in'

export type ReminderSpec = {
  kind?: ReminderKind
  after_seconds?: number | null
  at?: string | null
  occurrences?: number
  repeat_seconds?: number | null
  text: string
}

export type ReminderState = 'active' | 'paused' | 'delivering' | 'completed' | 'cancelled'

export type UiAction = {
  key: string
  type: 'set'
  value: JsonValue
} | {
  key: string
  type: 'toggle'
} | {
  type: 'reset'
} | {
  text: JsonValue
  type: 'copy'
}

export type UiKind = 'stack' | 'row' | 'grid' | 'card' | 'text' | 'heading' | 'metric' | 'badge' | 'divider' | 'tabs' | 'select' | 'checkbox' | 'switch' | 'slider' | 'number' | 'input' | 'button' | 'progress' | 'chart' | 'table' | 'tasks' | 'timeline' | 'callout' | 'details'

export type UiNode = {
  kind: UiKind
  action?: string | null
  bind?: string | null
  children?: string[]
  id: string
  props?: Record<string, JsonValue>
  visible?: JsonValue
}

export type UiOption = {
  label: string
  value: string
}

export type UiProgram = {
  actions?: Record<string, UiAction[]>
  computed?: Record<string, JsonValue>
  data?: Record<string, JsonValue>
  nodes: UiNode[]
  root: string
  state?: Record<string, UiStateField>
}

export type UiStateField = {
  kind: UiStateKind
  initial: JsonValue
  max?: number | null
  max_length?: number
  min?: number | null
  options?: UiOption[]
  step?: number | null
}

export type UiStateKind = 'boolean' | 'number' | 'text' | 'choice' | 'selection'

export type BotSession = {
  bot_id: string
  name: string
  session_id: number
}

export type BotPersona = {
  instructions: string
}

export type BotProfile = {
  archived: boolean
  created_at_ms: number
  id: string
  name: string
  persona: BotPersona
  revision: number
  session_id: number
  updated_at_ms: number
}

export type BotList = {
  default_bot_id?: string | null
  items: BotProfile[]
}

export type BotCreate = {
  name: string
  persona?: BotPersona
  request_key: string
}

export type BotUpdate = {
  expected_revision: number
  name: string
  persona: BotPersona
  request_key: string
}

export type BotArchive = {
  expected_revision: number
  request_key: string
}

export type ChannelPolicyUpdate = {
  expected_revision: number
  policy: ChannelAccessPolicy
  request_key: string
}

export type IntegrationsSnapshot = {
  bindings: IntegrationBinding[]
  providers: IntegrationProvider[]
  source_session_id: number
  workspace_id: number
}

export type IntegrationDetail = {
  access_audit: ChannelAccessAudit[]
  binding: IntegrationBinding
  channel_sessions: ChannelSessionRecord[]
  deliveries: IntegrationDelivery[]
  events: IntegrationEvent[]
}

export type CreateIntegration = {
  request_key: string
  spec: IntegrationSpec
}

export type IntegrationControl = {
  action: IntegrationControlAction
  expected_revision: number
}

export type AssistantSnapshot = {
  memories: AssistantMemory[]
  memories_truncated: boolean
  now_ms: number
  profile: AssistantProfile
  reminders: Reminder[]
  source_session_id: number
}

export type AssistantUpdate = {
  expected_revision: number
  patch: AssistantPatch
  request_key: string
}

export type AssistantMemoryUpdate = {
  kind?: MemoryKind
  body?: string | null
  description?: string | null
  expected_sha256?: string | null
  name: string
}

export type ReminderControl = {
  action: ReminderControlAction
  expected_revision: number
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

export type InlineDocument = {
  blocks: InlineBlock[]
  title?: string
  version: number
}
