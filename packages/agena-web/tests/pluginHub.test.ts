import { describe, expect, test } from 'bun:test'
import {
  parsePluginStatusList,
  pluginHubEntries,
  TELEGRAM_PLUGIN_ID,
  TELEGRAM_PAGE_ID,
} from '../src/features/pluginWeb/hub'
import { pluginPagePath } from '../src/features/pluginWeb/model'
import type { PluginPageCatalogItem } from '../src/features/pluginWeb/sdk'

const status = (plugin_id: string, state = 'running') => ({
  plugin_id,
  kind: 'static',
  state,
  summary: 'Adapter overview',
  translations: { 'zh-CN': { summary: '本地化插件说明' } },
})
const page: PluginPageCatalogItem = {
  plugin_id: 'demo.board',
  page_id: 'dashboard',
  title: 'Task board',
  api_version: 1,
  scope: 'instance',
  entry_url: '/plugin-assets/demo.board/r1/dashboard.js',
  style_urls: [],
  commands: [],
  settings: false,
  revision: 'r1',
  generation: 'g1',
}
describe('unified runtime-plugin inventory', () => {
  test('parses both runtime list envelopes and rejects duplicates', () => {
    const values = [status(TELEGRAM_PLUGIN_ID), status('demo.board')]
    expect(parsePluginStatusList(values)).toEqual(values)
    expect(parsePluginStatusList({ items: values })).toEqual(values)
    expect(() => parsePluginStatusList({ items: [values[0], values[0]] })).toThrow('Duplicate')
    expect(() => parsePluginStatusList({ unrelated: [] })).toThrow('Invalid')
  })
  test('Telegram is present without any surface.web entry and has its own native destination', () => {
    const all = pluginHubEntries(
      [status(TELEGRAM_PLUGIN_ID), status('agena.fs'), status('demo.board')],
      [page],
      'zh-CN',
    )
    expect(all).toHaveLength(3)
    const telegram = all.find((item) => item.plugin_id === TELEGRAM_PLUGIN_ID)!
    expect(telegram.native).toBe(true)
    expect(telegram.pages).toEqual([])
    expect(telegram.summary).toBe('本地化插件说明')
    expect(pluginPagePath(telegram.plugin_id, TELEGRAM_PAGE_ID)).toBe('/plugins/agena.telegram/manage')
    expect(all.find((item) => item.plugin_id === 'demo.board')?.pages).toEqual([page])
    expect(all.find((item) => item.plugin_id === 'agena.fs')?.pages).toEqual([])
  })
  test('keeps Web pages visible while the runtime inventory is being refreshed', () => {
    const entries = pluginHubEntries([], [page], 'en-US')
    expect(entries.map((item) => item.plugin_id)).toEqual(['demo.board'])
    expect(entries[0]!.state).toBe('unknown')
  })
})
