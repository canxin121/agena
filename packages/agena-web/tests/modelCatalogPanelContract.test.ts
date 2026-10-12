import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

const source = readFileSync(resolve(import.meta.dir, '../src/components/settings/ModelCatalogPanel.vue'), 'utf8')
const providerStudioSource = readFileSync(
  resolve(import.meta.dir, '../src/components/settings/ProviderStudioPanel.vue'),
  'utf8',
)
const defaultModelSource = readFileSync(
  resolve(import.meta.dir, '../src/components/settings/ModelDefaultsPanel.vue'),
  'utf8',
)
const approvalModelSource = readFileSync(
  resolve(import.meta.dir, '../src/components/settings/ApprovalModelPanel.vue'),
  'utf8',
)

test('Model Catalog workbench supports server search, origin filtering, paging, refresh, and full details', () => {
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

test('Model Catalog fills the settings content area instead of scrolling inside a short pane', () => {
  const navigation = readFileSync(
    resolve(import.meta.dir, '../src/components/settings/settingsNavigationCatalog.ts'),
    'utf8',
  )
  const workbench = readFileSync(
    resolve(import.meta.dir, '../src/components/settings/workbench/SettingsSectionWorkbench.vue'),
    'utf8',
  )
  const page = readFileSync(resolve(import.meta.dir, '../src/pages/SettingsPage.vue'), 'utf8')

  // The subpage declares the fill layout; SettingsPage and the section workbench
  // then hand it the viewport height and drop the centered width cap, so the
  // list and the detail pane scroll internally at full height.
  assert.ok(navigation.includes('fillsViewport: true'))
  assert.ok(workbench.includes('activeDefinition.value?.fillsViewport === true'))
  assert.ok(page.includes('contentFillsViewport'))
  assert.ok(source.includes('lg:grid-rows-[minmax(0,1fr)]'))
  assert.ok(source.includes('lg:min-h-0 lg:max-h-none lg:flex-1'))
  assert.ok(source.includes('lg:min-h-0 lg:p-5'))
  assert.equal(source.includes('min-h-[38rem]'), false)
  assert.equal(source.includes('lg:max-h-[47rem]'), false)
})

test('parallel tool support is edited as model metadata, never as a default/approval selection option', () => {
  assert.ok(providerStudioSource.includes("key: 'supports_parallel_tool_calls'"))
  assert.equal(defaultModelSource.includes('Parallel tool calls'), false)
  assert.equal(approvalModelSource.includes('Parallel tool calls'), false)
  assert.equal(defaultModelSource.includes('parallel_tool_calls'), false)
  assert.equal(approvalModelSource.includes('parallel_tool_calls'), false)
})
