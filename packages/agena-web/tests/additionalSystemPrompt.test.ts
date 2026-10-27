import assert from 'node:assert/strict'
import { test } from 'node:test'

import {
  additionalSystemPromptDraft,
  additionalSystemPromptIsDirty,
  additionalSystemPromptIsValid,
  additionalSystemPromptValue,
} from '../src/lib/additionalSystemPrompt'

test('prompt modes preserve inheritance, explicit disable, and exact custom content', () => {
  for (const value of [null, undefined, '', 'Be concise.', '  Preserve indentation.\n', '  ']) {
    const draft = additionalSystemPromptDraft(value)
    assert.equal(additionalSystemPromptValue(draft), value ?? null)
    assert.equal(additionalSystemPromptIsValid(draft), true)
    assert.equal(additionalSystemPromptIsDirty(draft, value ?? null), false)
  }
  assert.equal(additionalSystemPromptDraft(null).mode, 'inherit')
  assert.equal(additionalSystemPromptDraft('').mode, 'disabled')
})

test('an empty custom draft is distinct from a saved disabled override', () => {
  const draft = { mode: 'custom' as const, text: '' }
  assert.equal(additionalSystemPromptIsValid(draft), false)
  assert.equal(additionalSystemPromptIsDirty(draft, ''), true)
  assert.equal(additionalSystemPromptIsDirty(draft, null), true)
})

test('mode changes remain dirty even when text matches an inherited value', () => {
  assert.equal(additionalSystemPromptIsDirty({ mode: 'custom', text: 'Inherited text' }, null), true)
  assert.equal(additionalSystemPromptIsDirty({ mode: 'disabled', text: 'Retained draft' }, null), true)
  assert.equal(additionalSystemPromptIsDirty({ mode: 'inherit', text: 'Retained draft' }, null), false)
  assert.equal(additionalSystemPromptValue({ mode: 'disabled', text: 'Retained draft' }), '')
  assert.equal(additionalSystemPromptValue({ mode: 'inherit', text: 'Retained draft' }), null)
})
