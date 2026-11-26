import { computed, nextTick, ref, watch, type Ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiCommandLine, RiFlashlightLine } from '@remixicon/vue'

import { readPluginCommandCatalog } from '../../lib/pluginCommandRead'
import { isPlainNavigationKey } from '@/lib/keyboardNavigation'
import { getComposerInput, type ComposerExpose } from './composerInput'
import type { ComposerPickerKind } from './composerPickerTypes'
import type { ExternalAgentCommand } from '@/types/chat'
import {
  clientCommandsFromCatalog,
  commandAppliesToSession,
  normalizeCommandPaletteQuery,
  parseSlashInvocation,
  shouldResetCommandPaletteSelection,
  type ClientCommand,
  type ClientCommandAction,
} from './chatCommandsCatalog'
import {
  executePluginSlashCommand,
  type PluginCommandCatalogItem,
  type PluginCommandResult,
} from '@/lib/pluginOperations'

/** A locally-run command, with the published description resolved for display. */
export type BuiltInCommand = ClientCommand & {
  description: string
  /** The declaration's usage line, under the name the palette renders. */
  arguments: string
}

/**
 * A command the server owns. Its published declaration travels with it, so the
 * palette shows the same slash, usage and summary the catalog declared.
 */
export type PluginCommand = {
  kind: 'plugin'
  name: string
  description?: string
  scope?: string
  aliases: string[]
  pluginId: string
  commandId: string
  slash: string
  acceptsEmptyInput: boolean
  command: PluginCommandCatalogItem
  requiresArguments: boolean
  arguments: string
}

type LocalCommand = BuiltInCommand | PluginCommand

type NativeCommand = {
  kind: 'external'
  sessionId: string
  name: string
  description: string
  scope: string
  aliases: string[]
  arguments: string
  requiresArguments: boolean
}

export type Command = LocalCommand | NativeCommand

const externalCommandPrefix = 'agent:'
const agenaSessionActions = new Set<ClientCommandAction>([
  'rewind', 'rename', 'integrations', 'assistant', 'model', 'compact', 'clear',
  'favorite', 'hub', 'lineage', 'fork', 'side', 'btw', 'children', 'parent', 'schedules',
])

function isQualifiedAgentName(name: string): boolean {
  const bare = name.slice(externalCommandPrefix.length)
  return name.toLowerCase().startsWith(externalCommandPrefix) && !!bare && !bare.startsWith('/') && !/\p{Cc}/u.test(bare)
}

