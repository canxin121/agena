import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('providers panel exposes one runtime-wide default without provider defaults', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/components/settings/ProvidersPanel.vue'), 'utf8')
  assert.ok(source.includes("apiJson<ModelCatalogList>('/api/v1/model-catalog?offset=0&limit=1')"))
  assert.ok(source.includes('catalog.value?.summary?.model_count'))
  assert.ok(source.includes('catalog.value?.total'))
  assert.ok(source.includes('<ApprovalModelPanel />'))
  assert.ok(source.includes('default_selection'))
  assert.ok(source.includes('buildDefaultModelSettingsPatch'))
  assert.ok(source.includes('defaultModelKey'))
  assert.ok(source.includes('Runtime default'))
})

test('providers panel only renders configured provider inventory', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/components/settings/ProvidersPanel.vue'), 'utf8')
  assert.ok(source.includes('/configured-models'))
  assert.equal(source.includes('buildProviderDefaultSettingsPatch'), false)
  assert.equal(source.includes('provider.defaults'), false)
  assert.equal(source.includes('provider.default'), false)
})
