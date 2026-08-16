import assert from 'node:assert/strict'
import test from 'node:test'

import {
  mergeSessionStateSnapshot,
  sessionStateIsActive,
  sessionStateHasAttention,
} from '../src/stores/directorySessionRuntime.ts'

const ready = { kind: 'ready', data: {} }
const running = { kind: 'running', data: { workflow: 'quiescent' } }
const interrupted = { kind: 'interrupted', data: { reason: 'lease_lost' } }

test('state snapshots replace the canonical state when newer', () => {
  const next = mergeSessionStateSnapshot({ state: running, updatedAt: 100 }, { state: interrupted, updatedAt: 110 })

  assert.equal(next.state.kind, 'interrupted')
  assert.equal(sessionStateHasAttention(next), true)
  assert.equal(sessionStateIsActive(next), false)
})

test('a stale state snapshot cannot regress the canonical state', () => {
  const current = { state: interrupted, updatedAt: 200 }
  const next = mergeSessionStateSnapshot(current, { state: running, updatedAt: 120 })

  assert.deepEqual(next, current)
})

test('an untimestamped snapshot can seed an empty state map', () => {
  const next = mergeSessionStateSnapshot(undefined, { state: ready })

  assert.deepEqual(next, { state: ready, updatedAt: 0 })
})

test('creating and running are active, while awaiting interaction is attention', () => {
  assert.equal(sessionStateIsActive({ state: { kind: 'creating' }, updatedAt: 0 }), true)
  assert.equal(sessionStateIsActive({ state: running, updatedAt: 0 }), true)
  assert.equal(sessionStateHasAttention({ state: { kind: 'awaiting_interaction', data: {} }, updatedAt: 0 }), true)
})
