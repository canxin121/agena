import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('plugin workbench consumes Agena status/catalog wrappers and plugin endpoints', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/components/settings/PluginsPanel.vue'), 'utf8')

  assert.ok(source.includes("apiJson<PluginStatusListResponse | PluginStatus[]>('/api/v1/plugins')"))
  assert.ok(source.includes('Array.isArray(statusData?.items)'))
  assert.ok(source.includes("apiJson<PluginUiCatalogResponse>('/api/v1/plugins/ui')"))
  assert.ok(source.includes('/commands/${encodeURIComponent(command.id)}'))
  assert.ok(source.includes('/ui/actions/${encodeURIComponent(control.id)}'))
  assert.ok(!source.includes('<iframe'))
})

test('plugin deep links reload details even when the selected id is unchanged', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/components/settings/PluginsPanel.vue'), 'utf8')
  assert.ok(source.includes('if (selectedPluginId.value === targetPluginId)'))
  assert.ok(source.includes('await loadSelectedPlugin()'))
  assert.ok(source.includes('route.query.pluginTab'))
})

test('plugin workbench defaults to a plugin with a usable Studio contribution', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/components/settings/PluginsPanel.vue'), 'utf8')
  assert.ok(source.includes('function preferredPluginId()'))
  assert.ok(source.includes('contributedIds.has(status.plugin_id)'))
  assert.ok(source.includes(': preferredPluginId()'))
})
