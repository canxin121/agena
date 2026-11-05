import { settingsText as st } from '@/i18n/settingsText'

/** This data comes from PluginInspect.hooks, not from a second API request. */
export type PluginHookDescriptor = {
  name: string
  enabled: boolean
  trust_level?: string
  trust_status?: string
  source?: string
  source_path?: string | null
  current_hash?: string | null
}

export type PluginHookRegistration = {
  plugin_id: string
  trust_level?: string
  trust_status?: string
  provenance?: string[]
  source?: string
  source_path?: string | null
  manifest_hash?: string | null
  hooks?: PluginHookDescriptor[]
}

export type PluginHookGroup = 'lifecycle' | 'tools' | 'conversation' | 'commands' | 'integration'
export type PluginHookState = 'active' | 'inactive' | 'disabled' | 'declared'

export type PluginHookEntry = {
  name: string
  title: string
  description: string
  group: PluginHookGroup
  state: PluginHookState
  declared: boolean
  registered: boolean
  source?: string
  sourcePath?: string | null
  trustLevel?: string
  trustStatus?: string
  provenance?: string[]
  currentHash?: string | null
}

type HookDocumentation = {
  group: PluginHookGroup
  title: () => string
  description: () => string
}

/**
 * Keep stable protocol identifiers visible even when a friendly translation exists.
 * These identifiers match HookSubscription::names() in agena-plugin-sdk.
 */
const HOOK_DOCS: Record<string, HookDocumentation> = {
  init: {
    group: 'lifecycle',
    title: () => st('Plugin initialization'),
    description: () => st('Runs when the plugin is initialized.'),
  },
  shutdown: {
    group: 'lifecycle',
    title: () => st('Plugin shutdown'),
    description: () => st('Runs when the plugin is shutting down.'),
  },
  'session.start': {
    group: 'lifecycle',
    title: () => st('Session started'),
    description: () => st('Runs when a session starts.'),
  },
  'session.end': {
    group: 'lifecycle',
    title: () => st('Session ended'),
    description: () => st('Runs when a session ends.'),
  },
  'agent.stop': {
    group: 'lifecycle',
    title: () => st('Agent stopped'),
    description: () => st('Runs when the agent stops normally.'),
  },
  'agent.cancel': {
    group: 'lifecycle',
    title: () => st('Agent cancelled'),
    description: () => st('Runs when agent execution is cancelled.'),
  },
  pre_run: {
    group: 'lifecycle',
    title: () => st('Before agent run'),
    description: () => st('Runs before an agent run begins.'),
  },
  post_run: {
    group: 'lifecycle',
    title: () => st('After agent run'),
    description: () => st('Runs after an agent run completes.'),
  },
  'tool.execute.before': {
    group: 'tools',
    title: () => st('Before tool execution'),
    description: () => st('Runs before a tool is executed.'),
  },
  'tool.execute.after': {
    group: 'tools',
    title: () => st('After tool execution'),
    description: () => st('Runs after a tool executes successfully.'),
  },
  'tool.execute.failure': {
    group: 'tools',
    title: () => st('Tool execution failed'),
    description: () => st('Runs when tool execution fails.'),
  },
  'tool.invoke': {
    group: 'tools',
    title: () => st('Tool invocation'),
    description: () => st('Handles a tool invocation.'),
  },
  'tool.invoke.stream': {
    group: 'tools',
    title: () => st('Streaming tool invocation'),
    description: () => st('Handles a streaming tool invocation.'),
  },
  'tool.definition': {
    group: 'tools',
    title: () => st('Tool definition'),
    description: () => st('Runs when tool definitions are processed.'),
  },
  'chat.message': {
    group: 'conversation',
    title: () => st('Chat message'),
    description: () => st('Runs when a chat message is handled.'),
  },
  'chat.messages.transform': {
    group: 'conversation',
    title: () => st('Transform chat messages'),
    description: () => st('Transforms the messages sent to the model.'),
  },
  'chat.params': {
    group: 'conversation',
    title: () => st('Chat parameters'),
    description: () => st('Runs when chat request parameters are prepared.'),
  },
  'chat.headers': {
    group: 'conversation',
    title: () => st('Chat request headers'),
    description: () => st('Runs when chat request headers are prepared.'),
  },
  'chat.system.transform': {
    group: 'conversation',
    title: () => st('Transform system prompt'),
    description: () => st('Transforms the system instructions sent to the model.'),
  },
  'user.prompt.submit': {
    group: 'conversation',
    title: () => st('User prompt submitted'),
    description: () => st('Runs when a user submits a prompt.'),
  },
  'command.execute.before': {
    group: 'commands',
    title: () => st('Before command execution'),
    description: () => st('Runs before a command is executed.'),
  },
  'command.execute.after': {
    group: 'commands',
    title: () => st('After command execution'),
    description: () => st('Runs after a command is executed.'),
  },
  'shell.env': {
    group: 'commands',
    title: () => st('Shell environment'),
    description: () => st('Runs when the shell environment is prepared.'),
  },
  event: {
    group: 'integration',
    title: () => st('Event received'),
    description: () => st('Receives events published by the host.'),
  },
  auth: {
    group: 'integration',
    title: () => st('Authentication'),
    description: () => st('Participates in authentication handling.'),
  },
  'provider.list': {
    group: 'integration',
    title: () => st('Provider discovery'),
    description: () => st('Runs when available providers are listed.'),
  },
  notification: {
    group: 'integration',
    title: () => st('Notification'),
    description: () => st('Runs when the host delivers a notification.'),
  },
  config: {
    group: 'integration',
    title: () => st('Configuration resolved'),
    description: () => st('Runs when configuration is resolved.'),
  },
}

