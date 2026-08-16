import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

import { runtimeFromAgenaSession } from '../src/stores/directorySessionRuntime'

test('Agena session lifecycle states map to visible sidebar runtime states', () => {
  assert.equal(
    runtimeFromAgenaSession({ state: 'running', updated_at: '2026-08-16T08:00:00Z' }).displayState,
    'running',
  )
  assert.equal(runtimeFromAgenaSession({ state: 'creating' }).displayState, 'running')
  assert.equal(runtimeFromAgenaSession({ state: 'awaiting_user' }).displayState, 'needsReply')
  assert.equal(runtimeFromAgenaSession({ state: 'interrupted' }).displayState, 'needsReply')
  assert.equal(runtimeFromAgenaSession({ state: 'ready' }).displayState, 'idle')
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
