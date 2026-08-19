import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('Plugin Workbench exposes the unified Settings/Operations/Tools architecture surface', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/components/settings/PluginsPanel.vue'), 'utf8')
  for (const tab of ['overview', 'settings', 'operations', 'tools', 'logs', 'diagnostics']) {
    assert.ok(source.includes(`id: '${tab}'`), `missing plugin tab ${tab}`)
  }
  assert.ok(source.includes('<PluginContractEditor'))
  assert.ok(source.includes("apiJson<PluginArchitectureCatalog>('/api/v1/plugins/architecture')"))
  assert.ok(source.includes('/operations/${encodeURIComponent(operation.id)}/invoke'))
  assert.ok(source.includes('selectedToolRegistrations'))
  assert.ok(source.includes('Scoped tool registrations'))
  assert.ok(source.includes('filteredStatuses'))
  assert.ok(source.includes('<SearchInput'))
  assert.ok(source.includes('<OptionPicker'))
  assert.ok(!source.includes('/commands/'))
  assert.ok(!source.includes('/ui/actions/'))
  assert.ok(!source.includes("id: 'views'"))
  assert.ok(!source.includes("id: 'controls'"))
})
