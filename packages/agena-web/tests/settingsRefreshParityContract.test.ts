import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('the Settings sidebar refreshes active workbenches and preserves drafts across every settings domain', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/pages/SettingsPage.vue'), 'utf8')
  assert.equal(source.includes('settingsRefreshNonce'), false, 'refresh must not remount editors')
  assert.ok(source.includes('await pluginsToolsPanel.value?.refresh()'))
  assert.ok(source.includes('await permissionsPanel.value?.refresh()'))
  assert.ok(source.includes('await modelsProvidersPanel.value?.refresh()'))
  for (const panel of ['runtimeSessionPanel', 'diagnosticsPanel', 'interfacePanel']) {
    assert.ok(source.includes(`await ${panel}.value?.refresh()`), `${panel} must refresh in place`)
  }
  assert.ok(source.includes('await loadChatToolCatalog()'))
  for (const section of [
    'interface',
    'models-providers',
    'permissions',
    'plugins-tools',
    'runtime-session',
    'diagnostics',
  ]) {
    assert.ok(source.includes(`activeSection === '${section}'`), `missing active section ${section}`)
  }
})
