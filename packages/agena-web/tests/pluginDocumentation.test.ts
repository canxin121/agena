import assert from 'node:assert/strict'
import test from 'node:test'

import { localizePluginManifest, localizePluginSettings } from '../src/lib/pluginDocumentation'
import type { PluginSettingsContract } from '../src/lib/pluginOperations'

test('plugin workbench documentation follows exact locale, language, English, then base fallback', () => {
  const manifest = {
    summary: 'Stable plugin summary',
    help: 'Stable plugin help',
    translations: {
      'zh-CN': { summary: '插件摘要' },
      en: { help: 'Plugin help' },
    },
    tools: [
      {
        name: 'inspect',
        summary: 'Stable tool summary',
        docs: {
          summary: 'Stable tool summary',
          help: 'Stable tool help',
          translations: { zh: { summary: '工具摘要', help: '工具帮助' } },
        },
      },
    ],
  }

  const chinese = localizePluginManifest(manifest, 'zh_CN')
  assert.equal(chinese.summary, '插件摘要')
  assert.equal(chinese.help, 'Plugin help')
  assert.equal(chinese.tools?.[0]?.summary, '工具摘要')
  assert.equal(chinese.tools?.[0]?.docs?.help, '工具帮助')
  assert.equal(manifest.summary, 'Stable plugin summary', 'localization must not mutate the host response')

  const unknown = localizePluginManifest(manifest, 'xx-YY')
  assert.equal(unknown.summary, 'Stable plugin summary')
  assert.equal(unknown.help, 'Plugin help')
  assert.equal(unknown.tools?.[0]?.summary, 'Stable tool summary')
})

test('settings localization reaches nested fields, collection items, options and variant branches without changing values', () => {
  const settings: PluginSettingsContract = {
    version: 1,
    root: {
      id: 'root', path: '', kind: 'object', title: 'Settings', description: 'Settings help',
      fields: [
        {
          id: 'servers', path: '/servers', kind: 'record', title: 'Servers',
          value: {
            id: 'server', path: '/servers/*', kind: 'tagged_variant', title: 'Server',
            discriminator: 'transport', default: { transport: 'stdio', args: ['Arguments'] },
            variants: [{
              id: 'stdio', title: 'Standard I/O', description: 'Start a process', tag: 'stdio',
              fields: [{
                id: 'args', path: '/servers/*/args', kind: 'list', title: 'Arguments',
                constraints: { max_items: 12 }, default: ['Arguments'],
                item: { id: 'arg', path: '/servers/*/args/*', kind: 'text', title: 'Argument', sensitive: true },
              }],
            }],
          },
        },
        {
          id: 'mode', path: '/mode', kind: 'choice', title: 'Mode', default: 'automatic', required: true,
          options: [{ id: 'auto', title: 'Automatic', description: 'Choose automatically', value: 'automatic' }],
        },
        {
          id: 'features', path: '/features', kind: 'multi_choice', title: 'Features', default: ['fast'],
          options: [{ id: 'fast', title: 'Fast', value: 'fast' }],
        },
      ],
    },
  }
  const original = structuredClone(settings)
  const copy = {
    Settings: '设置', 'Settings help': '设置说明', Servers: '服务器', Server: '服务器条目',
    'Standard I/O': '标准输入输出', 'Start a process': '启动进程', Arguments: '参数', Argument: '参数项',
    Mode: '模式', Automatic: '自动', 'Choose automatically': '自动选择', Features: '功能', Fast: '快速',
  }
  const localized = localizePluginManifest({ settings, translations: { 'zh-CN': { settings: copy } } }, 'zh_CN')
  const root = localized.settings.root
  assert.equal(root.title, '设置')
  assert.equal(root.description, '设置说明')
  const record = root.fields![0]!
  assert.equal(record.title, '服务器')
  assert.equal(record.value?.title, '服务器条目')
  const branch = record.value!.variants![0]!
  assert.equal(branch.title, '标准输入输出')
  assert.equal(branch.description, '启动进程')
  const list = branch.fields![0]!
  assert.equal(list.title, '参数')
  assert.equal(list.item?.title, '参数项')
  assert.equal(root.fields![1]!.options![0]!.title, '自动')
  assert.equal(root.fields![1]!.options![0]!.description, '自动选择')
  assert.equal(root.fields![2]!.options![0]!.title, '快速')
  assert.deepEqual(settings, original, 'the host-owned contract must remain canonical')
  assert.equal(localized.settings.version, settings.version)
  assert.deepEqual(record.value!.default, { transport: 'stdio', args: ['Arguments'] })
  assert.equal(record.value!.discriminator, 'transport')
  assert.equal(branch.id, 'stdio')
  assert.equal(branch.tag, 'stdio')
  assert.equal(list.id, 'args')
  assert.equal(list.path, '/servers/*/args')
  assert.deepEqual(list.constraints, { max_items: 12 })
  assert.deepEqual(list.default, ['Arguments'])
  assert.equal(list.item!.sensitive, true)
  assert.equal(root.fields![1]!.options![0]!.value, 'automatic')
  assert.equal(root.fields![1]!.required, true)
  assert.deepEqual(root.fields![2]!.default, ['fast'])
})

test('settings copy falls back per string through exact locale, language, English and canonical text', () => {
  const contract: PluginSettingsContract = {
    version: 1,
    root: {
      id: 'root', path: '', kind: 'choice', title: 'Mode', description: 'Mode help',
      options: [
        { id: 'one', title: 'One', description: 'One help', value: 1 },
        { id: 'two', title: 'Two', description: 'Canonical help', value: 2 },
      ],
    },
  }
  const translations = {
    'ZH-cn': { settings: { Mode: ' 模式 ', 'Mode help': ' ' } },
    zh: { settings: { Mode: '语言级模式', 'Mode help': '模式说明' } },
    'en-US': { settings: { One: 'English one' } },
    en: { settings: { 'One help': 'English help' } },
  }
  const chinese = localizePluginSettings(contract, translations, 'zh_CN')
  assert.equal(chinese.root.title, '模式')
  assert.equal(chinese.root.description, '模式说明')
  assert.equal(chinese.root.options![0]!.title, 'English one')
  assert.equal(chinese.root.options![0]!.description, 'English help')
  assert.equal(chinese.root.options![1]!.title, 'Two')
  assert.equal(chinese.root.options![1]!.description, 'Canonical help')
  const unknown = localizePluginSettings(contract, translations, 'xx-YY')
  assert.equal(unknown.root.title, 'Mode')
  assert.equal(unknown.root.options![0]!.title, 'English one')
  assert.deepEqual(JSON.parse(JSON.stringify(localizePluginSettings(contract, undefined, 'zh-CN'))), contract)
})