export const PLUGIN_HOOK_GROUPS: readonly PluginHookGroup[] = [
  'lifecycle',
  'tools',
  'conversation',
  'commands',
  'integration',
]

export function pluginHookGroupLabel(group: PluginHookGroup): string {
  switch (group) {
    case 'lifecycle':
      return st('Lifecycle hooks')
    case 'tools':
      return st('Tool hooks')
    case 'conversation':
      return st('Conversation hooks')
    case 'commands':
      return st('Command hooks')
    case 'integration':
      return st('Integration hooks')
  }
}

export function pluginHookStateLabel(state: PluginHookState): string {
  switch (state) {
    case 'active':
      return st('Registered and active')
    case 'inactive':
      return st('Registered, plugin not running')
    case 'disabled':
      return st('Registration disabled')
    case 'declared':
      return st('Declared only')
  }
}

/** Translate trust statuses while keeping the raw wire value available. */
export function pluginHookTrustStatusLabel(status: string): string {
  switch (status.toLowerCase()) {
    case 'trusted':
      return st('Trusted')
    case 'verified':
      return st('Verified')
    case 'remote':
      return st('Remote')
    case 'untrusted':
      return st('Untrusted')
    default:
      return status
  }
}

function normalizedName(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

/**
 * Manifest subscriptions describe intent; the inspect registrations describe
 * runtime state. A stopped or failed plugin must never be labeled active.
 * Unknown/new hook identifiers remain visible instead of silently disappearing.
 */
export function buildPluginHookEntries(
  manifestHooks: unknown,
  registrations: readonly PluginHookRegistration[] | null | undefined,
  pluginState: string | null | undefined,
): PluginHookEntry[] {
  const names = new Map<
    string,
    { declared: boolean; registration?: PluginHookRegistration; descriptor?: PluginHookDescriptor }
  >()

  if (Array.isArray(manifestHooks)) {
    for (const value of manifestHooks) {
      const name = normalizedName(value)
      if (name) names.set(name, { declared: true })
    }
  }

  if (Array.isArray(registrations)) {
    for (const registration of registrations) {
      for (const descriptor of registration.hooks || []) {
        const name = normalizedName(descriptor.name)
        if (!name) continue
        const previous = names.get(name)
        // Prefer the enabled descriptor if multiple registrations have the same name.
        if (!previous?.descriptor || (!previous.descriptor.enabled && descriptor.enabled)) {
          names.set(name, {
            declared: previous?.declared || false,
            registration,
            descriptor,
          })
        }
      }
    }
  }

  const state = String(pluginState || '').toLowerCase()
  return [...names.entries()]
    .map(([name, data]): PluginHookEntry => {
      const doc = HOOK_DOCS[name]
      const descriptor = data.descriptor
      const registration = data.registration
      return {
        name,
        title: doc?.title() || name,
        description: doc?.description() || st('Unknown hook type.'),
        group: doc?.group || 'integration',
        state: !descriptor
          ? 'declared'
          : !descriptor.enabled
            ? 'disabled'
            : state === 'running'
              ? 'active'
              : 'inactive',
        declared: data.declared,
        registered: Boolean(descriptor),
        source: descriptor?.source || registration?.source,
        sourcePath: descriptor?.source_path || registration?.source_path,
        trustLevel: descriptor?.trust_level || registration?.trust_level,
        trustStatus: descriptor?.trust_status || registration?.trust_status,
        provenance: registration?.provenance,
        currentHash: descriptor?.current_hash || registration?.manifest_hash,
      }
    })
    .sort((a, b) => {
      const group = PLUGIN_HOOK_GROUPS.indexOf(a.group) - PLUGIN_HOOK_GROUPS.indexOf(b.group)
      return group || a.name.localeCompare(b.name)
    })
}

export function filterPluginHookEntries(entries: readonly PluginHookEntry[], query: string): PluginHookEntry[] {
  const term = String(query || '')
    .trim()
    .toLocaleLowerCase()
  if (!term) return [...entries]
  return entries.filter((entry) =>
    [
      entry.name,
      entry.title,
      entry.description,
      pluginHookGroupLabel(entry.group),
      pluginHookStateLabel(entry.state),
    ].some((part) => part.toLocaleLowerCase().includes(term)),
  )
}
