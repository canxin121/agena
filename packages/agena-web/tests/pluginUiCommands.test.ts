import assert from 'node:assert/strict'
import test from 'node:test'

import {
  executePluginSlashCommand,
  parsePluginCommandInput,
  type PluginSlashCommand,
} from '../src/lib/pluginUiCommands'

function command(overrides: Partial<PluginSlashCommand> = {}): PluginSlashCommand {
  return {
    pluginId: 'agena.memory',
    id: 'search',
    slash: '/memory-search',
    action: { kind: 'invoke_command', command: 'search' },
    ...overrides,
  }
}

test('plugin slash input maps shorthand into a single schema property', () => {
  assert.deepEqual(
    parsePluginCommandInput(
      command({
        inputSchema: {
          type: 'object',
          properties: { query: { type: 'string' } },
        },
      }),
      'release checklist',
    ),
    { query: 'release checklist' },
  )
})

test('plugin slash input parses named arguments and Agena aliases', () => {
  assert.deepEqual(
    parsePluginCommandInput(
      command({
        inputSchema: {
          type: 'object',
          properties: {
            query: { type: 'string', 'x-agena-aliases': ['q'] },
            limit: { type: 'integer' },
          },
        },
      }),
      'q=release limit=5',
    ),
    { query: 'release', limit: 5 },
  )
})

test('client-only plugin commands resolve without being sent as chat text', async () => {
  const selected = command({
    id: 'open',
    slash: '/memory',
    action: { kind: 'open_plugin_workbench', tab: 'config' },
  })
  assert.deepEqual(
    await executePluginSlashCommand({ command: selected, catalog: [selected], sessionId: 12, rawArgs: '' }),
    {
      kind: 'open_plugin_workbench',
      pluginId: 'agena.memory',
      tab: 'config',
    },
  )
})
