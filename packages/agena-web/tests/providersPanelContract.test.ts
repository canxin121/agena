import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('providers panel counts the paginated model catalog and reads runtime defaults', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/components/settings/ProvidersPanel.vue'), 'utf8')
  assert.ok(source.includes("apiJson<ModelCatalogList>('/api/v1/model-catalog?offset=0&limit=1')"))
  assert.ok(source.includes('catalog.value?.summary?.model_count'))
  assert.ok(source.includes('catalog.value?.total'))
  assert.ok(source.includes('runtime.value?.default_selection'))
})

test('providers panel verifies the persisted default after settings reload', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/components/settings/ProvidersPanel.vue'), 'utf8')
  assert.ok(source.includes("method: 'PATCH'"))
  assert.ok(source.includes('buildProviderDefaultSettingsPatch'))
  assert.ok(source.includes('await refresh()'))
  assert.ok(source.includes('The server accepted the update but did not apply the selected default.'))
})
