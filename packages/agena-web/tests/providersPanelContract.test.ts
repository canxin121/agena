import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('default and approval pages share a scope-aware model selection editor', () => {
  const read = (file: string) =>
    readFileSync(resolve(import.meta.dir, `../src/components/settings/${file}.vue`), 'utf8')
  const editor = read('ModelSelectionSettings')
  assert.ok(read('ModelDefaultsPanel').includes('purpose="default"'))
  assert.ok(read('ApprovalModelPanel').includes('purpose="approval"'))
  assert.ok(editor.includes("'providers.default_selection'"))
  assert.ok(editor.includes("'permission.approval_model'"))
  assert.ok(editor.includes('readRuntimeSettingSources(path.value'))
  assert.ok(editor.includes('setRuntimeSetting(path.value, value, { reload: true }, layer)'))
  assert.ok(editor.includes('deleteRuntimeSetting(path.value, { reload: true }, layer)'))
  assert.ok(editor.includes('onBeforeRouteLeave(canLeave)'))
  assert.ok(editor.includes('if (!dirty.value) syncEditor()'))
  assert.ok(editor.includes('Save default model'))
  assert.ok(editor.includes('Save approval model'))
  assert.equal(editor.includes('/api/v1/model-catalog'), false)
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
