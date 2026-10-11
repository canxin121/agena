import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('advanced settings editor targets explicit layers and validates before writes', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/components/settings/AdvancedSettingsPanel.vue'), 'utf8')
  assert.ok(source.includes('readRuntimeSettingSources'))
  assert.ok(source.includes('{ dry_run: true, validate: true, reload: false }'))
  assert.ok(source.includes('{ validate: true, reload: true }'))
  assert.ok(source.includes('targetLayer.value'))
  assert.ok(source.includes('deleteRuntimeSetting'))
  assert.ok(source.includes('Copy effective'))
  // The layer is chosen in the shared left navigation, so the editor follows the
  // forwarded scope and never renders its own layer picker.
  assert.ok(source.includes("props.scope === 'workspace' ? 'workspace' : 'global'"))
  assert.equal(source.includes(':options="layerOptions"'), false)
  assert.equal(source.includes('v-model="targetLayer"'), false)
})

test('diagnostics workbench exposes advanced settings as a real subpage', () => {
  const source = readFileSync(
    resolve(import.meta.dir, '../src/components/settings/DiagnosticsWorkbenchPanel.vue'),
    'utf8',
  )
  const catalog = readFileSync(
    resolve(import.meta.dir, '../src/components/settings/settingsNavigationCatalog.ts'),
    'utf8',
  )
  assert.ok(source.includes("buildSettingsSubpages('diagnostics')"))
  assert.ok(catalog.includes("id: 'advanced-settings'"))
  assert.ok(source.includes('<AdvancedSettingsPanel'))
  assert.ok(
    source.includes('<AdvancedSettingsPanel v-else-if="activePage === \'advanced-settings\'" :scope="scope" />'),
  )
  assert.ok(source.includes('defineProps<{ activePage: string; scope: string }>()'))
})
