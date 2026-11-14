import { describe, expect, test } from 'bun:test'
import { MAIN_TABS, PRIMARY_MAIN_TABS, WORKSPACE_MAIN_TABS, mainTabFromPath } from '../src/app/navigation/mainTabs'
import {
  pluginInteractionPages,
  pluginInteractionLocation,
  pluginSettingsLocation,
  pluginReturnTab,
} from '../src/features/pluginWeb/settingsEntry'
import type { PluginPageCatalogItem } from '../src/features/pluginWeb/sdk'
const page: PluginPageCatalogItem = {
  plugin_id: 'demo.board',
  page_id: 'dashboard',
  title: 'Board',
  api_version: 1,
  scope: 'instance',
  entry_url: '/plugin-assets/demo.board/r1/index.js',
  style_urls: [],
  commands: [],
  settings: false,
  revision: 'r1',
  generation: 'g1',
}

describe('plugin applications launched from existing settings', () => {
  test('desktop and mobile omit a standalone plugin destination while deep-linked app tabs remain valid', () => {
    expect(PRIMARY_MAIN_TABS.some((item) => item.id === ('plugins' as never))).toBe(false)
    expect(MAIN_TABS.some((item) => item.id === ('plugins' as never))).toBe(false)
    expect(PRIMARY_MAIN_TABS.some((item) => item.id === 'settings')).toBe(true)
    expect(WORKSPACE_MAIN_TABS.some((item) => item.id === 'plugins')).toBe(true)
    expect(mainTabFromPath('/plugins/agena.telegram/manage')).toBe('plugins')
  })
  test('Telegram adds one native launch action without requiring a Web-module declaration', () => {
    expect(pluginInteractionPages('agena.telegram', [])).toEqual([
      { plugin_id: 'agena.telegram', page_id: 'manage', title: 'Telegram', scope: 'instance', native: true },
    ])
    expect(pluginInteractionPages('agena.fs', [])).toEqual([])
    expect(pluginInteractionPages('', [page])).toEqual([])
  })
  test('only the selected plugin contributes interaction pages, and an existing Telegram page is not duplicated', () => {
    const second = { ...page, page_id: 'history', title: 'History', scope: 'session' as const }
    const other = { ...page, plugin_id: 'other.plugin' }
    expect(pluginInteractionPages('demo.board', [page, other, second]).map((item) => item.page_id)).toEqual([
      'dashboard',
      'history',
    ])
    expect(
      pluginInteractionPages('agena.telegram', [{ ...page, plugin_id: 'agena.telegram', page_id: 'manage' }]),
    ).toHaveLength(1)
  })
  test('launch and return preserve the plugin and original settings tab without changing the settings route hierarchy', () => {
    expect(pluginInteractionLocation(page, 'commands')).toEqual({
      path: '/plugins/demo.board/dashboard',
      query: { settingsTab: 'commands' },
    })
    expect(pluginSettingsLocation('demo.board', 'commands')).toEqual({
      path: '/settings/plugins-tools',
      query: { view: 'plugin-workbench', plugin: 'demo.board', pluginTab: 'commands' },
    })
    expect(pluginSettingsLocation()).toEqual({ path: '/settings/plugins-tools', query: { view: 'plugin-workbench' } })
    expect(pluginSettingsLocation('agena.telegram', 'settings').query.pluginTab).toBe('settings')
  })
  test('session-scoped pages require explicit project selection and instance pages never inherit one', () => {
    expect(pluginInteractionLocation({ ...page, scope: 'session' }, 'tools', '42').query).toEqual({
      settingsTab: 'tools',
      sessionId: '42',
    })
    expect(pluginInteractionLocation(page, 'overview', '42').query).toEqual({ settingsTab: 'overview' })
    for (const value of [undefined, null, '0', '-1', '1e3', ['42'], 42])
      expect(pluginInteractionLocation({ ...page, scope: 'session' }, 'tools', value).query.sessionId).toBeUndefined()
  })
  test('only existing settings tabs can be used as return targets', () => {
    for (const tab of ['overview', 'settings', 'tools', 'commands', 'hooks', 'logs', 'diagnostics'])
      expect(pluginReturnTab(tab)).toBe(tab)
    for (const value of ['runtime', 'workspace', 'https://example.test', '/chat', undefined, ['settings']])
      expect(pluginReturnTab(value)).toBe('overview')
  })
})