export function isExternalAgentCommandInvocation(text: string): boolean {
  const invocation = parseSlashInvocation(text)
  return !!invocation && isQualifiedAgentName(invocation.name)
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

/** The argument label a command contributes to palette search. */
function commandArguments(command: Command): string {
  return command.arguments
}

export function useChatCommands(opts: {
  draft: Ref<string>
  composerRef: Ref<ComposerExpose | null>
  composerPickerOpen: Ref<null | ComposerPickerKind>
  /** Whether the rendered conversation is the operator Bot. */
  isBotSession: Ref<boolean>
  onSend: () => Promise<void>
  externalAgent?: Ref<{ sessionId: string; label: string; commands: readonly ExternalAgentCommand[] } | null>
  onCommandSelected: (command: LocalCommand) => void | Promise<void>
}) {
  const { draft, composerRef, composerPickerOpen, isBotSession, externalAgent, onSend, onCommandSelected } = opts
  const { t } = useI18n()
  const localCommands = ref<LocalCommand[]>([])
  const commands = computed<Command[]>(() => {
    const agent = externalAgent?.value
    if (!agent) return localCommands.value
    return [
      ...localCommands.value
        .filter((command) => command.kind !== 'builtin' || !agenaSessionActions.has(command.action))
        .map((command) => ({
          ...command,
          aliases: command.aliases.filter((alias) => !isQualifiedAgentName(alias)),
        })),
      ...agent.commands.map((command): NativeCommand => ({
        kind: 'external',
        sessionId: agent.sessionId,
        name: `${externalCommandPrefix}${command.name}`,
        description: command.description,
        scope: command.skill ? `${agent.label} · ${t('chat.skills.title')}` : agent.label,
        aliases: [],
        arguments: command.argument_hint ?? '',
        requiresArguments: command.skill === true || command.argument_hint != null,
      })),
    ]
  })
  const commandsLoading = ref(false)
  const commandQuery = ref('')
  const commandOpen = ref(false)
  const commandIndex = ref(0)
  const commandFocusSearch = ref(true)
  let commandsLoadInFlight: Promise<void> | null = null

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
    void loadCommands()
  }

  /**
   * The description a built-in row shows. Text lives in the client's message
   * catalog, so the published declaration carries a key; the literal summary is
   * the fallback and the declaration's id is the last resort, so a row is never
   * blank.
   */
  function builtInDescription(command: ClientCommand): string {
    const key = text(command.docs?.summary_key)
    if (key) {
      const translated = String(t(key))
      // vue-i18n echoes the key back when no locale carries it.
      if (translated && translated !== key) return translated
    }
    return text(command.docs?.summary) || command.id
  }

  function builtInFromCatalog(command: ClientCommand): BuiltInCommand | null {
    return { ...command, arguments: command.usage, description: builtInDescription(command) }
  }

  /**
   * A command the server runs. The catalog is the only declaration, so nothing
   * is invented locally: no slash, no alias and no usage line exist here that
   * the server did not publish.
   */
  function pluginCommandFromCatalog(command: PluginCommandCatalogItem): PluginCommand | null {
    const pluginId = text(command.plugin_id)
    const commandId = text(command.id)
    const slash = text(command.slash)
    const name = slash.replace(/^\/+/, '').toLowerCase()
    if (!pluginId || !commandId || !name || /\s/.test(name)) {
      return null
    }
    const requiresArguments = command.accepts_empty_input !== true
    return {
      kind: 'plugin',
      name,
      description: text(command.docs?.summary) || text(command.title),
      aliases: Array.isArray(command.aliases)
        ? command.aliases.map((alias) => text(alias).replace(/^\/+/, '').toLowerCase()).filter(Boolean)
        : [],
      scope: text(command.category) || text(command.group) || 'plugin',
      pluginId,
      commandId,
      slash,
      acceptsEmptyInput: !requiresArguments,
      command,
      requiresArguments,
      arguments: text(command.docs?.usage) || (requiresArguments ? '<args>' : ''),
    }
  }

  async function loadCommandsInternal() {
    commandsLoading.value = true
    try {
      let catalogCommands: PluginCommandCatalogItem[] = []
      try {
        const pluginCatalog = await readPluginCommandCatalog()
        catalogCommands = pluginCatalog?.catalog?.commands || []
      } catch {
        // An unreachable plugin catalog means there is nothing to offer yet;
        // the next load picks it up.
      }

      const next = new Map<string, LocalCommand>()
      for (const command of clientCommandsFromCatalog(catalogCommands, isBotSession.value)) {
        const builtIn = builtInFromCatalog(command)
        if (builtIn && !next.has(builtIn.name)) next.set(builtIn.name, builtIn)
      }
      for (const rawCommand of catalogCommands) {
        // A declaration that does not name this conversation kind has no row,
        // exactly as the server refuses to run it here.
        if (!commandAppliesToSession(rawCommand, isBotSession.value)) continue
        const command = pluginCommandFromCatalog(rawCommand)
        if (!command) continue
        if (next.has(command.name)) continue
        // A slash already taken by a locally-run command must not be offered
        // twice; drop the alias rather than the row.
        command.aliases = command.aliases.filter((alias) => !next.has(alias))
        next.set(command.name, command)
      }
      localCommands.value = [...next.values()]
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

  async function ensureLocalCommands() {
    if (localCommands.value.length === 0) await loadCommands()
  }

  // Switching between a project conversation and the Bot changes which
  // declarations belong in the palette. Keep the rows honest without asking
  // the server again: the catalog itself is session-agnostic, applicability is
  // part of each declaration.
  watch(
    () => isBotSession.value,
    () => {
      if (localCommands.value.length === 0) return
      void loadCommands()
    },
  )

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
    const onlyExternal = externalAgent?.value && query.startsWith(externalCommandPrefix)
    return commands.value
      .map((command) => {
        if (onlyExternal && command.kind !== 'external') return null
        const candidate = `${command.name} ${command.description || ''} ${(command.aliases || []).join(' ')} ${commandArguments(command)}`
        const score = commandScore(query, candidate)
        return score == null ? null : { command, score, exact: commandMatches(command, query) }
      })
      .filter((item) => item !== null)
      .sort((left, right) => Number(right.exact) - Number(left.exact) || right.score - left.score || left.command.name.localeCompare(right.command.name))
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
    if (command.kind === 'external' && !commands.value.includes(command)) return
    if (commandNeedsArguments(command)) {
      // TUI displays the required placeholder in the palette, but inserts
      // only the command name into the composer. Copying "<message>" into a
      // real draft would make it very easy to submit the placeholder
      // literally, especially on touch keyboards.
      draft.value = `/${command.name} `
      closeCommandPalette()
      await nextTick()
      const input = getComposerInput(composerRef.value)
      if (!input) return
      input.focus()
      input.setSelectionRange(input.value.length, input.value.length)
      return
    }
    closeCommandPalette()
    if (command.kind === 'external') {
      const invocation = `/${command.name}`
      draft.value = invocation
      await nextTick()
      if (command.sessionId === externalAgent?.value?.sessionId && draft.value === invocation) await onSend()
      return
    }
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
    const isQualifiedName = externalAgent?.value && !/\s/.test(trimmed) && isExternalAgentCommandInvocation(trimmed)
    if (/^\/[^\s/]*$/.test(trimmed) || isQualifiedName) {
      openCommandPalette(trimmed.slice(1), { focusSearch: false })
      return
    }
    if (commandOpen.value) closeCommandPalette()
  }

  function handleCommandPaletteKeydown(event: KeyboardEvent) {
    if (!commandOpen.value) return
    // The palette is opened from the composer while the user is typing. An IME
    // confirm must never pick the highlighted command, close the palette, or
    // move the selection: those keys belong to the candidate window.
    if (!isPlainNavigationKey(event) || event.shiftKey) return
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
    if (event.key === 'Enter') {
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
      const isQualified = externalAgent?.value && isExternalAgentCommandInvocation(fallback)
      if (/^\/[^\s/]+(?:\s|$)/.test(fallback) || isQualified) {
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

  async function runPluginSlashCommand(raw: string, sessionId: string): Promise<PluginCommandResult | null> {
    await ensureLocalCommands()
    const matched = matchPluginSlashCommand(commands.value, raw)
    if (!matched) return null
    const numericSessionId = Number(sessionId)
    return await executePluginSlashCommand({
      command: matched.command.command,
      sessionId: Number.isSafeInteger(numericSessionId) && numericSessionId > 0 ? numericSessionId : null,
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
    ensureLocalCommands,
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
