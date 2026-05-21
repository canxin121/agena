import { describe, expect, test } from 'bun:test'
import { ref } from 'vue'

import { useSettingsPluginsState, type SettingsPluginsStateInput } from './useSettingsPluginsState'

describe('useSettingsPluginsState', () => {
  test('summarizes plugin config and patches plugin settings', async () => {
    const calls: Array<Record<string, unknown>> = []
    const input: SettingsPluginsStateInput = {
      actionError: ref(''),
      actionMessage: ref(''),
      load: async () => {
        calls.push({ kind: 'load' })
      },
      settingsPlugins: ref({
        configPath: '/workspace/.agena/config.json',
        configFound: true,
        enabled: false,
        defaultMode: 'help',
      fileEnabled: true,
      fileDefaultMode: 'detailed',
      pluginEntries: [
        {
          pluginId: 'demo.plugin',
          kind: 'stdio',
          disabled: false,
          source: 'file',
          filePresent: true,
          entry: {
            kind: 'stdio',
            command: 'demo',
            args: [],
            env: {},
            cwd: null,
            restart: {},
            options: {},
            timeouts: {},
            disabled: false,
          },
        },
      ],
      toolPresentationPluginOverridesCount: 1,
      toolPresentationToolOverridesCount: 2,
    }),
    }

    const state = useSettingsPluginsState(input, {
      patchSettings: async (patch) => {
        calls.push(patch)
        return {
          config_path: '/workspace/.agena/config.json',
          config_found: true,
          operation: 'patch',
          path: patch.path ?? null,
          dry_run: false,
          changed: true,
          created: false,
          deleted: false,
          validated: true,
          reload_requested: true,
          reload_required: false,
          reload: null,
          previous: {},
          current: {},
        }
      },
      setSettings: async (set) => {
        calls.push(set)
        return {
          config_path: '/workspace/.agena/config.json',
          config_found: true,
          operation: 'set',
          path: set.path,
          dry_run: false,
          changed: true,
          created: false,
          deleted: false,
          validated: true,
          reload_requested: true,
          reload_required: false,
          reload: null,
          previous: {},
          current: {},
        }
      },
    })

    expect(state.summaryFacts.value[0]?.value).toBe('/workspace/.agena/config.json')
    expect(state.summaryFacts.value[2]?.value).toBe('off')
    expect(state.summaryFacts.value[3]?.value).toBe('Help')
    expect(state.summaryFacts.value[4]?.value).toContain('enabled=on')

    await state.togglePluginsEnabled()
    expect(calls[0]).toEqual({
      path: 'plugins',
      changes: { enabled: true },
      validate: true,
      reload: true,
    })
    expect(calls[1]).toEqual({ kind: 'load' })
    expect(input.actionMessage.value).toBe('Plugins enabled; runtime reloaded.')

    calls.length = 0
    await state.setDefaultToolDescriptionMode('detailed')
    expect(calls[0]).toEqual({
      path: 'plugins.tool_presentation',
      changes: { default_mode: 'detailed' },
      validate: true,
      reload: true,
    })
    expect(calls[1]).toEqual({ kind: 'load' })

    calls.length = 0
    await state.togglePluginEntryDisabled(input.settingsPlugins.value!.pluginEntries[0])
    expect(calls[0]).toEqual({
      path: 'plugins.list."demo.plugin"',
      value: expect.objectContaining({
        kind: 'stdio',
        disabled: true,
      }),
      validate: true,
      reload: true,
    })
  })
})
