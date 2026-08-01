import { describe, expect, test } from 'bun:test'

import { resolvePluginCommandOutput } from './pluginUiActionRuntime'

describe('resolvePluginCommandOutput', () => {
  test('returns message output as a notice', async () => {
    const effect = await resolvePluginCommandOutput({
      pluginId: 'example.notes',
      result: {
        kind: 'message',
        text: 'hello from plugin',
      },
    })

    expect(effect).toEqual({ kind: 'notice', message: 'hello from plugin' })
  })

  test('passes through route and url effects', async () => {
    expect(
      await resolvePluginCommandOutput({
        pluginId: 'example.notes',
        result: {
          kind: 'open_route',
          route: '/plugins',
        },
      }),
    ).toEqual({ kind: 'open_route', route: '/plugins' })

    expect(
      await resolvePluginCommandOutput({
        pluginId: 'example.notes',
        result: {
          kind: 'open_url',
          url: 'https://example.com',
        },
      }),
    ).toEqual({ kind: 'open_url', url: 'https://example.com' })
  })

  test('turns nested tool output into submit prompt when requested', async () => {
    const effect = await resolvePluginCommandOutput({
      pluginId: 'example.notes',
      result: {
        kind: 'invoke_tool',
        tool: 'format',
        input: { text: 'hi' },
        submit_output_as_prompt: true,
      },
      invokeTool: async () => ({
        plugin_id: 'example.notes',
        tool: 'format',
        status: 'completed',
        title: 'Format',
        output_text: 'formatted hi',
        payload: null,
        metadata: {},
      }),
    })

    expect(effect).toEqual({ kind: 'submit_prompt', prompt: 'formatted hi' })
  })

  test('turns nested tool output into a notice when prompt submission is disabled', async () => {
    const effect = await resolvePluginCommandOutput({
      pluginId: 'example.notes',
      result: {
        kind: 'invoke_tool',
        tool: 'format',
        input: { text: 'hi' },
      },
      invokeTool: async () => ({
        plugin_id: 'example.notes',
        tool: 'format',
        status: 'completed',
        title: 'Format',
        output_text: 'formatted hi',
        payload: null,
        metadata: {},
      }),
    })

    expect(effect).toEqual({ kind: 'notice', message: 'formatted hi' })
  })

  test('never submits a non-completed tool outcome as a model prompt', async () => {
    const effect = await resolvePluginCommandOutput({
      pluginId: 'example.notes',
      result: {
        kind: 'invoke_tool',
        tool: 'publish',
        input: { text: 'hi' },
        submit_output_as_prompt: true,
      },
      invokeTool: async () => ({
        plugin_id: 'example.notes',
        tool: 'publish',
        status: 'capability_unavailable',
        title: 'Capability unavailable',
        output_text: 'Publishing is unavailable in this runtime.',
        payload: { status: 'capability_unavailable' },
        metadata: {},
      }),
    })

    expect(effect).toEqual({ kind: 'notice', message: 'Publishing is unavailable in this runtime.' })
  })

  test('resolves nested invoke_command outputs recursively', async () => {
    const effect = await resolvePluginCommandOutput({
      pluginId: 'example.notes',
      result: {
        kind: 'invoke_command',
        command: 'example.notes.inline',
        input: { name: 'Ada' },
      },
      invokeCommand: async () => ({
        plugin_id: 'example.notes',
        action_id: 'example.notes.inline',
        action: {
          kind: 'invoke_command',
          command: 'example.notes.inline',
          input: { name: 'Ada' },
        },
        result: {
          kind: 'message',
          text: 'hello Ada',
        },
      }),
    })

    expect(effect).toEqual({ kind: 'notice', message: 'hello Ada' })
  })

  test('falls back to the provided default notice when output is empty or unknown', async () => {
    const fallbackNotice = 'Ran plugin command.'

    expect(
      await resolvePluginCommandOutput({
        pluginId: 'example.notes',
        result: {
          kind: 'none',
        },
        fallbackNotice,
      }),
    ).toEqual({ kind: 'notice', message: fallbackNotice })

    expect(
      await resolvePluginCommandOutput({
        pluginId: 'example.notes',
        result: null,
        fallbackNotice,
      }),
    ).toEqual({ kind: 'notice', message: fallbackNotice })
  })
})
