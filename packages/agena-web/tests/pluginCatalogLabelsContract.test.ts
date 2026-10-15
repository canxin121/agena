import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

const read = (path: string) => readFileSync(resolve(import.meta.dir, path), 'utf8')

const LABEL_KEYS = ['Running', 'Restarting', 'Failed', 'Stopped']

const LOCALES = ['en-US', 'zh-CN', 'es-ES', 'fr-FR', 'hi-IN', 'ar-SA', 'pt-BR']

test('the plugin catalog presents purpose and localized state without transport categories', () => {
  const panel = read('../src/components/settings/PluginsPanel.vue')
  // The host reports machine values; the panel must not print them directly.
  assert.ok(!panel.includes('{{ status.kind }}'))
  assert.ok(!panel.includes('{{ selectedStatus.state }}'))
  assert.ok(!panel.includes('pluginKindLabel'))
  assert.ok(!panel.includes('transportFilter'))
  assert.ok(panel.includes('pluginSummaries.get(status.plugin_id) || pluginStateLabel(status.state)'))
  assert.ok(panel.includes('localizedDocumentationField(status.summary, status.translations'))
  assert.match(panel, /\{\{\s*pluginStateLabel\(selectedStatus\.state\)\s*\}\}/)
  // A filter option keeps the raw value as its model and shows the label.
  assert.ok(panel.includes('label: pluginStateLabel(value)'))
})

test('every plugin lifecycle state resolves a catalog label', () => {
  const labels = read('../src/components/settings/pluginLabels.ts')
  for (const key of LABEL_KEYS) assert.ok(labels.includes(`st('${key}')`), key)
})

test('every settings catalog translates the plugin labels', () => {
  const english = JSON.parse(read('../src/i18n/settings-text/en-US.json')) as Record<string, string>
  for (const locale of LOCALES) {
    const catalog = JSON.parse(read(`../src/i18n/settings-text/${locale}.json`)) as Record<string, string>
    for (const key of LABEL_KEYS) {
      assert.ok(String(catalog[key] ?? '').trim(), `${locale} is missing ${key}`)
      if (locale !== 'en-US') {
        assert.notEqual(catalog[key], english[key], `${locale} leaves ${key} in English`)
      }
    }
  }
})
