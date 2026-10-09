import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('conversation settings provide activity-kind and searchable exact-tool expansion overrides', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/pages/SettingsPage.vue'), 'utf8')
  assert.ok(source.includes('activityKindOptions'))
  assert.ok(source.includes('filteredToolActivityOptions'))
  assert.ok(source.includes('toolCatalogQuery'))
  assert.ok(source.includes('settings.appearance.chat.searchTools'))
  assert.ok(source.includes('v-for="opt in filteredToolActivityOptions"'))
  assert.ok(source.includes('transcriptPreferences.setActivityKindExpanded'))
  assert.ok(source.includes('transcriptPreferences.setToolExpanded'))
})
