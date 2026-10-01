/**
 * The web command catalog.
 *
 * Commands are declared once, by the server: the `agena.commands` plugin
 * declares the built-ins, and any plugin may declare its own through the
 * manifest. The host publishes them through `/api/v1/plugins/surface`, already
 * sorted, so this module's job is only to read that list — recognizing a
 * `Client` target as a command the browser runs itself and everything else as
 * a server-owned command.
 *
 * Nothing here is a second declaration: names, aliases, usage lines and
 * descriptions all come from the catalog. `CLIENT_COMMAND_ACTIONS` is only the
 * vocabulary of local actions this client knows how to run, mirroring
 * `agena_api::client_command::ClientCommandAction`; a `Client` command naming
 * an action absent here belongs to another client and is dropped.
 *
 * Keep this file free of Vue and i18n state so command parsing and the
 * catalog can be tested without mounting a page.
 */

import type { PluginCommandCatalogItem, PluginCommandDocs } from '@/lib/pluginOperations'

/** The local actions this client implements, mirroring `ClientCommandAction`. */
export const CLIENT_COMMAND_ACTIONS = [
  'help',
  'commands',
  'new',
  'sessions',
  'hub',
  'lineage',
  'rewind',
  'rename',
  'favorite',
  'timeline',
  'settings',
  'model',
  'commit',
  'pr',
  'export',
  'pager',
  'continue',
  'compact',
  'user-input',
  'allow',
  'allow-always',
  'deny',
  'deny-always',
  'attach',
  'download',
  'editor',
  'image',
  'paste',
  'copy',
  'copy-message',
  'copy-visible',
  'fork',
  'children',
  'parent',
  'diagnostics',
  'status',
  'usage',
  'activities',
  'background',
  'plan',
  'side',
] as const

export type ClientCommandAction = (typeof CLIENT_COMMAND_ACTIONS)[number]

export type ClientCommand = {
  kind: 'builtin'
  /** The declaration's stable identifier. */
  id: string
  /** The `/name` spelling a user types. */
  name: string
  /** The local action to dispatch on. */
  action: ClientCommandAction
  aliases: string[]
  /** The declaration's argument usage line; empty when it takes none. */
  usage: string
  /** A required argument (`<name>`) gates execution; an optional one does not. */
  requiresArguments: boolean
  /** Whether the command can run with no arguments at all. */
  acceptsEmptyInput: boolean
  docs: PluginCommandDocs
  showInPalette: boolean
  matchesSlash: boolean
  command: PluginCommandCatalogItem
}

export function normalizeCommandPaletteQuery(value: unknown): string {
  return String(value || '')
    .replace(/^\/+/, '')
    .trim()
}

/**
 * A composer keyup can reopen the palette while it is already open (the
 * palette is intentionally not auto-focused when the user types `/`). That
 * re-entry must preserve the keyboard selection when the query did not
 * change; otherwise ArrowDown is immediately followed by a reset to row 0.
 */
export function shouldResetCommandPaletteSelection(open: boolean, currentQuery: string, nextQuery: string): boolean {
  return !open || currentQuery !== nextQuery
}

function text(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

function slashName(slash: unknown): string {
  return text(slash).replace(/^\/+/, '')
}

/**
 * Read one catalog entry as a command this client runs locally, or `null` when
 * it is not one: another client's `Client` command, a server-owned target, or
 * a declaration hidden from every catalog.
 */
export function clientCommandFromCatalog(command: PluginCommandCatalogItem): ClientCommand | null {
  if (!command || command.target?.kind !== 'client') return null
  if (command.discoverability?.catalog === false) return null
  const action = text(command.target.action)
  if (!(CLIENT_COMMAND_ACTIONS as readonly string[]).includes(action)) return null
  const name = slashName(command.slash)
  if (!name || /\s/.test(name)) return null

  const usage = text(command.docs?.usage)
  return {
    kind: 'builtin',
    id: text(command.id) || action,
    name: name.toLowerCase(),
    action: action as ClientCommandAction,
    aliases: Array.isArray(command.aliases)
      ? command.aliases.map((alias) => slashName(alias).toLowerCase()).filter(Boolean)
      : [],
    usage,
    requiresArguments: usage.trimStart().startsWith('<'),
    acceptsEmptyInput: command.accepts_empty_input === true,
    docs: command.docs || {},
    showInPalette: command.discoverability?.palette !== false,
    matchesSlash: command.discoverability?.slash !== false,
    command,
  }
}

/** Every locally-run command the catalog declares, in catalog order. */
export function clientCommandsFromCatalog(commands: PluginCommandCatalogItem[] | undefined | null): ClientCommand[] {
  const collected: ClientCommand[] = []
  for (const command of commands || []) {
    const builtin = clientCommandFromCatalog(command)
    if (builtin) collected.push(builtin)
  }
  return collected
}

/** `/name <usage>` as a user would type it, for usage warnings. */
export function commandUsage(command: Pick<ClientCommand, 'name' | 'usage'>): string {
  return `/${command.name}${command.usage ? ` ${command.usage}` : ''}`
}

/** Compact label for the action-oriented command palette. */
export function paletteInvocation(command: Pick<ClientCommand, 'name' | 'usage' | 'requiresArguments'>): string {
  if (!command.requiresArguments) return `/${command.name}`
  const required = command.usage.split(' [', 1)[0] || command.usage
  return `/${command.name} ${required}`
}

export function parseSlashInvocation(raw: string): { name: string; args: string } | null {
  const input = String(raw || '').trim()
  if (!input.startsWith('/') || input.startsWith('//')) return null
  const content = input.slice(1).trimStart()
  if (!content) return null
  const separator = content.search(/\s/)
  const name = (separator < 0 ? content : content.slice(0, separator)).trim().toLowerCase()
  if (!name) return null
  return { name, args: separator < 0 ? '' : content.slice(separator + 1).trim() }
}
