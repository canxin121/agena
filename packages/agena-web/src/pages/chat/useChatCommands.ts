import { computed, nextTick, ref, watch, type Ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiCommandLine, RiFlashlightLine } from '@remixicon/vue'

import { apiJson } from '@/lib/api'
import {
  BUILT_IN_COMMANDS,
  findBuiltInCommand,
  normalizeCommandPaletteQuery,
  parseSlashInvocation,
  schemaNeedsPluginInput,
  shouldResetCommandPaletteSelection,
  type BuiltInCommandSpec,
} from './chatCommandsCatalog'
import {
  executePluginSlashCommand,
  type PluginCommandEffect,
  type PluginSlashCommand,
  type PluginUiAction,
} from '@/lib/pluginUiCommands'

export type BuiltInCommand = BuiltInCommandSpec & {
  description: string
}

export type PluginCommand = {
  kind: 'plugin'
  name: string
  description?: string
  scope?: string
  aliases: string[]
  pluginId: string
  commandId: string
  slash: string
  inputSchema?: unknown
  action: PluginUiAction
  requiresArguments: boolean
  arguments: string
}

export type Command = BuiltInCommand | PluginCommand

type ComposerExpose = {
  textareaEl?: HTMLTextAreaElement | { value: HTMLTextAreaElement | null } | null
}

type PluginCatalogCommand = {
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
}

