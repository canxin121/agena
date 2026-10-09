import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

import {
  BUILTIN_CHAT_ACTIVITY_KINDS,
  chatActivityKindIdForTranscriptPart,
  normalizeChatActivityKindCatalog,
  normalizeChatToolActivityId,
  normalizeChatToolPreferenceId,
} from '../src/lib/chatActivity'

const BUILTIN_KIND_IDS = [
  'reasoning',
  'operation',
  'resource',
  'skill_reference',
  'interaction',
  'hook',
  'error',
  'notice',
  'text',
]

test('web fallback activity kinds mirror the current server catalog', () => {
  assert.deepEqual(
    BUILTIN_CHAT_ACTIVITY_KINDS.map((item) => item.id),
    BUILTIN_KIND_IDS,
  )

  const serverCatalog = readFileSync(
    resolve(import.meta.dir, '../../../crates/agena-domain/src/activity_kind.rs'),
    'utf8',
  )
  for (const id of BUILTIN_KIND_IDS) {
    assert.ok(serverCatalog.includes(`= "${id}"`), `server activity catalog is missing ${id}`)
  }
})

test('server activity catalog normalization retains plugin-contributed kinds', () => {
  assert.deepEqual(
    normalizeChatActivityKindCatalog([
      { id: ' reasoning ', category: 'builtin', label: 'Reasoning' },
      { id: 'example.trace', category: 'plugin', label: 'Trace' },
      { id: 'Example.Trace', category: 'plugin', label: 'Case-sensitive trace' },
      { id: 'example.trace', category: 'plugin', label: 'Duplicate' },
      { category: 'plugin', label: 'Missing id' },
    ]),
    [
      { id: 'reasoning', category: 'builtin', label: 'Reasoning' },
      { id: 'example.trace', category: 'plugin', label: 'Trace' },
      { id: 'Example.Trace', category: 'plugin', label: 'Case-sensitive trace' },
    ],
  )
})

test('transcript presentation kinds resolve to server activity kind ids', () => {
  assert.equal(chatActivityKindIdForTranscriptPart('reasoning', 'think'), 'reasoning')
  assert.equal(chatActivityKindIdForTranscriptPart('operation', 'tool_call'), 'operation')
  assert.equal(chatActivityKindIdForTranscriptPart('resource', 'file_ref'), 'resource')
  assert.equal(chatActivityKindIdForTranscriptPart('command', 'skill_ref'), 'skill_reference')
  assert.equal(chatActivityKindIdForTranscriptPart('text_segment', 'text'), 'text')
  assert.equal(chatActivityKindIdForTranscriptPart('notice', 'hook'), 'hook')
  assert.equal(chatActivityKindIdForTranscriptPart('notice', 'system_notification'), 'notice')
  assert.equal(chatActivityKindIdForTranscriptPart('compaction', 'compaction'), 'notice')
  assert.equal(chatActivityKindIdForTranscriptPart('answer', 'text'), '')
  assert.equal(chatActivityKindIdForTranscriptPart('unknown', 'Example.Trace'), 'Example.Trace')
})

test('Agena namespaced tools map to categories while exact preferences stay distinct', () => {
  assert.equal(normalizeChatToolActivityId('fs.read'), 'read')
  assert.equal(normalizeChatToolActivityId('fs.replace'), 'edit')
  assert.equal(normalizeChatToolActivityId('fs.apply_patch'), 'apply_patch')
  assert.equal(normalizeChatToolActivityId('shell.run'), 'bash')
  assert.equal(normalizeChatToolActivityId('web.search'), 'websearch')
  assert.equal(normalizeChatToolActivityId('chatgpt.cloud_shell'), 'bash')
  assert.equal(normalizeChatToolActivityId('claude.cloud_code_execution'), 'bash')
  assert.equal(normalizeChatToolActivityId('gemini.cloud_url_context'), 'webfetch')
  assert.equal(normalizeChatToolActivityId('gemini.cloud_google_search'), 'websearch')
  assert.equal(normalizeChatToolActivityId('chatgpt.cloud_file_search'), 'codesearch')
  assert.equal(normalizeChatToolActivityId('custom.plugin_tool'), 'custom.plugin_tool')

  assert.equal(normalizeChatToolPreferenceId('agena.fs.read'), 'fs.read')
  assert.equal(normalizeChatToolPreferenceId('fs.read_many'), 'fs.read_many')
})

test('settings page consumes the current server activity catalog', () => {
  const settingsPage = readFileSync(resolve(import.meta.dir, '../src/pages/SettingsPage.vue'), 'utf8')
  const preferencesStore = readFileSync(resolve(import.meta.dir, '../src/stores/transcriptPreferences.ts'), 'utf8')
  assert.ok(settingsPage.includes('response?.activity_kinds'))
  assert.ok(settingsPage.includes('v-for="opt in activityKindOptions"'))
  assert.ok(settingsPage.includes('transcriptPreferences.setActivityKindExpanded'))
  assert.ok(preferencesStore.includes("setRuntimeSetting('ui.transcript'"))
  assert.ok(preferencesStore.includes("getRuntimeSetting('ui.transcript')"))
  assert.ok(!settingsPage.includes('activityTable.summary'))
  assert.ok(!settingsPage.includes('activityDefaultExpandedOptions'))
})
