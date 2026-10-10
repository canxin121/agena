import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'
import { PLUGIN_SETTINGS_VIEWS } from '../src/components/settings/pluginSettingsNavigation'

test('plugin workbench consumes the neutral surface and server-owned command endpoints', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/components/settings/PluginsPanel.vue'), 'utf8')

  assert.ok(source.includes("apiJson<PluginStatusListResponse | PluginStatus[]>('/api/v1/plugins')"))
  assert.ok(source.includes("apiJson<PluginSurfaceCatalogResponse>('/api/v1/plugins/surface')"))
  assert.ok(source.includes('/commands/${encodeURIComponent(command.id)}/invoke'))
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
  assert.ok(!source.includes('/ui/actions/'))
  assert.ok(!source.includes('<iframe'))
})

test('plugin contents are selected through fixed settings sidebar destinations', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/components/settings/PluginsPanel.vue'), 'utf8')
  assert.deepEqual(Object.values(PLUGIN_SETTINGS_VIEWS), [
    'overview',
    'settings',
    'commands',
    'tools',
    'logs',
    'diagnostics',
  ])
  assert.ok(source.includes('props.tab'))
  assert.ok(!source.includes('role="tablist"'))
  assert.ok(!source.includes('selectTab('))
})

test('plugin workbench prefers a plugin with a declared command', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/components/settings/PluginsPanel.vue'), 'utf8')
  assert.ok(source.includes('function preferredPluginId()'))
  assert.ok(source.includes('contributedIds.has(status.plugin_id)'))
})
