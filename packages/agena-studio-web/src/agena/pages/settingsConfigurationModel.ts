export type SettingsConfigurationFieldKind = 'string' | 'integer' | 'boolean'

export type SettingsConfigurationSection = 'defaults' | 'interface' | 'tracing' | 'runtime' | 'session'

export type SettingsConfigurationField = {
  path: string
  section: SettingsConfigurationSection
  label: string
  description: string
  kind: SettingsConfigurationFieldKind
  placeholder?: string
}

export type SettingsConfigurationDraft = {
  override: boolean
  value: string
}

export const settingsConfigurationSectionLabels: Record<SettingsConfigurationSection, string> = {
  defaults: 'Defaults',
  interface: 'Interface',
  tracing: 'Tracing',
  runtime: 'Runtime',
  session: 'Session',
}

export const settingsConfigurationFields: SettingsConfigurationField[] = [
  {
    section: 'defaults',
    path: 'providers.default',
    label: 'Default provider',
    description: 'Provider selected when a session or agent does not choose one explicitly.',
    kind: 'string',
    placeholder: 'openai',
  },
  {
    section: 'defaults',
    path: 'agents.default',
    label: 'Default agent',
    description: 'Agent profile selected for new sessions.',
    kind: 'string',
    placeholder: 'default',
  },
  {
    section: 'interface',
    path: 'ui.locale',
    label: 'Locale',
    description: 'Locale used by Agena interfaces that support localization.',
    kind: 'string',
    placeholder: 'en-US',
  },
  {
    section: 'interface',
    path: 'ui.tui.color_scheme',
    label: 'TUI color scheme',
    description: 'Terminal color capability override used by the TUI.',
    kind: 'string',
    placeholder: 'auto',
  },
  {
    section: 'interface',
    path: 'ui.tui.theme',
    label: 'TUI theme',
    description: 'Theme selected when Agena starts in the terminal.',
    kind: 'string',
    placeholder: 'default',
  },
  {
    section: 'tracing',
    path: 'tracing.filter',
    label: 'Trace filter',
    description: 'Tracing filter directive applied to runtime diagnostics.',
    kind: 'string',
    placeholder: 'agena=info',
  },
  {
    section: 'tracing',
    path: 'tracing.database',
    label: 'Trace database',
    description: 'Database destination used for persisted traces.',
    kind: 'string',
    placeholder: 'sqlite',
  },
  {
    section: 'tracing',
    path: 'tracing.adapter',
    label: 'Trace adapter',
    description: 'Tracing adapter used to publish or persist runtime spans.',
    kind: 'string',
    placeholder: 'default',
  },
  {
    section: 'runtime',
    path: 'runtime.providers.http.timeout_secs',
    label: 'Provider HTTP timeout',
    description: 'Maximum duration in seconds for a provider HTTP request.',
    kind: 'integer',
  },
  {
    section: 'runtime',
    path: 'runtime.providers.http.connect_timeout_secs',
    label: 'Provider connect timeout',
    description: 'Maximum duration in seconds for establishing provider connections.',
    kind: 'integer',
  },
  {
    section: 'runtime',
    path: 'runtime.providers.retry.max_retries',
    label: 'Request retry limit',
    description: 'Maximum provider request retries before a run fails.',
    kind: 'integer',
  },
  {
    section: 'runtime',
    path: 'runtime.providers.retry.base_delay_ms',
    label: 'Retry base delay',
    description: 'Initial retry backoff in milliseconds.',
    kind: 'integer',
  },
  {
    section: 'runtime',
    path: 'runtime.providers.retry.max_delay_ms',
    label: 'Retry maximum delay',
    description: 'Maximum retry backoff in milliseconds.',
    kind: 'integer',
  },
  {
    section: 'runtime',
    path: 'runtime.providers.stream_replay.max_retries_after_output',
    label: 'Stream replay retries',
    description: 'Retry limit after streamed output has already been observed.',
    kind: 'integer',
  },
  {
    section: 'runtime',
    path: 'runtime.providers.stream_replay.max_tracked_events',
    label: 'Tracked stream events',
    description: 'Maximum event count retained for stream replay and deduplication.',
    kind: 'integer',
  },
  {
    section: 'runtime',
    path: 'runtime.reload.enabled',
    label: 'Automatic runtime reload',
    description: 'Watch configuration inputs and reload the runtime when they change.',
    kind: 'boolean',
  },
  {
    section: 'runtime',
    path: 'runtime.reload.poll_interval_secs',
    label: 'Reload polling interval',
    description: 'Seconds between configuration change checks.',
    kind: 'integer',
  },
  {
    section: 'runtime',
    path: 'runtime.model_catalog.cache_max_age_secs',
    label: 'Model catalog cache age',
    description: 'Maximum model catalog cache age in seconds before refresh.',
    kind: 'integer',
  },
  {
    section: 'runtime',
    path: 'runtime.session.cache.max_sessions',
    label: 'Cached sessions',
    description: 'Maximum number of session runtimes held in memory.',
    kind: 'integer',
  },
  {
    section: 'runtime',
    path: 'runtime.session.cache.ttl_secs',
    label: 'Session cache TTL',
    description: 'Seconds an inactive session runtime remains cached.',
    kind: 'integer',
  },
  {
    section: 'runtime',
    path: 'runtime.session.cache.max_bytes',
    label: 'Session cache bytes',
    description: 'Approximate maximum memory footprint of cached sessions.',
    kind: 'integer',
  },
  {
    section: 'runtime',
    path: 'runtime.session.gc.enabled',
    label: 'Session garbage collection',
    description: 'Periodically remove expired session runtime state.',
    kind: 'boolean',
  },
  {
    section: 'runtime',
    path: 'runtime.session.gc.interval_secs',
    label: 'Session GC interval',
    description: 'Seconds between session garbage collection passes.',
    kind: 'integer',
  },
  {
    section: 'session',
    path: 'session.compaction.auto',
    label: 'Automatic compaction',
    description: 'Compact long sessions automatically when context limits approach.',
    kind: 'boolean',
  },
  {
    section: 'session',
    path: 'session.compaction.reserved_tokens',
    label: 'Reserved compaction tokens',
    description: 'Token headroom preserved for producing a compaction summary.',
    kind: 'integer',
  },
]

