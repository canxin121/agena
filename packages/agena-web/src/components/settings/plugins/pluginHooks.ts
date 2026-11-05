import { settingsText as st } from '@/i18n/settingsText'
import { i18n } from '@/i18n'
import { localizedDocumentationField } from '@/lib/pluginDocumentation'

export type PluginHookTranslation = { summary: string; help: string }
export type PluginHookFilters = {
  tools?: string[]
  commands?: string[]
  plugins?: string[]
  tags?: string[]
}

/** One distinct handler declared by the plugin author, even while stopped. */
export type PluginHookDeclaration = {
  id: string
  event: string
  summary: string
  help: string
  translations?: Record<string, PluginHookTranslation>
  priority: number
  filters: PluginHookFilters
  is_async: boolean
}

/** The manifest carries author copy; PluginInspect.hooks carries live state. */
export type PluginHookDescriptor = {
  name: string
  handler_id: string
  summary: string
  help: string
  translations: Record<string, PluginHookTranslation>
  priority: number
  filters: PluginHookFilters
  is_async: boolean
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
export type PluginHookEventGroup = {
  group: PluginHookGroup
  event: string
  hooks: PluginHookEntry[]
}

export type PluginHookEntry = {
  key: string
  name: string
  handlerId: string
  title: string
  description: string
  priority: number
  filters: PluginHookFilters
  isAsync: boolean
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
  'run.pre': {
    group: 'lifecycle',
    title: () => st('Before agent run'),
    description: () => st('Runs once when a session execution begins.'),
  },
  'run.post': {
    group: 'lifecycle',
    title: () => st('After agent run'),
    description: () => st('Runs once after session execution settles.'),
  },
  'compaction.before': {
    group: 'lifecycle',
    title: () => st('Before compaction'),
    description: () => st('Guards the start of a context compaction operation.'),
  },
  'compaction.after': {
    group: 'lifecycle',
    title: () => st('After compaction'),
    description: () => st('Observes the committed outcome of a compaction attempt.'),
  },
  'subagent.start': {
    group: 'lifecycle',
    title: () => st('Before subagent launch'),
    description: () => st('Guards a delegated agent before its initial execution.'),
  },
  'subagent.stop': {
    group: 'lifecycle',
    title: () => st('Subagent settled'),
    description: () => st('Observes a delegated agent after terminal settlement.'),
  },
  'tool.before': {
    group: 'tools',
    title: () => st('Before tool execution'),
    description: () => st('Runs before a tool is executed.'),
  },
  'tool.after': {
    group: 'tools',
    title: () => st('After tool execution'),
    description: () => st('Runs after a tool executes successfully.'),
  },
  'tool.failure': {
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
  'permission.request': {
    group: 'tools',
    title: () => st('Permission request'),
    description: () => st('Applies additional policy before interactive tool approval.'),
  },
  'chat.message': {
    group: 'conversation',
    title: () => st('Chat message'),
    description: () => st('Runs when a chat message is handled.'),
  },
  'chat.messages': {
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
  'chat.system': {
    group: 'conversation',
    title: () => st('Transform system prompt'),
    description: () => st('Transforms the system instructions sent to the model.'),
  },
  'model.request.before': {
    group: 'conversation',
    title: () => st('Before model request'),
    description: () => st('Guards the model request before provider I/O.'),
  },
  'model.request.after': {
    group: 'conversation',
    title: () => st('After model request'),
    description: () => st('Observes the settled model request status and usage.'),
  },
  'model.request.error': {
    group: 'conversation',
    title: () => st('Model request failure'),
    description: () => st('May request a bounded retry of a failed, side-effect-free round.'),
  },
  'prompt.submit': {
    group: 'conversation',
    title: () => st('User prompt submitted'),
    description: () => st('Runs when a user submits a prompt.'),
  },
  'shell.before': {
    group: 'commands',
    title: () => st('Before command execution'),
    description: () => st('Runs before a command is executed.'),
  },
  'shell.after': {
    group: 'commands',
    title: () => st('After command execution'),
    description: () => st('Runs after a command is executed.'),
  },
  'shell.env': {
    group: 'commands',
    title: () => st('Shell environment'),
    description: () => st('Runs when the shell environment is prepared.'),
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
  'config.resolved': {
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
 * A subscription is an event; the author declares the individual functions
 * behind that event. Keep one entry per function, even when two handlers
 * subscribe to the same event. Registration state comes exclusively from Host.
 */
export function buildPluginHookEntries(
  manifestHooks: unknown,
  registrations: readonly PluginHookRegistration[] | null | undefined,
  pluginState: string | null | undefined,
  manifestHandlers?: readonly PluginHookDeclaration[] | null,
  locale: string = i18n.global.locale.value,
): PluginHookEntry[] {
  type Entry = {
    name: string
    declared: boolean
    declaration?: PluginHookDeclaration
    registration?: PluginHookRegistration
    descriptor?: PluginHookDescriptor
  }
  const entries = new Map<string, Entry>()
  const declaredEvents = new Set(
    Array.isArray(manifestHooks) ? manifestHooks.filter((name): name is string => typeof name === 'string') : [],
  )

  for (const declaration of manifestHandlers || []) {
    const name = normalizedName(declaration.event)
    const id = normalizedName(declaration.id)
    if (!name || !id || !normalizedName(declaration.summary) || !normalizedName(declaration.help)) continue
    entries.set(`handler:${name}:${id}`, {
      name,
      declared: true,
      declaration,
    })
  }

  if (Array.isArray(registrations)) {
    for (const registration of registrations) {
      for (const descriptor of registration.hooks || []) {
        const name = normalizedName(descriptor.name)
        const id = normalizedName(descriptor.handler_id)
        if (!name || !id || !normalizedName(descriptor.summary) || !normalizedName(descriptor.help)) continue
        const key = `handler:${name}:${id}`
        const previous = entries.get(key)
        if (!previous?.descriptor || (!previous.descriptor.enabled && descriptor.enabled)) {
          entries.set(key, {
            name,
            declared: previous?.declared || declaredEvents.has(name),
            declaration: previous?.declaration,
            registration,
            descriptor,
          })
        }
      }
    }
  }

  const runtimeState = String(pluginState || '').toLowerCase()
  return [...entries.entries()]
    .map(([key, data]): PluginHookEntry | null => {
      const { name, declaration, descriptor, registration } = data
      const docs = HOOK_DOCS[name]
      const translations = declaration?.translations || descriptor?.translations
      const summary = localizedDocumentationField(
        declaration?.summary ?? descriptor?.summary,
        translations,
        'summary',
        locale,
      )
      const help = localizedDocumentationField(
        declaration?.help ?? descriptor?.help,
        translations,
        'help',
        locale,
      )
      const handlerId = declaration?.id || descriptor?.handler_id
      if (!handlerId || !summary || !help) return null
      return {
        key,
        name,
        handlerId,
        title: summary,
        description: help,
        priority: declaration?.priority ?? descriptor?.priority ?? 0,
        filters: declaration?.filters ?? descriptor?.filters ?? {},
        isAsync: declaration?.is_async ?? descriptor?.is_async ?? false,
        group: docs?.group || 'integration',
        state: !descriptor
          ? 'declared'
          : !descriptor.enabled
            ? 'disabled'
            : runtimeState === 'running'
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
    .filter((entry): entry is PluginHookEntry => entry !== null)
    .sort((a, b) => {
      return PLUGIN_HOOK_GROUPS.indexOf(a.group) - PLUGIN_HOOK_GROUPS.indexOf(b.group)
        || a.name.localeCompare(b.name)
        || (b.priority ?? 0) - (a.priority ?? 0)
        || (a.handlerId || '').localeCompare(b.handlerId || '')
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
      entry.handlerId || '',
      entry.title,
      entry.description,
      ...Object.values(entry.filters || {}).flat(),
      pluginHookGroupLabel(entry.group),
      pluginHookStateLabel(entry.state),
    ].some((part) => part.toLocaleLowerCase().includes(term)),
  )
}

/** One section per event, with distinct, priority-ordered handlers. */
export function groupPluginHookEntries(entries: readonly PluginHookEntry[]): PluginHookEventGroup[] {
  return PLUGIN_HOOK_GROUPS.flatMap((group) => {
    const events = new Map<string, PluginHookEntry[]>()
    for (const hook of entries) {
      if (hook.group !== group) continue
      const handlers = events.get(hook.name) || []
      handlers.push(hook)
      events.set(hook.name, handlers)
    }
    return [...events.entries()]
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([event, hooks]) => ({
        group,
        event,
        hooks: hooks.sort((a, b) => b.priority - a.priority || a.handlerId.localeCompare(b.handlerId)),
      }))
  })
}
