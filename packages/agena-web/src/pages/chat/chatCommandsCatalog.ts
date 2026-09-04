/**
 * The web command catalog follows crates/agena-tui-app/src/commands.rs where
 * the command is supported by the web client. The TUI-only session hub and
 * lineage commands are intentionally not exposed here.
 *
 * Keep this file free of Vue and i18n state so command parsing and the
 * catalog can be tested without mounting a page.  Descriptions are resolved
 * by useChatCommands through the `descriptionKey` field.
 */

export type BuiltInCommandId =
  | 'help'
  | 'commands'
  | 'new'
  | 'sessions'
  | 'rewind'
  | 'rename'
  | 'timeline'
  | 'settings'
  | 'model'
  | 'review'
  | 'commit'
  | 'pr'
  | 'export'
  | 'pager'
  | 'continue'
  | 'compact'
  | 'user-input'
  | 'allow'
  | 'allow-always'
  | 'deny'
  | 'deny-always'
  | 'attach'
  | 'skill'
  | 'skill-manager'
  | 'download'
  | 'editor'
  | 'image'
  | 'copy'
  | 'copy-message'
  | 'copy-visible'
  | 'fork'
  | 'children'
  | 'parent'
  | 'diagnostics'
  | 'status'
  | 'usage'
  | 'activities'
  | 'background'
  | 'plan'
  | 'side'

export type BuiltInCommandSpec = {
  kind: 'builtin'
  id: BuiltInCommandId
  name: BuiltInCommandId
  aliases: string[]
  arguments: string
  requiresArguments: boolean
  descriptionKey: string
  /** Some commands have an optional argument but open a UI by default. */
  opensInteractiveSurface: boolean
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

type CommandSeed = [BuiltInCommandId, string[], string, boolean?]

// Keep the order identical to the TUI catalog.  The palette can sort the
// result for searching, but a blank palette should still feel familiar in
// both clients.
const COMMAND_SEEDS: CommandSeed[] = [
  ['help', ['?'], ''],
  ['commands', ['palette'], ''],
  ['new', ['clear'], ''],
  ['sessions', [], ''],
  ['rewind', ['backtrack'], ''],
  ['rename', ['title'], ''],
  ['timeline', ['events'], ''],
  ['settings', ['config'], ''],
  ['model', [], ''],
  ['review', [], '[focus]'],
  ['commit', [], '<message>'],
  ['pr', [], '<title> [--body <text>] [--base <branch>] [--head <branch>]'],
  ['export', ['save'], '[path]'],
  ['pager', ['view', 'less'], ''],
  ['continue', ['resume-run'], ''],
  ['compact', ['compress', 'summarize'], ''],
  ['user-input', ['reply'], ''],
  ['allow', [], ''],
  ['allow-always', [], ''],
  ['deny', [], ''],
  ['deny-always', [], ''],
  ['attach', ['file'], ''],
  ['skill', ['skills'], ''],
  ['skill-manager', ['manage-skills'], ''],
  ['download', ['dl'], '<workspace-path>'],
  ['editor', ['edit'], ''],
  ['image', [], ''],
  ['copy', ['yank'], ''],
  ['copy-message', ['copy-last', 'copy-assistant'], ''],
  ['copy-visible', [], ''],
  ['fork', ['branch'], ''],
  ['children', ['child'], ''],
  ['parent', [], ''],
  ['diagnostics', ['feedback'], ''],
  ['status', [], ''],
  ['usage', ['stats', 'analytics'], ''],
  ['activities', ['tasks'], ''],
  ['background', [], ''],
  ['plan', ['plan-view', 'show-plan'], ''],
  ['side', ['btw', 'aside'], ''],
]

export const BUILT_IN_COMMANDS: BuiltInCommandSpec[] = COMMAND_SEEDS.map(([name, aliases, argumentsLabel]) => ({
  kind: 'builtin',
  id: name,
  name,
  aliases,
  arguments: argumentsLabel,
  requiresArguments: argumentsLabel.trimStart().startsWith('<'),
  descriptionKey: `chat.commandPalette.builtIns.${name}`,
  opensInteractiveSurface: [
    'sessions',
    'background',
    'rename',
    'timeline',
    'settings',
    'attach',
    'skill',
    'skill-manager',
    'image',
    'usage',
    'activities',
    'plan',
    'fork',
    'side',
  ].includes(name),
}))

const BUILT_IN_BY_NAME = new Map<string, BuiltInCommandSpec>()
for (const command of BUILT_IN_COMMANDS) {
  BUILT_IN_BY_NAME.set(command.name, command)
  for (const alias of command.aliases) BUILT_IN_BY_NAME.set(alias, command)
}

export function findBuiltInCommand(name: string): BuiltInCommandSpec | null {
  return (
    BUILT_IN_BY_NAME.get(
      String(name || '')
        .replace(/^\/+/, '')
        .trim()
        .toLowerCase(),
    ) || null
  )
}

export function paletteInvocation(
  command: Pick<BuiltInCommandSpec, 'arguments' | 'requiresArguments'> & { name: string },
): string {
  if (!command.requiresArguments) return `/${command.name}`
  const required = command.arguments.split(' [', 1)[0] || command.arguments
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

export function schemaRequiresArguments(schema: unknown): boolean {
  if (!schema || typeof schema !== 'object' || Array.isArray(schema)) return false
  const value = schema as { required?: unknown }
  return Array.isArray(value.required) && value.required.some((item) => typeof item === 'string' && item.trim())
}

/**
 * Matches the TUI's `plugin_command_accepts_empty_arguments` decision for
 * command-backed/plugin-tool-backed entries. A JSON schema with a scalar
 * input (or an object with required fields) cannot be invoked from a palette
 * selection without first giving the user a composer draft.
 */
export function schemaNeedsPluginInput(schema: unknown): boolean {
  // This is intentionally the same projection as the TUI's
  // `plugin_command_accepts_empty_arguments`. Static actions are handled by
  // the caller; this helper answers whether an empty slash invocation must
  // first collect arguments from the composer.
  if (schema == null) return false
  if (typeof schema !== 'object' || Array.isArray(schema)) {
    // JSON-schema booleans are the only non-object schemas the TUI accepts as
    // metadata. `true` accepts an empty input; `false` does not.
    return schema !== true
  }
  const value = schema as { type?: unknown; required?: unknown; minProperties?: unknown }
  const type = value.type
  const acceptsObject =
    type === undefined
      ? Object.keys(value).length === 0
      : type === 'object' || (Array.isArray(type) && type.some((kind) => kind === 'object'))
  if (!acceptsObject) return true
  if (schemaRequiresArguments(schema)) return true
  return typeof value.minProperties === 'number' && Number.isFinite(value.minProperties) && value.minProperties > 0
}
