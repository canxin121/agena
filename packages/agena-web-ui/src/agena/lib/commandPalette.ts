import { computed, ref, type ComputedRef, type Ref } from 'vue'
import type { Router } from 'vue-router'

import { buildRuntimeSectionPath, sectionTabNavigationItems } from '../pages/runtimePageStateModel'
import { parseUsageCommandArgs } from '../pages/usageStatsModel'
import type { PluginStudioCommand, PluginUiAction, RuntimeSkill } from './agenaApi'

export type CommandSource =
  | 'navigation'
  | 'runtime-skill'
  | 'runtime-command'
  | 'plugin-studio'
  | 'chat-action'
  | 'workspace-action'

export type CommandContext = {
  input: string
  args: string[]
}

export type CommandRunResult = {
  submitText?: string
  notice?: string
}

export type CommandItem = {
  id: string
  title: string
  description: string
  category: string
  source: CommandSource
  pluginId?: string
  action?: PluginUiAction
  slash?: string
  slashAliases?: string[]
  aliases?: string[]
  usage?: string
  sourceLabel?: string
  run: (context?: CommandContext) => void | CommandRunResult | Promise<void | CommandRunResult>
}

export type CommandPaletteState = {
  open: Ref<boolean>
  query: Ref<string>
  items: ComputedRef<CommandItem[]>
  filteredItems: ComputedRef<CommandItem[]>
  highlightedIndex: Ref<number>
  openPalette: () => void
  closePalette: () => void
  togglePalette: () => void
  moveHighlight: (delta: number) => void
  runHighlighted: () => Promise<boolean>
  runSlashCommand: (input: string) => Promise<{ matched: boolean; command?: CommandItem; result?: CommandRunResult }>
  runLocalSlashCommand: (
    input: string,
  ) => Promise<{ matched: boolean; command?: CommandItem; result?: CommandRunResult }>
}

export type CommandPaletteCatalogInput = {
  router: Router
  runtimeSkills: ComputedRef<RuntimeSkill[]>
  runtimeCommands: ComputedRef<RuntimeSkill[]>
  pluginCommands?: ComputedRef<PluginStudioCommand[]>
  localCommands?: ComputedRef<CommandItem[]>
  onSelectRuntimeEntry?: (entry: {
    kind: 'skill' | 'command'
    item: RuntimeSkill
    context?: CommandContext
  }) => void | CommandRunResult | Promise<void | CommandRunResult>
  onRunPluginAction?: (entry: {
    command: PluginStudioCommand
    context?: CommandContext
  }) => void | CommandRunResult | Promise<void | CommandRunResult>
}

function normalize(value: string): string {
  return String(value || '')
    .trim()
    .toLowerCase()
}

export function parseCommandInput(input: string): { slash: string; args: string[] } {
  const trimmed = String(input || '').trim()
  if (!trimmed.startsWith('/')) {
    return { slash: '', args: [] }
  }
  const tokens = trimmed.split(/\s+/).filter(Boolean)
  return {
    slash: normalize(tokens[0] || ''),
    args: tokens.slice(1),
  }
}

function schemaType(schema: unknown): string | null {
  if (!schema || typeof schema !== 'object' || Array.isArray(schema)) return null
  const value = (schema as { type?: unknown }).type
  if (typeof value === 'string') return value
  if (Array.isArray(value)) {
    const match = value.find((entry): entry is string => typeof entry === 'string' && entry !== 'null')
    return match || null
  }
  return null
}

function schemaProperties(schema: unknown): Record<string, unknown> {
  if (!schema || typeof schema !== 'object' || Array.isArray(schema)) return {}
  const properties = (schema as { properties?: unknown }).properties
  if (!properties || typeof properties !== 'object' || Array.isArray(properties)) return {}
  return properties as Record<string, unknown>
}

function schemaPropertyNames(schema: unknown): string[] {
  return Object.keys(schemaProperties(schema))
}

function schemaPropertyAliasMap(schema: unknown): Record<string, string> {
  const properties = schemaProperties(schema)
  const aliases: Record<string, string> = {}
  for (const [name, property] of Object.entries(properties)) {
    if (!property || typeof property !== 'object' || Array.isArray(property)) continue
    const values = (property as { 'x-agena-aliases'?: unknown })['x-agena-aliases']
    if (!Array.isArray(values)) continue
    for (const value of values) {
      if (typeof value !== 'string' || !value) continue
      aliases[value] = name
    }
  }
  return aliases
}

function parseSchemaLiteral(raw: string, schema: unknown): unknown {
  const type = schemaType(schema)
  if (type === 'string') return raw
  if (type === 'integer' || type === 'number') {
    const value = Number(raw)
    if (!Number.isNaN(value) && Number.isFinite(value)) {
      return type === 'integer' ? Math.trunc(value) : value
    }
  }
  if (type === 'boolean') {
    if (raw === 'true') return true
    if (raw === 'false') return false
  }
  if (type === 'null' && raw === 'null') {
    return null
  }
  return undefined
}

