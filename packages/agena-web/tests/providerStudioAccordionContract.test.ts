import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

const providerSource = readFileSync(
  resolve(import.meta.dir, '../src/components/settings/ProviderStudioPanel.vue'),
  'utf8',
)

test('provider studio renders providers and adapters as nested disclosure rows', () => {
  assert.ok(providerSource.includes('v-for="row in providerRows"'))
  assert.ok(providerSource.includes('v-for="adapter in adapterRows"'))
  assert.ok(providerSource.includes('<SettingsDisclosureRow'))
  assert.ok(providerSource.includes('expandedProviderKey'))
  assert.ok(providerSource.includes('expandedAdapterIds'))
  assert.ok(providerSource.includes('NEW_PROVIDER_ROW_KEY'))
})

test('adapter row headers own enable and destructive actions', () => {
  assert.ok(providerSource.includes(':checked="selectedAdapterIds.has(adapter.adapter_id)"'))
  assert.ok(providerSource.includes('@change="toggleAdapter(adapter.adapter_id)"'))
  assert.ok(providerSource.includes('@click="deleteAdapter(adapter.adapter_id)"'))
  assert.ok(providerSource.includes('@click="deleteProviderRow(row)"'))
  assert.ok(providerSource.includes('@click="deleteModel(adapter.adapter_id, model.id)"'))
})

test('provider studio uses one dirty-aware save boundary for provider, adapter, and model edits', () => {
  assert.ok(providerSource.includes('<SettingsSaveBar'))
  assert.ok(providerSource.includes(':dirty="providerDirty"'))
  assert.ok(providerSource.includes('pendingDeletedAdapterIds'))
  assert.ok(providerSource.includes('pendingDeletedModelKeys'))
  assert.ok(providerSource.includes('stageCurrentModelValue'))
  assert.ok(!providerSource.includes('Save adapter'))
  assert.ok(!providerSource.includes('Save model config'))
  assert.ok(!providerSource.includes('saveAdapter('))
  assert.ok(!providerSource.includes('saveModel('))
})
