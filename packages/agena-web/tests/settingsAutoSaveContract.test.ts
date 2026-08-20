import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

const serverField = readFileSync(resolve(import.meta.dir, '../src/components/settings/ServerSettingField.vue'), 'utf8')
const mcpPanel = readFileSync(resolve(import.meta.dir, '../src/components/settings/McpServerControlPanel.vue'), 'utf8')
const saveBar = readFileSync(resolve(import.meta.dir, '../src/components/settings/SettingsSaveBar.vue'), 'utf8')

test('simple server settings auto-save instead of rendering one Save button per field', () => {
  assert.ok(serverField.includes('scheduleAutoSave'))
  assert.ok(serverField.includes('flushAutoSave'))
  assert.ok(serverField.includes('Saved automatically'))
  assert.ok(serverField.includes('Waiting to save'))
  assert.ok(!serverField.includes('RiSave3Line'))
  assert.ok(!serverField.includes('@click="save"'))
})

test('MCP regular configuration has one dirty-aware page save action', () => {
  assert.ok(mcpPanel.includes('const controlDirty = computed'))
  assert.ok(mcpPanel.includes('<SettingsSaveBar'))
  assert.ok(mcpPanel.includes('@save="saveControlDraft"'))
  assert.ok(mcpPanel.includes('@discard="discardControlDraft"'))
  for (const oldHandler of [
    'savePublicUrl',
    'saveOAuthIssuerUrl',
    'saveAuthMode',
    'saveAnonymousAccess',
    'saveClientRegistration',
  ]) {
    assert.ok(!mcpPanel.includes(oldHandler), `obsolete field-level save handler remains: ${oldHandler}`)
  }
  assert.ok(mcpPanel.includes('setPassword'))
  assert.ok(mcpPanel.includes('clearPassword'))
})

test('shared save bar exposes dirty, saving, saved, save, and discard states', () => {
  assert.ok(saveBar.includes('You have unsaved changes.'))
  assert.ok(saveBar.includes('All changes are saved.'))
  assert.ok(saveBar.includes("(event: 'save')"))
  assert.ok(saveBar.includes("(event: 'discard')"))
})