function parseLooseJsonLiteral(raw: string): unknown {
  try {
    return JSON.parse(raw)
  } catch {
    return raw
  }
}

function parsePropertyLiteral(raw: string, schema: unknown): unknown {
  const literal = parseSchemaLiteral(raw, schema)
  return literal !== undefined ? literal : parseLooseJsonLiteral(raw)
}

function parseKeyValueArgs(
  args: string[],
  properties: Record<string, unknown>,
  aliases: Record<string, string>,
): Record<string, unknown> | null {
  if (!args.length) return null
  const output: Record<string, unknown> = {}
  for (const token of args) {
    const separator = token.indexOf('=')
    if (separator <= 0) return null
    const rawKey = token.slice(0, separator).trim()
    if (!rawKey) return null
    const key = rawKey in properties ? rawKey : aliases[rawKey]
    if (!key || !(key in properties)) return null
    const rawValue = token.slice(separator + 1)
    output[key] = parsePropertyLiteral(rawValue, properties[key])
  }
  return Object.keys(output).length ? output : null
}

export function buildPluginCommandPayload(command: PluginStudioCommand, context?: CommandContext): unknown {
  const raw = context?.args.join(' ').trim() || ''
  if (!raw) return undefined

  const schema = command.input_schema
  if (!schema) {
    return { args: raw }
  }

  const type = schemaType(schema)
  const properties = schemaProperties(schema)
  const propertyNames = schemaPropertyNames(schema)
  const propertyAliases = schemaPropertyAliasMap(schema)

  try {
    const parsed = JSON.parse(raw)
    if (type !== 'object') {
      return parsed
    }
    if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
      return parsed
    }
    if (propertyNames.length === 1) {
      return { [propertyNames[0]]: parsed }
    }
  } catch {
    if (propertyNames.length === 1 && type === 'object') {
      const key = propertyNames[0]
      return { [key]: parsePropertyLiteral(raw, properties[key]) }
    }
    if (propertyNames.length > 1 && type === 'object') {
      const payload = parseKeyValueArgs(context?.args || [], properties, propertyAliases)
      if (payload) {
        return payload
      }
    }
    const literal = parseSchemaLiteral(raw, schema)
    if (literal !== undefined) {
      return literal
    }
    return { args: raw }
  }

  if (propertyNames.length === 1 && type === 'object') {
    const key = propertyNames[0]
    return { [key]: parsePropertyLiteral(raw, properties[key]) }
  }
  if (propertyNames.length > 1 && type === 'object') {
    const payload = parseKeyValueArgs(context?.args || [], properties, propertyAliases)
    if (payload) {
      return payload
    }
  }
  const literal = parseSchemaLiteral(raw, schema)
  if (literal !== undefined) {
    return literal
  }
  return { args: raw }
}

export function sourceLabel(source: CommandSource): string {
  switch (source) {
    case 'navigation':
      return 'Navigation'
    case 'runtime-skill':
      return 'Runtime Skill'
    case 'runtime-command':
      return 'Runtime Command'
    case 'plugin-studio':
      return 'Plugin UI'
    case 'chat-action':
      return 'Chat Action'
    case 'workspace-action':
      return 'Workspace Action'
  }
}

export function commandSearchText(item: CommandItem): string {
  return [
    item.title,
    item.description,
    item.category,
    item.slash || '',
    ...(item.slashAliases || []),
    item.usage || '',
    item.sourceLabel || sourceLabel(item.source),
    ...(item.aliases || []),
  ]
    .join(' ')
    .toLowerCase()
}

export function commandMatchesSlash(item: CommandItem, slash: string): boolean {
  const normalizedSlash = normalize(slash)
  if (!normalizedSlash) return false
  return [item.slash, ...(item.slashAliases || [])].some((value) => normalize(value || '') === normalizedSlash)
}

function commandSlashStartsWith(item: CommandItem, slash: string): boolean {
  const normalizedSlash = normalize(slash)
  if (!normalizedSlash) return false
  return [item.slash, ...(item.slashAliases || [])].some((value) => normalize(value || '').startsWith(normalizedSlash))
}

