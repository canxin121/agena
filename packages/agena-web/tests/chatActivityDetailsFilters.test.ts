import assert from 'node:assert/strict'
import test from 'node:test'

import {
  ACTIVITY_DEFAULT_EXPANDED_OPTIONS,
  CHAT_ACTIVITY_EXPAND_KEYS,
  DEFAULT_CHAT_ACTIVITY_FILTERS,
  DEFAULT_CHAT_ACTIVITY_EXPANDED_TOOL_FILTERS,
  DEFAULT_CHAT_ACTIVITY_EXPAND_KEYS,
  DEFAULT_CHAT_TOOL_ACTIVITY_FILTERS,
  normalizeChatActivityDefaultExpanded,
  normalizeChatActivityFilters,
  normalizeChatToolActivityId,
  normalizeChatToolExpansionOverrides,
  normalizeChatToolPreferenceId,
  resolveChatToolDefaultExpanded,
} from '../src/lib/chatActivity'

test('activity defaults enable Agena transport detail types without an Agent category', () => {
  assert.equal(DEFAULT_CHAT_ACTIVITY_FILTERS.includes('step-start'), true)
  assert.equal(DEFAULT_CHAT_ACTIVITY_FILTERS.includes('step-finish'), true)
  assert.equal(DEFAULT_CHAT_ACTIVITY_FILTERS.includes('agent'), false)

  const optionIds = ACTIVITY_DEFAULT_EXPANDED_OPTIONS.map((item) => item.id)
  assert.equal(optionIds.includes('step-start'), false)
  assert.equal(optionIds.includes('step-finish'), false)
  assert.equal(optionIds.includes('agent'), false)
})

test('normalizers discard removed Agent keys and keep supported keys in default order', () => {
  const filters = normalizeChatActivityFilters(['step-start', 'agent', 'snapshot'])
  assert.deepEqual(filters, ['tool', 'step-start', 'snapshot'])

  const expanded = normalizeChatActivityDefaultExpanded(['step-finish', 'agent', 'snapshot', 'thinking'])
  assert.deepEqual(expanded, ['snapshot', 'thinking'])
})

test('default expansion only opens file-modifying details', () => {
  assert.deepEqual(DEFAULT_CHAT_ACTIVITY_EXPAND_KEYS, [])

  assert.deepEqual(DEFAULT_CHAT_ACTIVITY_EXPANDED_TOOL_FILTERS, ['edit', 'write', 'apply_patch', 'multiedit'])
  for (const tool of DEFAULT_CHAT_ACTIVITY_EXPANDED_TOOL_FILTERS) {
    assert.equal(DEFAULT_CHAT_TOOL_ACTIVITY_FILTERS.includes(tool), true)
  }
})

test('expand defaults stay configurable with stable normalization order', () => {
  const expanded = normalizeChatActivityDefaultExpanded([
    ' thinking ',
    'patch',
    'retry',
    'snapshot',
    'patch',
    'JUSTIFICATION',
    'compaction',
    'unknown',
  ])

  assert.deepEqual(expanded, ['snapshot', 'patch', 'retry', 'compaction', 'thinking', 'justification'])
  assert.deepEqual(CHAT_ACTIVITY_EXPAND_KEYS, ['snapshot', 'patch', 'retry', 'compaction', 'thinking', 'justification'])
})

test('Agena namespaced tools map to the existing activity categories', () => {
  assert.equal(normalizeChatToolActivityId('fs.read'), 'read')
  assert.equal(normalizeChatToolActivityId('fs.replace'), 'edit')
  assert.equal(normalizeChatToolActivityId('fs.apply_patch'), 'apply_patch')
  assert.equal(normalizeChatToolActivityId('shell.run'), 'bash')
  assert.equal(normalizeChatToolActivityId('web.search'), 'websearch')
  assert.equal(normalizeChatToolActivityId('web.fetch'), 'webfetch')
  assert.equal(normalizeChatToolActivityId('interaction.ask'), 'question')
  assert.equal(normalizeChatToolActivityId('tasks.run'), 'task')
  assert.equal(normalizeChatToolActivityId('custom.plugin_tool'), 'custom.plugin_tool')
})

test('exact tool expansion overrides do not collapse into broad categories', () => {
  assert.equal(normalizeChatToolPreferenceId('agena.fs.read'), 'fs.read')
  assert.equal(normalizeChatToolPreferenceId('fs.read_many'), 'fs.read_many')

  const overrides = normalizeChatToolExpansionOverrides({
    'agena.fs.read': true,
    'fs.read_many': false,
    'agena.shell.run': 'invalid',
  })
  const legacy = new Set<string>(['edit', 'write', 'apply_patch', 'multiedit'])

  assert.deepEqual(overrides, { 'fs.read': true, 'fs.read_many': false })
  assert.equal(resolveChatToolDefaultExpanded('fs.read', overrides, legacy), true)
  assert.equal(resolveChatToolDefaultExpanded('agena.fs.read_many', overrides, legacy), false)
  assert.equal(resolveChatToolDefaultExpanded('fs.replace', overrides, legacy), true)
  assert.equal(resolveChatToolDefaultExpanded('fs.glob', overrides, legacy), false)
})
