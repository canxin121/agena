import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

import {
  BUILTIN_CHAT_ACTIVITY_KINDS,
  DEFAULT_CHAT_ACTIVITY_EXPANDED_TOOL_FILTERS,
  DEFAULT_CHAT_ACTIVITY_KIND_EXPANDED,
  DEFAULT_CHAT_TOOL_ACTIVITY_FILTERS,
  chatActivityKindIdForTranscriptPart,
  migrateLegacyChatActivityDefaultExpanded,
  normalizeChatActivityKindCatalog,
  normalizeChatActivityKindDefaultExpanded,
  normalizeChatToolActivityId,
  normalizeChatToolExpansionOverrides,
  normalizeChatToolPreferenceId,
  resolveChatActivityKindDefaultExpanded,
  resolveChatToolDefaultExpanded,
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

test('web fallback activity kinds mirror the server catalog instead of legacy OpenCode parts', () => {
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
  for (const retired of ['snapshot', 'patch', 'retry', 'justification', 'step-start', 'step-finish']) {
    assert.equal(BUILTIN_KIND_IDS.includes(retired), false)
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

test('part expansion defaults migrate old keys without treating them as Agena kinds', () => {
  assert.deepEqual(DEFAULT_CHAT_ACTIVITY_KIND_EXPANDED, ['reasoning'])
  assert.deepEqual(normalizeChatActivityKindDefaultExpanded([' operation ', 'reasoning', 'OPERATION']), [
    'operation',
    'reasoning',
    'OPERATION',
  ])
  assert.deepEqual(
    migrateLegacyChatActivityDefaultExpanded(['snapshot', 'patch', 'retry', 'thinking', 'compaction', 'justification']),
    ['reasoning', 'notice'],
  )
  assert.deepEqual(resolveChatActivityKindDefaultExpanded(null), ['reasoning'])
  assert.deepEqual(resolveChatActivityKindDefaultExpanded({ chatActivityDefaultExpanded: [] }), ['reasoning'])
  assert.deepEqual(resolveChatActivityKindDefaultExpanded({ chatActivityKindDefaultExpanded: [] }), [])
  assert.deepEqual(
    resolveChatActivityKindDefaultExpanded({ chatActivityKindDefaultExpanded: ['operation', 'example.trace'] }),
    ['operation', 'example.trace'],
  )
})

test('transcript presentation kinds resolve to server activity kind ids', () => {
  assert.equal(chatActivityKindIdForTranscriptPart('reasoning', 'think'), 'reasoning')
  assert.equal(chatActivityKindIdForTranscriptPart('operation', 'tool_call'), 'operation')
  assert.equal(chatActivityKindIdForTranscriptPart('resource', 'file_ref'), 'resource')
  assert.equal(chatActivityKindIdForTranscriptPart('skill', 'skill_ref'), 'skill_reference')
  assert.equal(chatActivityKindIdForTranscriptPart('text_segment', 'text'), 'text')
  assert.equal(chatActivityKindIdForTranscriptPart('notice', 'hook'), 'hook')
  assert.equal(chatActivityKindIdForTranscriptPart('notice', 'system_notification'), 'notice')
  assert.equal(chatActivityKindIdForTranscriptPart('compaction', 'compaction'), 'notice')
  assert.equal(chatActivityKindIdForTranscriptPart('answer', 'text'), '')
  assert.equal(chatActivityKindIdForTranscriptPart('unknown', 'Example.Trace'), 'Example.Trace')
})

test('default expansion opens editing tools and otherwise inherits operation', () => {
  assert.deepEqual(DEFAULT_CHAT_ACTIVITY_EXPANDED_TOOL_FILTERS, ['edit', 'write', 'apply_patch', 'multiedit'])
  for (const tool of DEFAULT_CHAT_ACTIVITY_EXPANDED_TOOL_FILTERS) {
    assert.equal(DEFAULT_CHAT_TOOL_ACTIVITY_FILTERS.includes(tool), true)
  }

  const legacy = new Set<string>(DEFAULT_CHAT_ACTIVITY_EXPANDED_TOOL_FILTERS)
  assert.equal(resolveChatToolDefaultExpanded('fs.replace', {}, legacy, false), true)
  assert.equal(resolveChatToolDefaultExpanded('fs.glob', {}, legacy, false), false)
  assert.equal(resolveChatToolDefaultExpanded('fs.glob', {}, legacy, true), true)
  assert.equal(resolveChatToolDefaultExpanded('fs.glob', { 'fs.glob': false }, legacy, true), false)
})

test('Agena namespaced tools map to categories while exact preferences stay distinct', () => {
  assert.equal(normalizeChatToolActivityId('fs.read'), 'read')
  assert.equal(normalizeChatToolActivityId('fs.replace'), 'edit')
  assert.equal(normalizeChatToolActivityId('fs.apply_patch'), 'apply_patch')
  assert.equal(normalizeChatToolActivityId('shell.run'), 'bash')
  assert.equal(normalizeChatToolActivityId('web.search'), 'websearch')
  assert.equal(normalizeChatToolActivityId('custom.plugin_tool'), 'custom.plugin_tool')

  assert.equal(normalizeChatToolPreferenceId('agena.fs.read'), 'fs.read')
  assert.equal(normalizeChatToolPreferenceId('fs.read_many'), 'fs.read_many')
  assert.deepEqual(
    normalizeChatToolExpansionOverrides({
      'agena.fs.read': true,
      'fs.read_many': false,
      'agena.shell.run': 'invalid',
    }),
    { 'fs.read': true, 'fs.read_many': false },
  )
})

test('settings page consumes the server activity catalog and has no legacy summary matrix', () => {
  const settingsPage = readFileSync(resolve(import.meta.dir, '../src/pages/SettingsPage.vue'), 'utf8')
  assert.ok(settingsPage.includes('response?.activity_kinds'))
  assert.ok(settingsPage.includes('v-for="opt in activityKindOptions"'))
  assert.ok(settingsPage.includes('chatActivityKindDefaultExpanded'))
  assert.ok(!settingsPage.includes('activityTable.summary'))
  assert.ok(!settingsPage.includes('activityDefaultExpandedOptions'))
})
