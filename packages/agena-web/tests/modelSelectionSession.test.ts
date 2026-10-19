import test from 'node:test'
import assert from 'node:assert/strict'

import {
  deriveSessionSelectionFromMessages,
  normalizeSessionManualModelStorageEntry,
  readSessionManualModelEntry,
  readSessionManualModelPair,
  readSessionRunConfigSelection,
  removeSessionManualModelPair,
  sessionCachePredatesSession,
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
      at: 1,
    }),
    {
      provider: 'openai',
      adapter: 'responses',
      model: 'gpt-5',
      thinkingMode: 'high',
      speedMode: 'fast',
      verbosity: 'compact',
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

test('manual model storage keeps the model identity together with the moment it was chosen', () => {
  assert.deepEqual(
    normalizeSessionManualModelStorageEntry(' session-1 ', JSON.stringify({ at: 1700, value: ' openai//gpt-5 ' })),
    { key: 'session-1', value: JSON.stringify({ at: 1700, value: 'openai//gpt-5' }) },
  )
  // Legacy entries were bare slugs without a timestamp. They cannot be
  // attributed to a session, so they are dropped instead of trusted.
  assert.equal(normalizeSessionManualModelStorageEntry('session-1', ' openai//gpt-5 '), null)
  assert.equal(normalizeSessionManualModelStorageEntry('session-1', 'openai/gpt-5'), null)
  assert.equal(normalizeSessionManualModelStorageEntry('session-1', 'invalid'), null)
  assert.equal(
    normalizeSessionManualModelStorageEntry('session-1', JSON.stringify({ at: 0, value: 'openai//gpt-5' })),
    null,
  )
  assert.equal(
    normalizeSessionManualModelStorageEntry('session-1', JSON.stringify({ at: 1, value: 'openai/gpt-5' })),
    null,
  )
})

test('write, read, and remove a session manual model identity', () => {
  const initial = { 'session-a': JSON.stringify({ at: 100, value: 'openai//gpt-5' }) }
  const updated = writeSessionManualModelPair(initial, 'session-b', 'anthropic', 'messages', 'claude-sonnet', 200)
  assert.deepEqual(readSessionManualModelEntry(updated, 'session-b'), {
    at: 200,
    value: 'anthropic/messages/claude-sonnet',
  })
  assert.deepEqual(readSessionManualModelPair(updated, 'session-b'), {
    provider: 'anthropic',
    adapter: 'messages',
    model: 'claude-sonnet',
  })
  assert.equal(
    writeSessionManualModelPair(updated, 'session-b', 'anthropic', 'messages', 'claude-sonnet', 200),
    updated,
  )
  assert.equal(readSessionManualModelEntry(updated, 'session-b-missing'), null)

  const removed = removeSessionManualModelPair(updated, 'session-b')
  assert.deepEqual(readSessionManualModelPair(removed, 'session-b'), {
    provider: '',
    adapter: '',
    model: '',
  })
  assert.equal(removeSessionManualModelPair(removed, 'missing'), removed)
})

test('session caches written before the session existed are stale', () => {
  const createdMs = Date.parse('2026-10-19T13:43:26Z')

  assert.equal(sessionCachePredatesSession(createdMs + 1_000, createdMs), false)
  assert.equal(sessionCachePredatesSession(createdMs - 60_000, createdMs), false)
  assert.equal(sessionCachePredatesSession(Date.parse('2026-08-30T06:45:00Z'), createdMs), true)
  assert.equal(sessionCachePredatesSession(0, createdMs), true)
  assert.equal(sessionCachePredatesSession(Number.NaN, createdMs), true)
  // Without a known creation time only catalog availability can judge a cache.
  assert.equal(sessionCachePredatesSession(Date.parse('2026-08-30T06:45:00Z'), null), false)
})
