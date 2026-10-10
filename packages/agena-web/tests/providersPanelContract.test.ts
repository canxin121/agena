import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('model defaults panel edits one layer-scoped default without model catalog statistics', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/components/settings/ModelDefaultsPanel.vue'), 'utf8')
  assert.equal(source.includes('/api/v1/model-catalog'), false)
  assert.equal(source.includes('model_count'), false)
  assert.equal(source.includes('Model Catalog'), false)
  // The page reads the effective, global, and workspace layers of one path and
  // writes only the layer the user selected.
  assert.ok(source.includes('readRuntimeSettingSources(DEFAULT_SELECTION_PATH)'))
  assert.ok(source.includes('setRuntimeSetting(DEFAULT_SELECTION_PATH'))
  assert.ok(source.includes('deleteRuntimeSetting(DEFAULT_SELECTION_PATH'))
  assert.ok(source.includes("'Effective default'"))
  assert.ok(source.includes("'Global default'"))
  assert.ok(source.includes("'Workspace default'"))
  assert.ok(!source.includes('<ApprovalModelPanel'))
  const approval = readFileSync(
    resolve(import.meta.dir, '../src/components/settings/ApprovalSettingsPanel.vue'),
    'utf8',
  )
  assert.ok(approval.includes('<ApprovalModelPanel :scope="scope" />'))
  assert.ok(source.includes('Save default model'))
})

test('the approval model editor follows the page scope and writes that layer', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/components/settings/ApprovalModelPanel.vue'), 'utf8')
  assert.ok(source.includes("scope?: 'effective' | 'global' | 'workspace'"))
  assert.ok(source.includes('readRuntimeSettingSources(APPROVAL_MODEL_PATH)'))
  assert.ok(source.includes('setRuntimeSetting(APPROVAL_MODEL_PATH'))
  assert.ok(source.includes('deleteRuntimeSetting(APPROVAL_MODEL_PATH'))
  assert.equal(source.includes('/api/v1/settings?source=effective'), false)
})

test('configured provider routes stay in Provider Studio instead of becoming a second settings list', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/components/settings/ModelDefaultsPanel.vue'), 'utf8')
  const providerStudio = readFileSync(
    resolve(import.meta.dir, '../src/components/settings/ProviderStudioPanel.vue'),
    'utf8',
  )
  const navigation = readFileSync(
    resolve(import.meta.dir, '../src/components/settings/settingsNavigationCatalog.ts'),
    'utf8',
  )
  assert.equal(source.includes('/configured-models'), false)
  assert.equal(source.includes('provider inventory'), false)
  assert.equal(source.includes('configured provider inventory'), false)
  assert.equal(source.includes('buildProviderDefaultSettingsPatch'), false)
  assert.equal(source.includes('provider.defaults'), false)
  assert.equal(source.includes('provider.default'), false)
  assert.ok(providerStudio.includes('/configured-models'))
  assert.equal(navigation.includes("id: 'inventory'"), false)
})