type PluginUiCatalog = {
  catalog?: {
    studio?: {
      commands?: PluginCatalogCommand[]
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

function commandName(command: Pick<Command, 'name'>): string {
  return String(command.name || '')
    .replace(/^\/+/, '')
    .trim()
    .toLowerCase()
}

function commandMatches(command: Command, name: string): boolean {
  const normalized = String(name || '')
    .replace(/^\/+/, '')
    .trim()
    .toLowerCase()
  return (
    commandName(command) === normalized ||
    command.aliases.some((alias) => alias.replace(/^\/+/, '').trim().toLowerCase() === normalized)
  )
}

export function matchSlashCommand(commands: Command[], raw: string): { command: Command; args: string } | null {
  const parsed = parseSlashInvocation(raw)
  if (!parsed) return null
  const command = commands.find((candidate) => commandMatches(candidate, parsed.name))
  return command ? { command, args: parsed.args } : null
}

/** Kept as a named helper for callers/tests that only care about plugins. */
export function matchPluginSlashCommand(
  commands: Command[],
  raw: string,
): { command: PluginCommand; args: string } | null {
  const matched = matchSlashCommand(commands, raw)
  return matched?.command.kind === 'plugin' ? (matched as { command: PluginCommand; args: string }) : null
}

export function commandNeedsArguments(command: Command): boolean {
  return command.requiresArguments
}

export function useChatCommands(opts: {
  draft: Ref<string>
  composerRef: Ref<ComposerExpose | null>
  composerPickerOpen: Ref<null | 'model' | 'thinking' | 'speed'>
  onSend: () => Promise<void>
  onCommandSelected: (command: Command) => void | Promise<void>
}) {
  const { draft, composerRef, composerPickerOpen, onSend, onCommandSelected } = opts
  const { t } = useI18n()
  const commands = ref<Command[]>([])
  const commandsLoading = ref(false)
  const commandQuery = ref('')
  const commandOpen = ref(false)
  const commandIndex = ref(0)
  const commandFocusSearch = ref(true)
  let commandsLoadInFlight: Promise<void> | null = null

  const builtInCommands = computed<BuiltInCommand[]>(() =>
    BUILT_IN_COMMANDS.map((command) => ({
      ...command,
      description: String(t(command.descriptionKey)),
    })),
  )

  function closeCommandPalette() {
    commandOpen.value = false
    commandQuery.value = ''
    commandIndex.value = 0
  }

  function openCommandPalette(query = '', options: { focusSearch?: boolean } = {}) {
    if (composerPickerOpen.value) composerPickerOpen.value = null
    const nextQuery = normalizeCommandPaletteQuery(query)
    if (shouldResetCommandPaletteSelection(commandOpen.value, commandQuery.value, nextQuery)) {
      commandIndex.value = 0
    }
    commandQuery.value = nextQuery
    commandFocusSearch.value = options.focusSearch !== false
    commandOpen.value = true
    if (commands.value.length === 0) void loadCommands()
  }

  function pluginCommandFromCatalog(command: PluginCatalogCommand): PluginCommand | null {
    const pluginId = text(command.plugin_id)
    const commandId = text(command.id)
    const slash = text(command.slash)
    const name = slash.replace(/^\/+/, '').toLowerCase()
    // Match the TUI's slash-name projection: commands without a slash name,
    // or with whitespace inside that name, are not invocable palette entries.
    if (!pluginId || !commandId || !name || /\s/.test(name)) return null
    const declaredAction = command.action || { kind: 'none' }
    const action =
      declaredAction.kind === 'none' && text(command.handler)
        ? ({ kind: 'invoke_command', command: commandId } satisfies PluginUiAction)
        : declaredAction
    return {
      kind: 'plugin',
      name,
      description: text(command.description) || text(command.title),
      aliases: Array.isArray(command.aliases)
        ? command.aliases.map((alias) => text(alias).replace(/^\/+/, '').toLowerCase()).filter(Boolean)
        : [],
      scope: text(command.category) || 'plugin',
      pluginId,
      commandId,
      slash,
      inputSchema: command.input_schema,
      action,
      requiresArguments:
        action.kind === 'open_plugin_workbench' || action.kind === 'open_url' || action.kind === 'submit_prompt'
          ? false
          : schemaNeedsPluginInput(command.input_schema),
      arguments:
        action.kind === 'open_plugin_workbench' || action.kind === 'open_url' || action.kind === 'submit_prompt'
          ? ''
          : schemaNeedsPluginInput(command.input_schema)
            ? '<args>'
            : '',
    }
  }

  async function loadCommandsInternal() {
    commandsLoading.value = true
    try {
      // Built-ins are local and are always available, even if the plugin
      // catalog is temporarily unavailable.
      const builtIns = builtInCommands.value
      const next = new Map<string, Command>(builtIns.map((command) => [command.name, command]))
      try {
        const pluginCatalog = await apiJson<PluginUiCatalog>('/api/v1/plugins/ui')
        for (const rawCommand of pluginCatalog?.catalog?.studio?.commands || []) {
          const command = pluginCommandFromCatalog(rawCommand)
          if (!command) continue
          // TUI gives a built-in command precedence over a plugin primary
          // name. Apply the same rule to aliases so the two clients never
          // show two rows for the same slash invocation.
          if (findBuiltInCommand(command.name)) continue
          if (command.aliases.some((alias) => Boolean(findBuiltInCommand(alias)))) {
            command.aliases = command.aliases.filter((alias) => !findBuiltInCommand(alias))
          }
          if (next.has(command.name)) continue
          next.set(command.name, command)
        }
      } catch {
        // A missing plugin catalog must not make the built-in command palette
        // disappear.
      }
      commands.value = [...next.values()]
    } finally {
      commandsLoading.value = false
    }
  }

  async function loadCommands() {
    if (commandsLoadInFlight) return commandsLoadInFlight
    const request = loadCommandsInternal()
    commandsLoadInFlight = request
    try {
      await request
    } finally {
      if (commandsLoadInFlight === request) commandsLoadInFlight = null
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
        const candidate = `${command.name} ${command.description || ''} ${(command.aliases || []).join(' ')} ${command.arguments || ''}`
        const score = commandScore(query, candidate)
        return score == null ? null : { command, score }
      })
      .filter((item): item is { command: Command; score: number } => Boolean(item))
      .sort((left, right) => right.score - left.score || left.command.name.localeCompare(right.command.name))
      .map((item) => item.command)
  })

  watch([() => filteredCommands.value.length, commandQuery], () => {
    commandIndex.value = Math.max(0, Math.min(commandIndex.value, filteredCommands.value.length - 1))
  })

  function setCommandQuery(value: string) {
    const nextQuery = normalizeCommandPaletteQuery(value)
    if (commandQuery.value === nextQuery) return
    commandQuery.value = nextQuery
    commandIndex.value = 0
  }

  function moveCommandSelection(delta: number) {
    const count = filteredCommands.value.length
    if (!count) return
    commandIndex.value = (commandIndex.value + delta + count) % count
  }

  async function selectCommand(command: Command | null | undefined) {
    if (!command) return
    if (commandNeedsArguments(command)) {
      // TUI displays the required placeholder in the palette, but inserts
      // only the command name into the composer. Copying "<message>" into a
      // real draft would make it very easy to submit the placeholder
      // literally, especially on touch keyboards.
      draft.value = `/${command.name} `
      closeCommandPalette()
      await nextTick()
      const input = getComposerTextareaEl(composerRef.value)
      if (!input) return
      input.focus()
      input.setSelectionRange(input.value.length, input.value.length)
      return
    }
    closeCommandPalette()
    draft.value = ''
    await onCommandSelected(command)
  }

  function handleDraftInput() {
    if (composerPickerOpen.value) composerPickerOpen.value = null
    const trimmed = draft.value.trim()
    // TUI opens the palette for a bare slash. Web additionally keeps it open
    // while the user types the command name, which makes the same workflow
    // practical on touch keyboards. Keep the textarea focused here: the
    // command query is the text after the slash, so unknown slash commands
    // can still fall through to ordinary message sending.
    if (/^\/[^\s/]*$/.test(trimmed)) {
      openCommandPalette(trimmed.slice(1), { focusSearch: false })
      return
    }
    if (commandOpen.value) closeCommandPalette()
  }

  function handleCommandPaletteKeydown(event: KeyboardEvent) {
    if (!commandOpen.value) return
    if (event.key === 'Escape') {
      event.preventDefault()
      closeCommandPalette()
      return
    }
    if (event.key === 'ArrowDown') {
      event.preventDefault()
      moveCommandSelection(1)
      return
    }
    if (event.key === 'ArrowUp') {
      event.preventDefault()
      moveCommandSelection(-1)
      return
    }
    if (event.key === 'Enter' && !event.ctrlKey && !event.metaKey && !event.shiftKey) {
      event.preventDefault()
      const command = filteredCommands.value[commandIndex.value] || null
      if (command) {
        void selectCommand(command)
        return
      }

      // TUI treats an unknown slash invocation as ordinary composer text.
      // This branch is also used when the user searches for a plugin command
      // that is not present in the current catalog.
      const typed = draft.value.trim()
      const fallback = typed.startsWith('/') && !typed.startsWith('//') ? typed : `/${commandQuery.value.trim()}`
      if (/^\/[^\s/]+(?:\s|$)/.test(fallback)) {
        closeCommandPalette()
        draft.value = fallback
        void onSend()
      } else {
        closeCommandPalette()
      }
    }
  }

  function handleDraftKeydown(event: KeyboardEvent) {
    if (commandOpen.value) {
      handleCommandPaletteKeydown(event)
      if (event.defaultPrevented) return
    }
    if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) {
      event.preventDefault()
      void onSend()
    }
  }

  async function runPluginSlashCommand(raw: string, sessionId: string): Promise<PluginCommandEffect | null> {
    if (commands.value.length === 0) await loadCommands()
    const matched = matchPluginSlashCommand(commands.value, raw)
    if (!matched) return null
    const numericSessionId = Number(sessionId)
    if (!Number.isSafeInteger(numericSessionId) || numericSessionId <= 0) {
      throw new Error('Open a session before running plugin commands.')
    }
    const catalog: PluginSlashCommand[] = commands.value
      .filter((command): command is PluginCommand => command.kind === 'plugin')
      .map((command) => ({
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
    return command.kind === 'builtin' ? RiFlashlightLine : RiCommandLine
  }

  return {
    commands,
    commandsLoading,
    commandQuery,
    commandOpen,
    commandIndex,
    setCommandQuery,
    commandFocusSearch,
    filteredCommands,
    loadCommands,
    openCommandPalette,
    closeCommandPalette,
    handleDraftInput,
    handleDraftKeydown,
    handleCommandPaletteKeydown,
    moveCommandSelection,
    selectCommand,
    runPluginSlashCommand,
    commandIcon,
  }
}
