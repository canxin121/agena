import assert from 'node:assert/strict'
import test from 'node:test'

import { extractSessionActivityUpdate } from '../src/lib/sessionActivityEvent.js'

test('extractSessionActivityUpdate: runtime_signal activity with running status maps to busy', () => {
  const evt = {
    type: 'runtime_signal',
    properties: {
      kind: 'activity',
      session_id: 7,
      payload: { id: 'a1', kind: 'activity', status: 'running', session_id: 7 },
    },
  }
  assert.deepEqual(extractSessionActivityUpdate(evt), { sessionID: '7', phase: 'busy' })
})

test('extractSessionActivityUpdate: runtime_signal activity with pending/waiting/paused maps to busy', () => {
  for (const status of ['pending', 'waiting', 'paused']) {
    const evt = {
      type: 'runtime_signal',
      properties: { kind: 'activity', session_id: 3, payload: { status, session_id: 3 } },
    }
    assert.deepEqual(extractSessionActivityUpdate(evt), { sessionID: '3', phase: 'busy' })
  }
})

test('extractSessionActivityUpdate: runtime_signal activity with terminal status maps to idle', () => {
  const evt = {
    type: 'runtime_signal',
    properties: { kind: 'activity', session_id: 4, payload: { status: 'terminal', session_id: 4 } },
  }
  assert.deepEqual(extractSessionActivityUpdate(evt), { sessionID: '4', phase: 'idle' })
})

test('extractSessionActivityUpdate: runtime_signal with non-activity kind is ignored', () => {
  const evt = { type: 'runtime_signal', properties: { kind: 'message_added', session_id: 's1' } }
  assert.equal(extractSessionActivityUpdate(evt), null)
})

test('extractSessionActivityUpdate: runtime_signal activity without a session id is ignored', () => {
  const evt = { type: 'runtime_signal', properties: { kind: 'activity', payload: { status: 'running' } } }
  assert.equal(extractSessionActivityUpdate(evt), null)
})

test('extractSessionActivityUpdate: session_changed run part in_progress maps to busy', () => {
  const evt = {
    type: 'session_changed',
    properties: {
      kind: 'part_added',
      session_id: 2,
      part: { part_id: 1, kind: 'run', state: 'in_progress' },
    },
  }
  assert.deepEqual(extractSessionActivityUpdate(evt), { sessionID: '2', phase: 'busy' })
})

test('extractSessionActivityUpdate: session_changed run part terminal maps to idle', () => {
  const evt = {
    type: 'session_changed',
    properties: {
      kind: 'part_updated',
      session_id: 5,
      part: { part_id: 1, kind: 'run', state: 'terminal' },
    },
  }
  assert.deepEqual(extractSessionActivityUpdate(evt), { sessionID: '5', phase: 'idle' })
})

test('extractSessionActivityUpdate: session_changed non-run part is ignored', () => {
  const evt = {
    type: 'session_changed',
    properties: { kind: 'part_added', session_id: 's1', part: { id: 'p1', kind: 'text' } },
  }
  assert.equal(extractSessionActivityUpdate(evt), null)
})

test('extractSessionActivityUpdate: legacy opencode event types are ignored', () => {
  assert.equal(
    extractSessionActivityUpdate({
      type: 'opencode-studio:session-activity',
      properties: { sessionID: 's1', phase: 'busy' },
    }),
    null,
  )
  assert.equal(
    extractSessionActivityUpdate({ type: 'session.status', properties: { sessionID: 's1', status: { type: 'busy' } } }),
    null,
  )
})
