import assert from 'node:assert/strict'
import test from 'node:test'

import { localizePluginManifest } from '../src/lib/pluginDocumentation'

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
