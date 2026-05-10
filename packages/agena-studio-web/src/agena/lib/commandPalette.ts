import { computed, ref, type ComputedRef, type Ref } from 'vue'
import type { Router } from 'vue-router'

import {
  buildRuntimeSectionPath,
  sectionTabNavigationItems,
} from '../pages/runtimePageStateModel'
import type { RuntimeSkill } from './agenaApi'

export type CommandSource = 'navigation' | 'runtime-skill' | 'runtime-command' | 'chat-action' | 'workspace-action'

export type CommandContext = {
  input: string
  args: string[]
}

export type CommandItem = {
  id: string
  title: string
  description: string
  category: string
  source: CommandSource
  slash?: string
  aliases?: string[]
  usage?: string
  sourceLabel?: string
  run: (context?: CommandContext) => void | Promise<void>
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
  runSlashCommand: (input: string) => Promise<{ matched: boolean; command?: CommandItem }>
  runLocalSlashCommand: (input: string) => Promise<{ matched: boolean; command?: CommandItem }>
}

export type CommandPaletteCatalogInput = {
  router: Router
  runtimeSkills: ComputedRef<RuntimeSkill[]>
  runtimeCommands: ComputedRef<RuntimeSkill[]>
  localCommands?: ComputedRef<CommandItem[]>
  onSelectRuntimeEntry?: (entry: { kind: 'skill' | 'command'; item: RuntimeSkill }) => void | Promise<void>
}

function normalize(value: string): string {
  return String(value || '').trim().toLowerCase()
}

function parseCommandInput(input: string): { slash: string; args: string[] } {
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

function sourceLabel(source: CommandSource): string {
  switch (source) {
    case 'navigation':
      return 'Navigation'
    case 'runtime-skill':
      return 'Runtime Skill'
    case 'runtime-command':
      return 'Runtime Command'
    case 'chat-action':
      return 'Chat Action'
    case 'workspace-action':
      return 'Workspace Action'
  }
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
      aliases: ['plugin logs', 'marketplace'],
      usage: '/plugins',
      sourceLabel: sourceLabel('navigation'),
      run: async () => {
        await router.push('/plugins')
      },
    },
    {
      id: 'nav.settings',
      title: 'Open Settings',
      description: 'Manage credentials, permission rules, and desktop configuration.',
      category: 'Navigation',
      source: 'navigation',
      slash: '/settings',
      aliases: ['credentials', 'permissions'],
      usage: '/settings',
      sourceLabel: sourceLabel('navigation'),
      run: async () => {
        await router.push('/settings')
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
      return normalize(item.slash || '') === parsed.slash
    }
  }

  const haystack = [
    item.title,
    item.description,
    item.category,
    item.slash || '',
    item.usage || '',
    ...(item.aliases || []),
  ]
    .join(' ')
    .toLowerCase()
  return haystack.includes(q)
}

function skillToCommand(skill: RuntimeSkill, source: 'runtime-skill' | 'runtime-command'): Omit<CommandItem, 'run'> {
  return {
    id: `${source}.${skill.name}`,
    title: skill.name,
    description: skill.description || (source === 'runtime-command' ? 'Runtime-discovered command.' : 'Runtime-discovered skill.'),
    category: source === 'runtime-command' ? 'Runtime Commands' : 'Runtime Skills',
    source,
    slash: `/${skill.name}`,
    aliases: skill.aliases,
    usage: `/${skill.name}`,
    sourceLabel: sourceLabel(source),
  }
}

function isLocalCommandSource(source: CommandSource): boolean {
  return source === 'navigation' || source === 'chat-action' || source === 'workspace-action'
}

export function createCommandPaletteItems(input: CommandPaletteCatalogInput): ComputedRef<CommandItem[]> {
  return computed<CommandItem[]>(() => {
    const navigation = buildNavigationCommands(input.router)
    const local = input.localCommands?.value || []
    const runtimeSkillCommands = input.runtimeSkills.value.map((skill) => ({
      ...skillToCommand(skill, 'runtime-skill'),
      run: () => input.onSelectRuntimeEntry?.({ kind: 'skill', item: skill }),
    }))
    const runtimeCommands = input.runtimeCommands.value.map((command) => ({
      ...skillToCommand(command, 'runtime-command'),
      run: () => input.onSelectRuntimeEntry?.({ kind: 'command', item: command }),
    }))
    return [...navigation, ...local, ...runtimeCommands, ...runtimeSkillCommands]
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
    const context = parsed.slash && normalize(item.slash || '') === parsed.slash ? { input: inputText, args: parsed.args } : undefined

    await item.run(context)
    closePalette()
    return true
  }

  async function runSlashCommand(inputText: string): Promise<{ matched: boolean; command?: CommandItem }> {
    const parsed = parseCommandInput(inputText)
    if (!parsed.slash) return { matched: false }

    const command = items.value.find((item) => normalize(item.slash || '') === parsed.slash)
    if (!command) return { matched: false }
    await command.run({ input: String(inputText || '').trim(), args: parsed.args })
    return { matched: true, command }
  }

  async function runLocalSlashCommand(inputText: string): Promise<{ matched: boolean; command?: CommandItem }> {
    const parsed = parseCommandInput(inputText)
    if (!parsed.slash) return { matched: false }

    const command = items.value.find(
      (item) => isLocalCommandSource(item.source) && normalize(item.slash || '') === parsed.slash,
    )
    if (!command) return { matched: false }
    await command.run({ input: String(inputText || '').trim(), args: parsed.args })
    return { matched: true, command }
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
