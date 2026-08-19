import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('plugin workbench consumes the neutral surface and server-owned operation endpoints', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/components/settings/PluginsPanel.vue'), 'utf8')

  assert.ok(source.includes("apiJson<PluginStatusListResponse | PluginStatus[]>('/api/v1/plugins')"))
  assert.ok(source.includes("apiJson<PluginSurfaceCatalogResponse>('/api/v1/plugins/surface')"))
  assert.ok(source.includes('/operations/${encodeURIComponent(operation.id)}/invoke'))
  assert.ok(source.includes('/settings`'))
  assert.ok(source.includes('<PluginContractEditor'))
  assert.ok(source.includes("apiJson<PluginArchitectureCatalog>('/api/v1/plugins/architecture')"))
  assert.ok(source.includes('selectedBlocked'))
  assert.ok(source.includes('selectedReloadDecision'))
  assert.ok(source.includes('selectedProfileChanges'))
  assert.ok(source.includes('Applied plugin profiles'))
  assert.ok(source.includes('selectedIncomingDependencies'))
  assert.ok(source.includes('selectedEffectLifecycle'))
  assert.ok(source.includes('selectedToolRegistrations'))
  assert.ok(source.includes('Scoped tool registrations'))
  assert.ok(!source.includes('/commands/'))
  assert.ok(!source.includes('/ui/actions/'))
  assert.ok(!source.includes('<iframe'))
})

test('plugin deep links use fixed host-owned workbench tabs', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/components/settings/PluginsPanel.vue'), 'utf8')
  for (const tab of ['overview', 'settings', 'operations', 'tools', 'logs', 'diagnostics']) {
    assert.ok(source.includes(`id: '${tab}'`), `missing fixed plugin tab ${tab}`)
  }
  assert.ok(source.includes('route.query.pluginTab'))
  assert.ok(!source.includes("id: 'views'"))
  assert.ok(!source.includes("id: 'controls'"))
})

test('plugin workbench prefers a plugin with a declared operation', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/components/settings/PluginsPanel.vue'), 'utf8')
  assert.ok(source.includes('function preferredPluginId()'))
  assert.ok(source.includes('contributedIds.has(status.plugin_id)'))
})
