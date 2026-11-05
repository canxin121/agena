import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { i18n } from '../src/i18n'
import { settingsTextForLocale } from '../src/i18n/settingsTextCatalog'
import {
  buildPluginHookEntries,
  filterPluginHookEntries,
  groupPluginHookEntries,
  pluginHookGroupLabel,
  pluginHookStateLabel,
  pluginHookTrustStatusLabel,
  type PluginHookDescriptor,
  type PluginHookRegistration,
} from '../src/components/settings/plugins/pluginHooks'
import { pluginTabForSettingsView } from '../src/components/settings/pluginSettingsNavigation'
import { settingsQueryForDestination } from '../src/components/settings/settingsDestination'

function descriptor(
  name: string,
  handler_id: string,
  summary: string,
  help: string,
  enabled = true,
): PluginHookDescriptor {
  return {
    name, handler_id, summary, help, enabled,
    translations: {}, priority: 0, filters: {}, is_async: true,
    source: 'cdylib', trust_level: 'verified', trust_status: 'trusted',
    current_hash: 'hash-2',
  }
}

const registration: PluginHookRegistration = {
  plugin_id: 'fixture',
  trust_level: 'verified',
  trust_status: 'trusted',
  provenance: ['signed'],
  source: 'cdylib',
  source_path: '/plugins/fixture.so',
  manifest_hash: 'hash-1',
  hooks: [
    descriptor('tool.before', 'check_tool', 'Inspect tool input', 'Checks tool arguments before execution.'),
    descriptor('notification', 'notify', 'Publish notifications', 'Sends notification events to observers.', false),
    descriptor('future.hook', 'future_handler', 'A future custom Hook', 'Runs when a future event is dispatched.'),
  ],
}

