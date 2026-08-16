import { computed, nextTick, ref, watch, type Ref } from 'vue'
import { RiCommandLine, RiFlashlightLine } from '@remixicon/vue'

import { apiJson } from '@/lib/api'
import {
  executePluginSlashCommand,
  type PluginCommandEffect,
  type PluginSlashCommand,
  type PluginUiAction,
} from '@/lib/pluginUiCommands'

export type Command = {
  name: string
  description?: string
  scope?: string
  isBuiltIn?: boolean
  aliases?: string[]
  pluginId: string
  commandId: string
  slash: string
  inputSchema?: unknown
  action: PluginUiAction
}

type ComposerExpose = {
  textareaEl?: HTMLTextAreaElement | { value: HTMLTextAreaElement | null } | null
}

type PluginUiCatalog = {
  catalog?: {
    studio?: {
      commands?: Array<{
        plugin_id?: string
        id?: string
        title?: string
        description?: string
        category?: string
        slash?: string | null
        aliases?: string[]
        input_schema?: unknown
        handler?: string | null
        action?: PluginUiAction
      }>
    }
  }
}

function getComposerTextareaEl(composer: ComposerExpose | null): HTMLTextAreaElement | null {
  const textarea = composer?.textareaEl
  if (!textarea) return null
  return textarea instanceof HTMLTextAreaElement ? textarea : textarea.value
}

function text(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

export function matchPluginSlashCommand(commands: Command[], raw: string): { command: Command; args: string } | null {
  const input = String(raw || '').trim()
  if (!input.startsWith('/')) return null
  const separator = input.search(/\s/)
  const name = input
    .slice(1, separator < 0 ? undefined : separator)
    .trim()
    .toLowerCase()
  if (!name) return null
  const command = commands.find((candidate) => {
    if (candidate.name.toLowerCase() === name) return true
    return (candidate.aliases || []).some((alias) => alias.replace(/^\/+/, '').toLowerCase() === name)
  })
  if (!command) return null
  return { command, args: separator < 0 ? '' : input.slice(separator + 1).trim() }
}

export function useChatCommands(opts: {
  draft: Ref<string>
  composerRef: Ref<ComposerExpose | null>
  composerPickerOpen: Ref<null | 'model' | 'thinking' | 'speed'>
  onSend: () => Promise<void>
}) {
  const { draft, composerRef, composerPickerOpen, onSend } = opts
  const commands = ref<Command[]>([])
  const commandsLoading = ref(false)
  const commandQuery = ref('')
  const commandOpen = ref(false)
  const commandIndex = ref(0)

  function closeCommandPalette() {
    commandOpen.value = false
    commandQuery.value = ''
    commandIndex.value = 0
  }

  async function loadCommands() {
    commandsLoading.value = true
    try {
      const pluginCatalog = await apiJson<PluginUiCatalog>('/api/v1/plugins/ui')
      const next = new Map<string, Command>()
      for (const command of pluginCatalog?.catalog?.studio?.commands || []) {
        const pluginId = text(command.plugin_id)
        const commandId = text(command.id)
        const slash = text(command.slash)
        const name = slash.replace(/^\/+/, '')
        if (!pluginId || !commandId || !name || next.has(name)) continue
        const declaredAction = command.action || { kind: 'none' }
        const action =
          declaredAction.kind === 'none' && text(command.handler)
            ? ({ kind: 'invoke_command', command: commandId } satisfies PluginUiAction)
            : declaredAction
        next.set(name, {
          name,
          description: text(command.description) || text(command.title),
          aliases: Array.isArray(command.aliases) ? command.aliases.map(text).filter(Boolean) : [],
          scope: text(command.category) || 'plugin',
          pluginId,
          commandId,
          slash,
          inputSchema: command.input_schema,
          action,
        })
      }
      commands.value = [...next.values()].sort((left, right) => left.name.localeCompare(right.name))
    } finally {
      commandsLoading.value = false
    }
  }

  function commandScore(query: string, candidate: string): number | null {
    const normalizedQuery = query.trim().toLowerCase()
    if (!normalizedQuery) return 0
    const normalizedCandidate = candidate.toLowerCase()
    const index = normalizedCandidate.indexOf(normalizedQuery)
    if (index >= 0) return 100 - index
    let cursor = -1
    let score = 0
    for (const character of normalizedQuery) {
      cursor = normalizedCandidate.indexOf(character, cursor + 1)
      if (cursor < 0) return null
      score += Math.max(1, 20 - cursor)
    }
    return score
  }

  const filteredCommands = computed(() => {
    const query = commandQuery.value.trim().toLowerCase()
    if (!query) return commands.value
    return commands.value
      .map((command) => {
        const candidate = `${command.name} ${command.description || ''} ${(command.aliases || []).join(' ')}`
        const score = commandScore(query, candidate)
        return score == null ? null : { command, score }
      })
      .filter((item): item is { command: Command; score: number } => Boolean(item))
      .sort((left, right) => right.score - left.score || left.command.name.localeCompare(right.command.name))
      .map((item) => item.command)
  })

  watch([() => filteredCommands.value.length, commandQuery], () => {
    commandIndex.value = 0
  })

  function handleDraftInput() {
    if (!getComposerTextareaEl(composerRef.value)) return
    if (composerPickerOpen.value) composerPickerOpen.value = null
    closeCommandPalette()
  }

  function handleDraftKeydown(event: KeyboardEvent) {
    if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) {
      event.preventDefault()
      void onSend()
    }
  }

  function insertCommand(command: Command) {
    draft.value = `/${command.name} `
    closeCommandPalette()
    void nextTick(() => {
      const input = getComposerTextareaEl(composerRef.value)
      if (!input) return
      input.focus()
      input.setSelectionRange(input.value.length, input.value.length)
    })
  }

  async function runPluginSlashCommand(raw: string, sessionId: string): Promise<PluginCommandEffect | null> {
    if (commands.value.length === 0) await loadCommands()
    const matched = matchPluginSlashCommand(commands.value, raw)
    if (!matched) return null
    const numericSessionId = Number(sessionId)
    if (!Number.isSafeInteger(numericSessionId) || numericSessionId <= 0) {
      throw new Error('Open a session before running plugin commands.')
    }
    const catalog: PluginSlashCommand[] = commands.value.map((command) => ({
      pluginId: command.pluginId,
      id: command.commandId,
      slash: command.slash,
      inputSchema: command.inputSchema,
      action: command.action,
    }))
    return await executePluginSlashCommand({
      command: {
        pluginId: matched.command.pluginId,
        id: matched.command.commandId,
        slash: matched.command.slash,
        inputSchema: matched.command.inputSchema,
        action: matched.command.action,
      },
      catalog,
      sessionId: numericSessionId,
      rawArgs: matched.args,
    })
  }

  function commandIcon(command: Command) {
    return command.isBuiltIn ? RiFlashlightLine : RiCommandLine
  }

  return {
    commands,
    commandsLoading,
    commandQuery,
    commandOpen,
    commandIndex,
    filteredCommands,
    loadCommands,
    handleDraftInput,
    handleDraftKeydown,
    insertCommand,
    runPluginSlashCommand,
    commandIcon,
  }
}
