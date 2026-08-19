import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('Model Catalog workbench supports server search, origin filtering, paging, refresh, and full details', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/components/settings/ModelCatalogPanel.vue'), 'utf8')
  assert.ok(source.includes("params.set('q'"))
  assert.ok(source.includes("params.set('origin'"))
  assert.ok(source.includes('offset: String(offset.value)'))
  assert.ok(source.includes('limit: String(PAGE_SIZE)'))
  assert.ok(source.includes("'/api/v1/model-catalog/refresh'"))
  for (const field of [
    'context_window_tokens',
    'max_input_tokens',
    'max_output_tokens',
    'thinking_modes',
    'speed_modes',
    'pricing',
    'supports_parallel_tool_calls',
    'supports_verbosity',
  ]) {
    assert.ok(source.includes(field), `missing Model Catalog field ${field}`)
  }
})
