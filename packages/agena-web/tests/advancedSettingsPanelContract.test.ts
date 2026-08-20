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
})
