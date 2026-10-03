import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('global Agena SSE gaps, lag, and errors revalidate durable sidebar state', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/app/runtime/useAppRuntime.ts'), 'utf8')
  const revalidation = 'recovery.invalidate(0)'

  assert.ok(source.includes("endpoint: '/api/v1/changes/stream?scope_kind=global'"))
  assert.ok(source.includes('onSequenceGap: () =>'))
  assert.ok(source.includes('onOpen: () => recovery.invalidate(0)'))
  assert.ok(source.includes('chat.reconcileLiveState()'))
  assert.ok(source.includes('directorySessions.scheduleSidebarRecoverySync'))
  assert.ok(source.includes("evt.type === 'lagged' || evt.type === 'subscription_closed'"))
  assert.ok(source.includes('onError: (err) =>'))
  assert.ok(source.split(revalidation).length - 1 >= 3)
})
