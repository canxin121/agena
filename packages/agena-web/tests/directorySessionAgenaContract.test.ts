import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

import { stateSnapshotFromAgenaSession } from '../src/stores/directorySessionRuntime'

test('Agena session lifecycle states stay canonical tagged SessionState values', () => {
  assert.equal(
    stateSnapshotFromAgenaSession({
      state: { kind: 'running', data: { workflow: 'quiescent' } },
      updated_at: '2026-08-16T08:00:00Z',
    }).state.kind,
    'running',
  )
  assert.equal(stateSnapshotFromAgenaSession({ state: { kind: 'creating' } }).state.kind, 'creating')
  assert.equal(
    stateSnapshotFromAgenaSession({
      state: { kind: 'awaiting_interaction', data: { requests: [] } },
    }).state.kind,
    'awaiting_interaction',
  )
  assert.equal(
    stateSnapshotFromAgenaSession({
      state: { kind: 'interrupted', data: { reason: 'lease_lost' } },
    }).state.kind,
    'interrupted',
  )
  assert.equal(stateSnapshotFromAgenaSession({ state: { kind: 'ready', data: {} } }).state.kind, 'ready')
})

test('sidebar uses cursor pagination and hydrates workspaces outside the visible page', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/stores/directorySessionStore.ts'), 'utf8')
  assert.ok(source.includes('excludeSubagents: true'))
  assert.ok(source.includes('seenCursors'))
  assert.ok(source.includes('await chatApi.getWorkspace(workspaceId)'))
  assert.ok(!source.includes('/api/v1/sessions/overview?'))
})

test('sidebar derives pinned sessions from durable server metadata', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/stores/directorySessionStore.ts'), 'utf8')
  assert.ok(source.includes('chat.updateSessionMetadata(sid, { pinned })'))
  assert.ok(source.includes('.filter((session) => session.pinned === true)'))
  assert.ok(source.includes('const pinnedIds = pinnedSessionIdSet(allOverview)'))
})
