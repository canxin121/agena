import { describe, expect, test } from 'bun:test'

import type { PluginStudioCommand } from './agenaApi'
import { buildPluginCommandPayload } from './commandPalette'

function pluginCommand(overrides: Partial<PluginStudioCommand> = {}): PluginStudioCommand {
  return {
    plugin_id: 'example.notes',
    id: 'example.notes.write',
    title: 'Write Note',
    description: 'Write a note.',
    category: 'Plugin',
    location: 'command_palette',
    action: { kind: 'invoke_command', command: 'example.notes.write' },
    ...overrides,
  }
}

describe('buildPluginCommandPayload', () => {
  test('parses JSON object input when schema is structured', () => {
    const command = pluginCommand({
      input_schema: {
        type: 'object',
        properties: {
          name: { type: 'string' },
        },
      },
    })

    expect(
      buildPluginCommandPayload(command, {
        input: '/write {"name":"Ada"}',
        args: ['{"name":"Ada"}'],
      }),
    ).toEqual({ name: 'Ada' })
  })

  test('maps bare text to the single object field', () => {
    const command = pluginCommand({
      input_schema: {
        type: 'object',
        properties: {
          name: { type: 'string' },
        },
      },
    })

    expect(
      buildPluginCommandPayload(command, {
        input: '/write Ada',
        args: ['Ada'],
      }),
    ).toEqual({ name: 'Ada' })
  })

  test('parses bare literals for single-field object schemas using the field type', () => {
    const command = pluginCommand({
      input_schema: {
        type: 'object',
        properties: {
          count: { type: 'integer' },
        },
      },
    })

    expect(
      buildPluginCommandPayload(command, {
        input: '/write 3',
        args: ['3'],
      }),
    ).toEqual({ count: 3 })
  })

  test('supports key=value shorthand for multi-field object schemas', () => {
    const command = pluginCommand({
      input_schema: {
        type: 'object',
        properties: {
          path: { type: 'string' },
          force: { type: 'boolean' },
          count: { type: 'integer' },
        },
      },
    })

    expect(
      buildPluginCommandPayload(command, {
        input: '/write path=123 force=true count=3',
        args: ['path=123', 'force=true', 'count=3'],
      }),
    ).toEqual({ path: '123', force: true, count: 3 })
  })

  test('supports key=value shorthand with schema aliases', () => {
    const command = pluginCommand({
      input_schema: {
        type: 'object',
        properties: {
          filePath: {
            type: 'string',
            'x-agena-aliases': ['path'],
          },
          count: { type: 'integer' },
        },
      },
    })

    expect(
      buildPluginCommandPayload(command, {
        input: '/write path=README.md count=3',
        args: ['path=README.md', 'count=3'],
      }),
    ).toEqual({ filePath: 'README.md', count: 3 })
  })

  test('returns raw string for top-level string schemas', () => {
    const command = pluginCommand({
      input_schema: {
        type: 'string',
      },
    })

    expect(
      buildPluginCommandPayload(command, {
        input: '/write hello',
        args: ['hello'],
      }),
    ).toBe('hello')
  })

  test('parses bare literals for top-level boolean schemas', () => {
    const command = pluginCommand({
      input_schema: {
        type: 'boolean',
      },
    })

    expect(
      buildPluginCommandPayload(command, {
        input: '/write true',
        args: ['true'],
      }),
    ).toBe(true)
  })

  test('parses bare literals for top-level integer schemas', () => {
    const command = pluginCommand({
      input_schema: {
        type: 'integer',
      },
    })

    expect(
      buildPluginCommandPayload(command, {
        input: '/write 3',
        args: ['3'],
      }),
    ).toBe(3)
  })

  test('falls back to legacy args envelope without schema', () => {
    const command = pluginCommand()

    expect(
      buildPluginCommandPayload(command, {
        input: '/write hello world',
        args: ['hello', 'world'],
      }),
    ).toEqual({ args: 'hello world' })
  })
})
