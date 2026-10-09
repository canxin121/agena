import assert from 'node:assert/strict'
import { existsSync, readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('tool harness configuration is absent from settings navigation and editors', () => {
  const settingsRoot = resolve(import.meta.dir, '../src/components/settings')
  const navigation = readFileSync(resolve(settingsRoot, 'settingsNavigationCatalog.ts'), 'utf8')
  const advanced = readFileSync(resolve(settingsRoot, 'AdvancedSettingsPanel.vue'), 'utf8')
  const plugins = readFileSync(resolve(settingsRoot, 'PluginsToolsPanel.vue'), 'utf8')
  const tuiSettings = readFileSync(
    resolve(import.meta.dir, '../../../crates/agena-tui-app/src/app_session_interactive/settings.rs'),
    'utf8',
  )
  assert.equal(existsSync(resolve(settingsRoot, 'HarnessSettingsPanel.vue')), false)
  assert.equal(navigation.includes('harnesses'), false)
  assert.equal(advanced.includes("value: 'harnesses'"), false)
  assert.equal(plugins.toLowerCase().includes('harness'), false)
  assert.equal(tuiSettings.includes('settings_studio_harness_items'), false)
})
