import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('appearance settings enumerate Agena tools and persist exact expansion overrides', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/pages/SettingsPage.vue'), 'utf8')

  assert.ok(source.includes("apiJson<ToolCatalogResponse>('/api/v1/plugins/ui')"))
  assert.ok(source.includes('response?.permission_tools'))
  assert.ok(source.includes('chatToolActivityDefaultExpandedOverrides'))
  assert.ok(source.includes('normalizeChatToolPreferenceId'))
  for (const functionName of [
    'tools_list',
    'tools_search',
    'tools_help',
    'tools_tags',
    'tools_call',
    'plugins_list',
    'plugins_search',
    'plugins_tags',
  ]) {
    assert.ok(source.includes(`id: '${functionName}'`), `missing Tool API function ${functionName}`)
  }
})