function buildNavigationCommands(router: Router): CommandItem[] {
  const sectionCommands: CommandItem[] = [
    {
      id: 'nav.chat',
      title: 'Open Chat',
      description: 'Go to the main chat workspace and active session view.',
      category: 'Navigation',
      source: 'navigation',
      slash: '/chat',
      aliases: ['session', 'messages'],
      usage: '/chat',
      sourceLabel: sourceLabel('navigation'),
      run: async () => {
        await router.push('/chat')
      },
    },
    {
      id: 'nav.workspace',
      title: 'Open Workspace',
      description: 'Browse workspaces, files, and project configuration entry points.',
      category: 'Navigation',
      source: 'navigation',
      slash: '/workspace',
      aliases: ['files', 'project'],
      usage: '/workspace',
      sourceLabel: sourceLabel('navigation'),
      run: async () => {
        await router.push('/workspace')
      },
    },
    {
      id: 'nav.runtime',
      title: 'Open Runtime',
      description: 'Inspect runtime overview, workflow, MCP, LSP, skills, and operator status.',
      category: 'Navigation',
      source: 'navigation',
      slash: '/runtime',
      aliases: ['workflow', 'operator'],
      usage: '/runtime',
      sourceLabel: sourceLabel('navigation'),
      run: async () => {
        await router.push('/runtime')
      },
    },
    {
      id: 'nav.plugins',
      title: 'Open Plugins',
      description: 'Inspect installed plugins, plugin detail, and retained logs.',
      category: 'Navigation',
      source: 'navigation',
      slash: '/plugins',
      slashAliases: ['/plugin'],
      aliases: ['plugin logs', 'marketplace'],
      usage: '/plugins [query]',
      sourceLabel: sourceLabel('navigation'),
      run: async (context) => {
        const search = context?.args.join(' ').trim() || ''
        await router.push({ path: '/plugins/installed', query: search ? { search } : undefined })
      },
    },
    {
      id: 'nav.settings',
      title: 'Open Settings',
      description: 'Manage credentials, runtime configuration, plugins, and permission rules.',
      category: 'Navigation',
      source: 'navigation',
      slash: '/settings',
      aliases: ['credentials', 'permissions'],
      usage: '/settings [query]',
      sourceLabel: sourceLabel('navigation'),
      run: async (context) => {
        const query = context?.args.join(' ').trim() || ''
        const normalized = query.toLowerCase()
        const tab =
          normalized === 'providers' || normalized === 'provider'
            ? 'providers'
            : normalized === 'agents' || normalized === 'agent'
              ? 'agents'
              : normalized === 'plugins' || normalized === 'plugin'
                ? 'plugins'
                : normalized === 'permissions' || normalized === 'permission' || normalized === 'guardrails'
                  ? 'permissions'
                  : normalized === 'memory' || normalized === 'mem'
                    ? 'memory'
                    : query
                      ? 'configuration'
                      : 'providers'
        await router.push({
          path: buildRuntimeSectionPath('settings', tab),
          query: query && tab === 'configuration' ? { search: query } : undefined,
        })
      },
    },
    {
      id: 'nav.usage',
      title: 'Open Usage',
      description: 'Inspect token, cost, provider, model, and session usage analytics.',
      category: 'Navigation',
      source: 'navigation',
      slash: '/usage',
      slashAliases: ['/stats', '/analytics'],
      aliases: ['cost', 'tokens', 'analytics'],
      usage: '/usage [today|yesterday|7d|14d|30d|90d|month|year|all] [--provider ID] [--model ID] [--no-subagents]',
      sourceLabel: sourceLabel('navigation'),
      run: async (context) => {
        const plan = parseUsageCommandArgs(context?.args || [])
        if (plan.kind === 'error') return { notice: plan.message }
        await router.push({ path: '/usage', query: plan.query })
      },
    },
  ]

  const tabCommands: CommandItem[] = sectionTabNavigationItems.map((item) => ({
    id: item.id,
    title: item.title,
    description: item.description,
    category: 'Navigation',
    source: 'navigation',
    slash: item.slash,
    aliases: item.aliases,
    usage: item.slash,
    sourceLabel: sourceLabel('navigation'),
    run: async () => {
      await router.push(buildRuntimeSectionPath(item.section, item.tab))
    },
  }))

  return [...sectionCommands, ...tabCommands]
}

function matchesQuery(item: CommandItem, query: string): boolean {
  const q = normalize(query)
  if (!q) return true

  if (q.startsWith('/')) {
    const parsed = parseCommandInput(query)
    if (parsed.slash) {
      if (commandMatchesSlash(item, parsed.slash)) return true
      if (!parsed.args.length && commandSlashStartsWith(item, parsed.slash)) return true
      if (!item.slash && !item.slashAliases?.length) return false
    }
  }

  return commandSearchText(item).includes(q)
}

function skillToCommand(skill: RuntimeSkill, source: 'runtime-skill' | 'runtime-command'): Omit<CommandItem, 'run'> {
  return {
    id: `${source}.${skill.name}`,
    title: skill.name,
    description:
      skill.description || (source === 'runtime-command' ? 'Runtime-discovered command.' : 'Runtime-discovered skill.'),
    category: source === 'runtime-command' ? 'Runtime Commands' : 'Runtime Skills',
    source,
    slash: `/${skill.name}`,
    slashAliases: skill.aliases.map((alias) => `/${alias}`),
    aliases: skill.aliases,
    usage: `/${skill.name}`,
    sourceLabel: sourceLabel(source),
  }
}

