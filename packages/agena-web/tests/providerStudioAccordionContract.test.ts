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
  assert.ok(providerSource.includes('@click.stop="deleteModel(adapter.adapter_id, model.id)"'))
})

test('provider studio uses one dirty-aware save boundary for provider, adapter, and model edits', () => {
  assert.ok(providerSource.includes('<SettingsSaveBar'))
  assert.ok(providerSource.includes(':dirty="providerDirty"'))
  assert.ok(providerSource.includes('const editorDirty = ref(false)'))
  assert.ok(
    !providerSource.includes('(savedEditorState.value && providerEditorStateFingerprint() !== savedEditorState.value)'),
  )
  assert.ok(providerSource.includes('pendingDeletedAdapterIds'))
  assert.ok(providerSource.includes('pendingDeletedModelKeys'))
  assert.ok(providerSource.includes('stageCurrentModelValue'))
  assert.ok(!providerSource.includes('Save adapter'))
  assert.ok(!providerSource.includes('Save model config'))
  assert.ok(!providerSource.includes('saveAdapter('))
  assert.ok(!providerSource.includes('saveModel('))
})

test('model edit action stops propagation from the adapter disclosure row', () => {
  assert.ok(providerSource.includes('@click.stop="openModelEditor(adapter.adapter_id, model)"'))
})

test('provider studio uses the Input component model-value contract for editable fields', () => {
  const inputTags = [...providerSource.matchAll(/<Input\b[\s\S]*?\/>/g)].map((match) => match[0])

  assert.ok(inputTags.length >= 6)
  assert.ok(inputTags.every((tag) => !tag.includes(':value=')))
  assert.ok(inputTags.every((tag) => !tag.includes('@input=')))
  assert.ok(inputTags.some((tag) => tag.includes(':model-value="draft.provider_id"')))
  assert.ok(inputTags.some((tag) => tag.includes('@update:model-value="setFieldValue(field.path, $event)"')))
  assert.ok(providerSource.includes('function cloneModelPath'))
  assert.ok(providerSource.includes('function scheduleModelJsonSync'))
  assert.ok(providerSource.includes('const modelJsonDirty = ref(false)'))
  assert.ok(providerSource.includes('@input="markModelJsonDirty"'))
})

test('model editor is rendered inside the matching model row', () => {
  const modelLoop = providerSource.indexOf('v-for="model in adapter.models"')
  const editor = providerSource.indexOf(
    'v-if="editingModel?.adapterId === adapter.adapter_id && editingModel?.modelId === model.id"',
  )
  const globalEditor = providerSource.indexOf('<section v-if="editingModel"')

  assert.ok(modelLoop >= 0)
  assert.ok(editor > modelLoop)
  assert.equal(globalEditor, -1)
})