describe('plugin Hook registration inspector', () => {
  test('the plugin workbench accepts bookmarkable Hooks tabs and preserves plugin selection', () => {
    expect(pluginTabForSettingsView('plugin-workbench', 'hooks')).toBe('hooks')
    const route = { view: 'plugin-workbench', plugin: 'fixture', pluginTab: 'hooks', scope: 'global' }
    expect(
      settingsQueryForDestination({ section: 'plugins-tools', view: 'plugin-workbench' }, 'plugins-tools', route),
    ).toEqual({ view: 'plugin-workbench', plugin: 'fixture', pluginTab: 'hooks' })
  })

  test('combines declared and runtime Hook entries without duplicates', () => {
    const declaredHandlers = [
      {
        id: 'inspect_chat',
        event: 'chat.message',
        summary: 'Inspect chat messages',
        help: 'Checks each provider-bound chat message before sending.',
        priority: 0,
        filters: {},
        is_async: true,
      },
    ]
    const hooks = buildPluginHookEntries(
      ['tool.before', 'notification', 'chat.message', 'tool.before'],
      [registration],
      'running',
      declaredHandlers,
    )
    expect(hooks).toHaveLength(4)
    expect(new Set(hooks.map((hook) => hook.name)).size).toBe(4)
    expect(hooks.find((hook) => hook.name === 'tool.before')).toMatchObject({
      state: 'active',
      declared: true,
      registered: true,
      source: 'cdylib',
      sourcePath: '/plugins/fixture.so',
      trustLevel: 'verified',
      trustStatus: 'trusted',
      currentHash: 'hash-2',
    })
    expect(hooks.find((hook) => hook.name === 'notification')).toMatchObject({
      state: 'disabled',
      declared: true,
      registered: true,
    })
    expect(hooks.find((hook) => hook.name === 'chat.message')).toMatchObject({
      state: 'declared',
      declared: true,
      registered: false,
    })
    expect(hooks.find((hook) => hook.name === 'future.hook')).toMatchObject({
      state: 'active',
      declared: false,
      registered: true,
      title: 'A future custom Hook',
    })
  })

  test('each handler has its own introduction, priority, filters and author translations', () => {
    const declarations = [
      {
        id: 'check_shell', event: 'tool.before',
        summary: 'Check shell commands', help: 'Reviews shell commands before execution.',
        translations: {
          'zh-CN': { summary: '检查 Shell 命令', help: '在执行前检查 Shell 命令。' },
          'fr-FR': { summary: 'Vérifier les commandes shell', help: 'Vérifie les commandes shell avant leur exécution.' },
        },
        priority: 100, filters: { tools: ['bash'] }, is_async: true,
      },
      {
        id: 'protect_files', event: 'tool.before',
        summary: 'Protect file writes', help: 'Checks paths before writing.',
        priority: 80, filters: { tools: ['fs.write'] }, is_async: true,
      },
    ]
    const live = [{
      ...registration,
      hooks: declarations.map((handler) => ({
        ...descriptor(handler.event, handler.id, handler.summary, handler.help),
        priority: handler.priority,
        filters: handler.filters,
        translations: handler.translations || {},
        is_async: handler.is_async,
      })),
    }]
    const chinese = buildPluginHookEntries(['tool.before'], live, 'running', declarations, 'zh-CN')
    expect(chinese).toHaveLength(2)
    expect(chinese.map((entry) => entry.handlerId)).toEqual(['check_shell', 'protect_files'])
    expect(chinese.map((entry) => entry.title)).toEqual(['检查 Shell 命令', 'Protect file writes'])
    expect(chinese[0]).toMatchObject({
      description: '在执行前检查 Shell 命令。',
      filters: { tools: ['bash'] }, priority: 100, isAsync: true, state: 'active',
    })
    expect(new Set(chinese.map((entry) => entry.key)).size).toBe(2)
    const groups = groupPluginHookEntries(chinese)
    expect(groups).toHaveLength(1)
    expect(groups[0]).toMatchObject({
      group: 'tools',
      event: 'tool.before',
    })
    expect(groups[0].hooks.map((entry) => entry.handlerId)).toEqual(['check_shell', 'protect_files'])
    expect(filterPluginHookEntries(chinese, 'check_shell')).toHaveLength(1)
    expect(filterPluginHookEntries(chinese, 'fs.write')).toHaveLength(1)

    const french = buildPluginHookEntries(['tool.before'], live, 'running', declarations, 'fr-FR')
    expect(french[0].title).toBe('Vérifier les commandes shell')
    expect(french[0].description).toBe('Vérifie les commandes shell avant leur exécution.')
    const spanish = buildPluginHookEntries(['tool.before'], live, 'running', declarations, 'es-ES')
    expect(spanish[0].title).toBe('Check shell commands')

    const stopped = buildPluginHookEntries(['tool.before'], [], 'stopped', declarations, 'zh-CN')
    expect(stopped).toHaveLength(2)
    expect(stopped.every((entry) => entry.state === 'declared')).toBe(true)
    expect(stopped[0].title).toBe('检查 Shell 命令')
    expect(groupPluginHookEntries(stopped)[0].hooks).toHaveLength(2)

    // A bare subscription is not itself a registered, self-documented Handler.
    expect(buildPluginHookEntries(['tool.before'], [], 'stopped')).toEqual([])
  })

  test('a stopped or failed plugin does not claim registrations are active', () => {
    for (const state of ['stopped', 'failed', 'restarting', undefined]) {
      const hooks = buildPluginHookEntries([], [registration], state)
      expect(hooks.find((hook) => hook.name === 'tool.before')?.state).toBe('inactive')
      expect(hooks.find((hook) => hook.name === 'notification')?.state).toBe('disabled')
    }
  })

  test('empty or malformed data is handled without inventing registrations', () => {
    expect(buildPluginHookEntries(null, undefined, 'running')).toEqual([])
    expect(buildPluginHookEntries(['', null, 'init'], [], 'running')).toEqual([])
  })

  test('search matches identifiers, translated names, descriptions and states', () => {
    const previous = i18n.global.locale.value
    try {
      i18n.global.locale.value = 'zh-CN'
      const hooks = buildPluginHookEntries(['tool.before', 'session.start'], [registration], 'running')
      expect(filterPluginHookEntries(hooks, 'tool.before')).toHaveLength(1)
      expect(filterPluginHookEntries(hooks, 'Inspect tool input').map((hook) => hook.name)).toEqual(['tool.before'])
      expect(filterPluginHookEntries(hooks, '生效').some((hook) => hook.name === 'tool.before')).toBe(true)
      expect(filterPluginHookEntries(hooks, 'not-a-hook')).toEqual([])
      expect(pluginHookGroupLabel('tools')).toBe('工具钩子')
      expect(pluginHookStateLabel('declared')).toBe('仅声明，未注册')
      expect(pluginHookTrustStatusLabel('trusted')).toBe('受信任')
    } finally {
      i18n.global.locale.value = previous
    }
  })

  test('covers every canonical Hook subscription name in the SDK', () => {
    const manifest = readFileSync(new URL('../../../crates/agena-plugin-sdk/src/manifest.rs', import.meta.url), 'utf8')
    const section = manifest.split('const HOOK_NAMES:')[1]?.split('];')[0] || ''
    const ids = [...section.matchAll(/"([a-z][a-z_.]+)"/g)].map((match) => match[1])
    expect(ids.length).toBeGreaterThan(20)
    expect(new Set(ids).size).toBe(ids.length)
    const declarations = ids.map((event) => ({
      id: `handler_${event}`,
      event,
      summary: `Documented ${event}`,
      help: `Handles the ${event} event with plugin-authored metadata.`,
      priority: 0,
      filters: {},
      is_async: true,
    }))
    const hooks = buildPluginHookEntries(ids, undefined, 'running', declarations)
    expect(hooks).toHaveLength(ids.length)
    expect(
      hooks.every(
        (hook) => hook.title !== hook.name && hook.description !== settingsTextForLocale('en-US', 'Unknown hook type.'),
      ),
    ).toBe(true)
  })

  test('all supported locales translate the Hooks navigation and common statuses', () => {
    const locales = ['zh-CN', 'es-ES', 'fr-FR', 'pt-BR', 'hi-IN', 'ar-SA'] as const
    for (const locale of locales) {
      for (const key of ['Hooks', 'Search hooks', 'Registered and active', 'Declared only', 'Lifecycle hooks']) {
        expect(settingsTextForLocale(locale, key)).not.toBe(key)
      }
    }
  })

  test('all supported locales translate each new Hook lifecycle label and execution mode', () => {
    const locales = ['zh-CN', 'es-ES', 'fr-FR', 'pt-BR', 'hi-IN', 'ar-SA'] as const
    const keys = [
      'Runs once when a session execution begins.',
      'Runs once after session execution settles.',
      'Before compaction',
      'Guards the start of a context compaction operation.',
      'After compaction',
      'Observes the committed outcome of a compaction attempt.',
      'Before subagent launch',
      'Guards a delegated agent before its initial execution.',
      'Subagent settled',
      'Observes a delegated agent after terminal settlement.',
      'Permission request',
      'Applies additional policy before interactive tool approval.',
      'Before model request',
      'Guards the model request before provider I/O.',
      'After model request',
      'Observes the settled model request status and usage.',
      'Model request failure',
      'May request a bounded retry of a failed, side-effect-free round.',
      'Asynchronous',
      'Synchronous',
    ]
    for (const locale of locales) {
      for (const key of keys) {
        expect(settingsTextForLocale(locale, key)).not.toBe(key)
      }
    }
  })

  test('Hook navigation uses one localized language, not a parenthetical English gloss', () => {
    expect(settingsTextForLocale('zh-CN', 'Hooks')).toBe('钩子')
    expect(settingsTextForLocale('es-ES', 'Hooks')).toBe('Ganchos')
    expect(settingsTextForLocale('fr-FR', 'Hooks')).toBe('Points d’accroche')
    expect(settingsTextForLocale('pt-BR', 'Hooks')).toBe('Ganchos')
  })
})
