import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { i18n } from '../src/i18n'
import { settingsTextForLocale } from '../src/i18n/settingsTextCatalog'
import {
  buildPluginHookEntries,
  filterPluginHookEntries,
  pluginHookGroupLabel,
  pluginHookStateLabel,
  pluginHookTrustStatusLabel,
  type PluginHookRegistration,
} from '../src/components/settings/plugins/pluginHooks'
import { pluginTabForSettingsView } from '../src/components/settings/pluginSettingsNavigation'
import { settingsQueryForDestination } from '../src/components/settings/settingsDestination'

const registration: PluginHookRegistration = {
  plugin_id: 'fixture',
  trust_level: 'verified',
  trust_status: 'trusted',
  provenance: ['signed'],
  source: 'cdylib',
  source_path: '/plugins/fixture.so',
  manifest_hash: 'hash-1',
  hooks: [
    {
      name: 'tool.execute.before',
      enabled: true,
      source: 'cdylib',
      trust_level: 'verified',
      trust_status: 'trusted',
      current_hash: 'hash-2',
    },
    { name: 'event', enabled: false, source: 'cdylib' },
    { name: 'future.hook', enabled: true, source: 'cdylib' },
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
    const hooks = buildPluginHookEntries(
      ['tool.execute.before', 'event', 'chat.message', 'tool.execute.before'],
      [registration],
      'running',
    )
    expect(hooks).toHaveLength(4)
    expect(new Set(hooks.map((hook) => hook.name)).size).toBe(4)
    expect(hooks.find((hook) => hook.name === 'tool.execute.before')).toMatchObject({
      state: 'active',
      declared: true,
      registered: true,
      source: 'cdylib',
      sourcePath: '/plugins/fixture.so',
      trustLevel: 'verified',
      trustStatus: 'trusted',
      currentHash: 'hash-2',
    })
    expect(hooks.find((hook) => hook.name === 'event')).toMatchObject({
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
      title: 'future.hook',
    })
  })

  test('a stopped or failed plugin does not claim registrations are active', () => {
    for (const state of ['stopped', 'failed', 'restarting', undefined]) {
      const hooks = buildPluginHookEntries([], [registration], state)
      expect(hooks.find((hook) => hook.name === 'tool.execute.before')?.state).toBe('inactive')
      expect(hooks.find((hook) => hook.name === 'event')?.state).toBe('disabled')
    }
  })

  test('empty or malformed data is handled without inventing registrations', () => {
    expect(buildPluginHookEntries(null, undefined, 'running')).toEqual([])
    expect(buildPluginHookEntries(['', null, 'init'], [], 'running')).toMatchObject([
      { name: 'init', state: 'declared' },
    ])
  })

  test('search matches identifiers, translated names, descriptions and states', () => {
    const previous = i18n.global.locale.value
    try {
      i18n.global.locale.value = 'zh-CN'
      const hooks = buildPluginHookEntries(['tool.execute.before', 'session.start'], [registration], 'running')
      expect(filterPluginHookEntries(hooks, 'tool.execute.before')).toHaveLength(1)
      expect(filterPluginHookEntries(hooks, '工具执行前').map((hook) => hook.name)).toEqual(['tool.execute.before'])
      expect(filterPluginHookEntries(hooks, '生效').some((hook) => hook.name === 'tool.execute.before')).toBe(true)
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
    const hooks = buildPluginHookEntries(ids, undefined, 'running')
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

  test('Hook navigation uses one localized language, not a parenthetical English gloss', () => {
    expect(settingsTextForLocale('zh-CN', 'Hooks')).toBe('钩子')
    expect(settingsTextForLocale('es-ES', 'Hooks')).toBe('Ganchos')
    expect(settingsTextForLocale('fr-FR', 'Hooks')).toBe('Points d’accroche')
    expect(settingsTextForLocale('pt-BR', 'Hooks')).toBe('Ganchos')
  })
})
