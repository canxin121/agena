import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('model defaults panel exposes one runtime-wide default without provider defaults', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/components/settings/ModelDefaultsPanel.vue'), 'utf8')
  assert.ok(source.includes("apiJson<ModelCatalogList>('/api/v1/model-catalog?offset=0&limit=1')"))
  assert.ok(source.includes('catalog.value?.summary?.model_count'))
  assert.ok(source.includes('catalog.value?.total'))
  assert.ok(source.includes('<ApprovalModelPanel />'))
  assert.ok(source.includes('default_selection'))
  assert.ok(source.includes('buildDefaultModelSettingsPatch'))
  assert.ok(source.includes('defaultModelKey'))
  assert.ok(source.includes('Runtime default'))
})

test('configured provider routes stay in Provider Studio instead of becoming a second settings list', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/components/settings/ModelDefaultsPanel.vue'), 'utf8')
  const providerStudio = readFileSync(resolve(import.meta.dir, '../src/components/settings/ProviderStudioPanel.vue'), 'utf8')
  const navigation = readFileSync(resolve(import.meta.dir, '../src/components/settings/settingsNavigationCatalog.ts'), 'utf8')
  assert.equal(source.includes('/configured-models'), false)
  assert.equal(source.includes('provider inventory'), false)
  assert.equal(source.includes('configured provider inventory'), false)
  assert.equal(source.includes('buildProviderDefaultSettingsPatch'), false)
  assert.equal(source.includes('provider.defaults'), false)
  assert.equal(source.includes('provider.default'), false)
  assert.ok(providerStudio.includes('/configured-models'))
  assert.equal(navigation.includes("id: 'inventory'"), false)
})
