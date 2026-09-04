import test from 'node:test'
import assert from 'node:assert/strict'

import {
  deriveSessionSelectionFromMessages,
  normalizeSessionManualModelStorageEntry,
  readSessionManualModelPair,
  readSessionRunConfigSelection,
  removeSessionManualModelPair,
  writeSessionManualModelPair,
} from '../src/pages/chat/modelSelectionSession'

test('readSessionRunConfigSelection trims Agena model and mode fields', () => {
  assert.deepEqual(
    readSessionRunConfigSelection({
      providerID: ' openai ',
      adapterID: ' responses ',
      modelID: ' gpt-5 ',
      thinkingMode: ' high ',
      speedMode: ' fast ',
      verbosity: ' compact ',
      parallelToolCalls: false,
      at: 1,
    }),
    {
      provider: 'openai',
      adapter: 'responses',
      model: 'gpt-5',
      thinkingMode: 'high',
      speedMode: 'fast',
      verbosity: 'compact',
      parallelToolCalls: false,
    },
  )
})

test('deriveSessionSelectionFromMessages prefers the latest complete run marker identity', () => {
  assert.deepEqual(
    deriveSessionSelectionFromMessages([
      { info: { providerID: 'anthropic', adapterID: 'messages', modelID: 'claude-sonnet' } },
      { info: { providerID: 'openai' } },
    ]),
    {
      provider: 'anthropic',
      adapter: 'messages',
      model: 'claude-sonnet',
      thinkingMode: '',
      speedMode: '',
      verbosity: '',
    },
  )
})

test('manual model storage accepts only current provider/adapter/model keys', () => {
  assert.deepEqual(normalizeSessionManualModelStorageEntry(' session-1 ', ' openai//gpt-5 '), {
    key: 'session-1',
    value: 'openai//gpt-5',
  })
  assert.equal(normalizeSessionManualModelStorageEntry('session-1', 'openai/gpt-5'), null)
  assert.equal(normalizeSessionManualModelStorageEntry('session-1', 'invalid'), null)
})

test('write, read, and remove a session manual model identity', () => {
  const initial = { 'session-a': 'openai//gpt-5' }
  const updated = writeSessionManualModelPair(initial, 'session-b', 'anthropic', 'messages', 'claude-sonnet')
  assert.deepEqual(readSessionManualModelPair(updated, 'session-b'), {
    provider: 'anthropic',
    adapter: 'messages',
    model: 'claude-sonnet',
  })
  assert.equal(writeSessionManualModelPair(updated, 'session-b', 'anthropic', 'messages', 'claude-sonnet'), updated)

  const removed = removeSessionManualModelPair(updated, 'session-b')
  assert.deepEqual(readSessionManualModelPair(removed, 'session-b'), {
    provider: '',
    adapter: '',
    model: '',
  })
  assert.equal(removeSessionManualModelPair(removed, 'missing'), removed)
})
