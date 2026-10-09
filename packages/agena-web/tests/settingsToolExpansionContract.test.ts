import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('appearance settings enumerate Agena tools and persist exact expansion overrides', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/pages/SettingsPage.vue'), 'utf8')

  assert.ok(source.includes("apiJson<ToolCatalogResponse>('/api/v1/plugins/surface',"))
  assert.ok(source.includes('response?.permission_tools'))
  assert.ok(source.includes('transcriptPreferences.setToolExpanded'))
  assert.ok(source.includes('normalizeChatToolPreferenceId'))
  const preferences = readFileSync(resolve(import.meta.dir, '../src/stores/transcriptPreferences.ts'), 'utf8')
  assert.ok(preferences.includes('ui.transcript'))
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

test('Web and TUI transcript expansion use one shared runtime preference document', () => {
  const root = resolve(import.meta.dir, '../src')
  const preferences = readFileSync(resolve(root, 'stores/transcriptPreferences.ts'), 'utf8')
  const settings = readFileSync(resolve(root, 'stores/settings.ts'), 'utf8')
  const conversationSettings = readFileSync(resolve(root, 'pages/SettingsPage.vue'), 'utf8')
  const tuiSettings = readFileSync(resolve(root, 'components/settings/InterfaceSettingsPanel.vue'), 'utf8')
  const settingsType = settings.slice(0, settings.indexOf('const STORAGE_KEY'))
  const tuiProjection = readFileSync(
    resolve(import.meta.dir, '../../../crates/agena-tui-app/src/app_backend/config.rs'),
    'utf8',
  )

  assert.equal((preferences.match(/getRuntimeSetting\('ui\.transcript'\)/g) || []).length, 1)
  assert.ok(preferences.includes("setRuntimeSetting('ui.transcript'"))
  assert.doesNotMatch(preferences, /ui\.tui\.transcript|parseLegacy|useSettingsStore/)
  assert.doesNotMatch(settingsType, /chatActivityKindDefaultExpanded|chatToolActivityDefaultExpanded/)
  assert.ok(settings.includes('removedTranscriptPreferences'), 'old browser copies are deleted during hydration')
  assert.ok(conversationSettings.includes('transcriptPreferences.setActivityDefaultExpanded'))
  assert.ok(conversationSettings.includes('transcriptPreferences.setToolExpanded'))
  assert.doesNotMatch(tuiSettings, /ui\.transcript|activityDefaultExpanded|toolOverrides/)
  assert.ok(tuiProjection.includes('ui.get("transcript")'))
  assert.doesNotMatch(tuiProjection, /legacy_transcript|tui\.get\("transcript"\)/)
})
