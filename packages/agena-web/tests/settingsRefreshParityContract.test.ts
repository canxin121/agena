import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('the Settings sidebar refresh remounts every active server-backed workbench', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/pages/SettingsPage.vue'), 'utf8')
  assert.ok(source.includes('settingsRefreshNonce.value += 1'))
  for (const section of [
    'interface',
    'models-providers',
    'permissions',
    'plugins-tools',
    'runtime-session',
    'diagnostics',
  ]) {
    assert.ok(source.includes(`activeSection === '${section}'`), `missing active section ${section}`)
    assert.ok(source.includes(`\`${section}-\${settingsRefreshNonce}\``), `missing refresh remount key for ${section}`)
  }
})