export function valueAtSettingsPath(root: unknown, path: string): unknown {
  let value = root
  for (const segment of path.split('.')) {
    if (!value || typeof value !== 'object' || Array.isArray(value)) return undefined
    if (!Object.prototype.hasOwnProperty.call(value, segment)) return undefined
    value = (value as Record<string, unknown>)[segment]
  }
  return value
}

export function formatSettingsConfigurationValue(value: unknown): string {
  if (value === undefined || value === null) return ''
  if (typeof value === 'string') return value
  if (typeof value === 'number' || typeof value === 'boolean') return String(value)
  return JSON.stringify(value)
}

export function createSettingsConfigurationDraft(fileRoot: unknown, field: SettingsConfigurationField) {
  const value = valueAtSettingsPath(fileRoot, field.path)
  return {
    override: value !== undefined,
    value: formatSettingsConfigurationValue(value),
  } satisfies SettingsConfigurationDraft
}

export function parseSettingsConfigurationDraft(
  field: SettingsConfigurationField,
  draft: SettingsConfigurationDraft,
): unknown {
  if (!draft.override) return undefined
  if (field.kind === 'string') return draft.value
  if (field.kind === 'boolean') {
    if (draft.value === 'true') return true
    if (draft.value === 'false') return false
    throw new Error(`${field.label} must be true or false.`)
  }
  const normalized = draft.value.trim()
  if (!/^-?\d+$/.test(normalized)) throw new Error(`${field.label} must be a whole number.`)
  const value = Number(normalized)
  if (!Number.isSafeInteger(value)) throw new Error(`${field.label} is outside the supported integer range.`)
  return value
}

export function settingsConfigurationDraftChanged(
  fileRoot: unknown,
  field: SettingsConfigurationField,
  draft: SettingsConfigurationDraft,
): boolean {
  const previous = valueAtSettingsPath(fileRoot, field.path)
  if (!draft.override) return previous !== undefined
  try {
    return !Object.is(previous, parseSettingsConfigurationDraft(field, draft))
  } catch {
    return true
  }
}
