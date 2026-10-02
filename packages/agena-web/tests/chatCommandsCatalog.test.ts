import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

import {
  CLIENT_COMMAND_ACTIONS,
  clientCommandFromCatalog,
  clientCommandsFromCatalog,
  commandUsage,
  normalizeCommandPaletteQuery,
  paletteInvocation,
  parseSlashInvocation,
  shouldResetCommandPaletteSelection,
  type ClientCommand,
} from '../src/pages/chat/chatCommandsCatalog'
import type { PluginCommandCatalogItem } from '../src/lib/pluginOperations'

const REPO_ROOT = resolve(import.meta.dir, '../../..')

/** The `action => "/slash"` rows of a `client_command_actions!` invocation. */
function declaredActionRows(source: string): Array<{ action: string; slash: string }> {
  const rows: Array<{ action: string; slash: string }> = []
  for (const line of source.split('\n')) {
    const match = /^\s*[A-Za-z][A-Za-z0-9]*\s*=>\s*"([^"]+)",\s*"(\/[^"]+)";/.exec(line)
    if (match) rows.push({ action: match[1]!, slash: match[2]! })
  }
  return rows
}

describe('web command vocabulary parity with the server declarations', () => {
  test('the client vocabulary mirrors the Rust ClientCommandAction rows', () => {
    const rust = readFileSync(
      resolve(REPO_ROOT, 'crates/agena-api/src/client_command.rs'),
      'utf8',
    )
    const rows = declaredActionRows(rust)
    expect(rows.length).toBeGreaterThan(0)
    expect(CLIENT_COMMAND_ACTIONS).toEqual(rows.map((row) => row.action))
  })

  test('the built-in declaration publishes the same actions under the same slashes', () => {
    const rust = readFileSync(
      resolve(REPO_ROOT, 'crates/agena-bundled-plugins/src/plugins/provided/commands.rs'),
      'utf8',
    )
    const rows: Array<{ action: string; slash: string }> = []
    const rowPattern = /action:\s*"([^"]+)",\s*\n\s*slash:\s*"(\/[^"]+)",/g
    for (let match = rowPattern.exec(rust); match; match = rowPattern.exec(rust)) {
      rows.push({ action: match[1]!, slash: match[2]! })
    }
    expect(rows.length).toBeGreaterThan(0)
    expect(rows.map((row) => row.action)).toEqual([...CLIENT_COMMAND_ACTIONS])
    expect(new Set(rows.map((row) => row.slash)).size).toBe(rows.length)
  })
})

function catalogItem(
  overrides: Partial<PluginCommandCatalogItem> & { id: string; target: PluginCommandCatalogItem['target'] },
): PluginCommandCatalogItem {
  return {
    plugin_id: 'agena.commands',
    accepts_empty_input: true,
    default_input: {},
    title: overrides.id,
    group: 'Built-in',
    category: 'Client',
    slash: `/${overrides.id}`,
    aliases: [],
    docs: {},
    input: { version: 1, root: {} as PluginCommandCatalogItem['input']['root'] },
    ...overrides,
  }
}

function clientItem(
  id: string,
  options: { action?: string; slash?: string; aliases?: string[]; usage?: string } = {},
): PluginCommandCatalogItem {
  return catalogItem({
    id,
    target: { kind: 'client', action: options.action || id },
    slash: options.slash || `/${id}`,
    aliases: options.aliases || [],
    docs: options.usage ? { usage: options.usage } : {},
  })
}

function builtin(id: string, options: Parameters<typeof clientItem>[1] = {}): ClientCommand {
  const command = clientCommandFromCatalog(clientItem(id, options))
  if (!command) throw new Error(`${id} is not a command this client can run`)
  return command
}

describe('web command catalog', () => {
  test('the client action vocabulary covers every built-in the catalog declares', () => {
    expect(CLIENT_COMMAND_ACTIONS).toContain('help')
    expect(CLIENT_COMMAND_ACTIONS).toContain('side')
    expect(new Set(CLIENT_COMMAND_ACTIONS).size).toBe(CLIENT_COMMAND_ACTIONS.length)
  })

  test('only client-targeted actions this client spells are read, and nothing else is', () => {
    const commands = clientCommandsFromCatalog([
      clientItem('help', { aliases: ['?'] }),
      // Declared for another client: this build has no such action.
      clientItem('web-only', { action: 'an-action-only-another-client-has' }),
      // Server-owned targets are not the client's to run.
      catalogItem({ id: 'remote', target: { kind: 'method', handler: 'remote.run' } }),
      catalogItem({ id: 'tool-backed', target: { kind: 'tool', tool: 'remote.tool' } }),
    ])

    expect(commands.map((command) => command.id)).toEqual(['help'])
    expect(commands[0]!.aliases).toEqual(['?'])
  })

  test('usage drives argument requirements and palette labels', () => {
    expect(builtin('commit', { usage: '<message>' }).requiresArguments).toBe(true)
    expect(paletteInvocation(builtin('commit', { usage: '<message>' }))).toBe('/commit <message>')
    expect(paletteInvocation(builtin('pr', { usage: '<title> [--body <text>]' }))).toBe('/pr <title>')
    // An optional argument still runs without one.
    expect(builtin('export', { usage: '[path]' }).requiresArguments).toBe(false)
    expect(paletteInvocation(builtin('export', { usage: '[path]' }))).toBe('/export')
    expect(paletteInvocation(builtin('sessions'))).toBe('/sessions')
    expect(commandUsage(builtin('download', { usage: '<workspace-path>' }))).toBe('/download <workspace-path>')
    expect(commandUsage(builtin('sessions'))).toBe('/sessions')
  })

  test('slash parsing preserves arguments and rejects non-commands', () => {
    expect(parseSlashInvocation('/commit fix the parser')).toEqual({ name: 'commit', args: 'fix the parser' })
    expect(parseSlashInvocation(' /DL artifacts/out.zip ')).toEqual({ name: 'dl', args: 'artifacts/out.zip' })
    expect(parseSlashInvocation('// literal')).toBeNull()
    expect(parseSlashInvocation('ordinary text')).toBeNull()
  })

  test('command palette selection survives composer re-entry when the query is unchanged', () => {
    expect(normalizeCommandPaletteQuery('/// continue ')).toBe('continue')
    expect(shouldResetCommandPaletteSelection(true, '', '')).toBe(false)
    expect(shouldResetCommandPaletteSelection(true, 'co', 'co')).toBe(false)
    expect(shouldResetCommandPaletteSelection(false, '', '')).toBe(true)
    expect(shouldResetCommandPaletteSelection(true, 'co', 'com')).toBe(true)
  })
})