function pluginStudioCommandToCommand(command: PluginStudioCommand): Omit<CommandItem, 'run'> {
  const slash = command.slash?.trim() || undefined
  return {
    id: `plugin-studio.${command.plugin_id}.${command.id}`,
    title: command.title,
    description: command.description || 'Plugin-provided Studio command.',
    category: command.category || 'Plugin',
    source: 'plugin-studio',
    pluginId: command.plugin_id,
    action: command.action,
    slash,
    aliases: command.aliases || [],
    usage: command.usage || slash,
    sourceLabel: `${sourceLabel('plugin-studio')} · ${command.plugin_id}`,
  }
}

function isLocalCommandSource(source: CommandSource): boolean {
  return source === 'navigation' || source === 'chat-action' || source === 'workspace-action'
}

export function createCommandPaletteItems(input: CommandPaletteCatalogInput): ComputedRef<CommandItem[]> {
  return computed<CommandItem[]>(() => {
    const navigation = buildNavigationCommands(input.router)
    const local = input.localCommands?.value || []
    const pluginCommands = (input.pluginCommands?.value || [])
      .filter((command) => command.location === 'command_palette' || command.location === 'chat_palette')
      .map((command) => ({
        ...pluginStudioCommandToCommand(command),
        run: (context?: CommandContext) => input.onRunPluginAction?.({ command, context }),
      }))
    const runtimeSkillCommands = input.runtimeSkills.value.map((skill) => ({
      ...skillToCommand(skill, 'runtime-skill'),
      run: (context?: CommandContext) => input.onSelectRuntimeEntry?.({ kind: 'skill', item: skill, context }),
    }))
    const runtimeCommands = input.runtimeCommands.value.map((command) => ({
      ...skillToCommand(command, 'runtime-command'),
      run: (context?: CommandContext) => input.onSelectRuntimeEntry?.({ kind: 'command', item: command, context }),
    }))
    return [...navigation, ...local, ...pluginCommands, ...runtimeCommands, ...runtimeSkillCommands]
  })
}

export function createCommandPalette(input: CommandPaletteCatalogInput): CommandPaletteState {
  const open = ref(false)
  const query = ref('')
  const highlightedIndex = ref(0)

  const items = createCommandPaletteItems(input)

  const filteredItems = computed(() => items.value.filter((item) => matchesQuery(item, query.value)))

  function openPalette() {
    open.value = true
  }

  function closePalette() {
    open.value = false
    query.value = ''
    highlightedIndex.value = 0
  }

  function togglePalette() {
    if (open.value) {
      closePalette()
      return
    }
    openPalette()
  }

  function moveHighlight(delta: number) {
    const count = filteredItems.value.length
    if (!count) {
      highlightedIndex.value = 0
      return
    }
    highlightedIndex.value = (highlightedIndex.value + delta + count) % count
  }

  async function runHighlighted(): Promise<boolean> {
    const item = filteredItems.value[highlightedIndex.value]
    if (!item) return false

    const inputText = String(query.value || '').trim()
    const parsed = parseCommandInput(inputText)
    const context =
      parsed.slash && commandMatchesSlash(item, parsed.slash) ? { input: inputText, args: parsed.args } : undefined

    await item.run(context)
    closePalette()
    return true
  }

  async function runSlashCommand(inputText: string): Promise<{
    matched: boolean
    command?: CommandItem
    result?: CommandRunResult
  }> {
    const parsed = parseCommandInput(inputText)
    if (!parsed.slash) return { matched: false }

    const command = items.value.find((item) => commandMatchesSlash(item, parsed.slash))
    if (!command) return { matched: false }
    const result = (await command.run({ input: String(inputText || '').trim(), args: parsed.args })) || undefined
    return { matched: true, command, result }
  }

  async function runLocalSlashCommand(inputText: string): Promise<{
    matched: boolean
    command?: CommandItem
    result?: CommandRunResult
  }> {
    const parsed = parseCommandInput(inputText)
    if (!parsed.slash) return { matched: false }

    const command = items.value.find(
      (item) => isLocalCommandSource(item.source) && commandMatchesSlash(item, parsed.slash),
    )
    if (!command) return { matched: false }
    const result = (await command.run({ input: String(inputText || '').trim(), args: parsed.args })) || undefined
    return { matched: true, command, result }
  }

  return {
    open,
    query,
    items,
    filteredItems,
    highlightedIndex,
    openPalette,
    closePalette,
    togglePalette,
    moveHighlight,
    runHighlighted,
    runSlashCommand,
    runLocalSlashCommand,
  }
}
