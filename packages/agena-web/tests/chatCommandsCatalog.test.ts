import { describe, expect, test } from 'bun:test'

import {
  BUILT_IN_COMMANDS,
  findBuiltInCommand,
  paletteInvocation,
  parseSlashInvocation,
  schemaNeedsPluginInput,
  schemaRequiresArguments,
} from '../src/pages/chat/chatCommandsCatalog'

describe('web command catalog', () => {
  test('contains the same built-in names as the TUI catalog', () => {
    expect(BUILT_IN_COMMANDS.map((command) => command.name)).toEqual([
      'help',
      'commands',
      'new',
      'sessions',
      'hub',
      'lineage',
      'rewind',
      'rename',
      'timeline',
      'settings',
      'model',
      'review',
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
      'skill',
      'skill-manager',
      'download',
      'editor',
      'image',
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
    ])
  })

  test('aliases resolve to the canonical command and required arguments stay explicit', () => {
    expect(findBuiltInCommand('resume-run')?.name).toBe('continue')
    expect(findBuiltInCommand('/copy-last')?.name).toBe('copy-message')
    expect(findBuiltInCommand('dl')?.name).toBe('download')
    expect(paletteInvocation(findBuiltInCommand('commit')!)).toBe('/commit <message>')
    expect(paletteInvocation(findBuiltInCommand('pr')!)).toBe('/pr <title>')
    expect(paletteInvocation(findBuiltInCommand('review')!)).toBe('/review')
  })

  test('slash parsing preserves arguments and rejects non-commands', () => {
    expect(parseSlashInvocation('/commit fix the parser')).toEqual({ name: 'commit', args: 'fix the parser' })
    expect(parseSlashInvocation(' /DL artifacts/out.zip ')).toEqual({ name: 'dl', args: 'artifacts/out.zip' })
    expect(parseSlashInvocation('// literal')).toBeNull()
    expect(parseSlashInvocation('ordinary text')).toBeNull()
  })

  test('plugin schemas only require palette input when required fields exist', () => {
    expect(schemaRequiresArguments({ type: 'object', properties: { name: { type: 'string' } } })).toBe(false)
    expect(schemaRequiresArguments({ type: 'object', required: ['name'] })).toBe(true)
    expect(schemaNeedsPluginInput({ type: 'object', properties: { name: { type: 'string' } } })).toBe(false)
    expect(schemaNeedsPluginInput({ type: 'object', required: ['name'] })).toBe(true)
    expect(schemaNeedsPluginInput({ type: 'string' })).toBe(true)
    expect(schemaNeedsPluginInput({ description: 'an empty object is valid' })).toBe(true)
    expect(schemaNeedsPluginInput({})).toBe(false)
    expect(schemaNeedsPluginInput({ type: 'object', minProperties: 1 })).toBe(true)
    expect(schemaNeedsPluginInput(true)).toBe(false)
    expect(schemaNeedsPluginInput(false)).toBe(true)
  })
})
