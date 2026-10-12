import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

import {
  clearPreviewSessionId,
  createPreviewSessionScope,
  prunePreviewSessionScope,
  readPreviewSessionId,
  renamePreviewSessionId,
  scopeHoldsPreviewSession,
  writePreviewSessionId,
} from '../src/stores/previewSessionScope'

// Two preview panes used to follow each other because the active preview session
// lived in one global value.  It is now kept per workspace window.

test('each pane keeps its own preview session and only inherits the default', () => {
  let scope = createPreviewSessionScope('default-session')

  assert.equal(readPreviewSessionId(scope, 'left'), 'default-session')
  assert.equal(readPreviewSessionId(scope, 'right'), 'default-session')

  scope = writePreviewSessionId(scope, 'left', 'left-session')
  assert.equal(readPreviewSessionId(scope, 'left'), 'left-session')
  // The other pane keeps following the persisted default.
  assert.equal(readPreviewSessionId(scope, 'right'), 'default-session')
  assert.equal(readPreviewSessionId(scope), 'default-session')

  scope = writePreviewSessionId(scope, 'right', 'right-session')
  assert.equal(readPreviewSessionId(scope, 'left'), 'left-session')
  assert.equal(readPreviewSessionId(scope, 'right'), 'right-session')
})

test('only the default scope moves the persisted value', () => {
  let scope = createPreviewSessionScope('first')
  scope = writePreviewSessionId(scope, 'left', 'left-session')
  assert.equal(scope.defaultId, 'first')

  scope = writePreviewSessionId(scope, null, 'second')
  assert.equal(scope.defaultId, 'second')
  assert.equal(readPreviewSessionId(scope, 'left'), 'left-session')

  scope = writePreviewSessionId(scope, 'left', '')
  assert.equal(readPreviewSessionId(scope, 'left'), 'second')
  assert.equal(scope.defaultId, 'second')
})

test('deleted sessions leave every pane', () => {
  let scope = createPreviewSessionScope('shared')
  scope = writePreviewSessionId(scope, 'left', 'shared')
  scope = writePreviewSessionId(scope, 'right', 'other')

  assert.equal(scopeHoldsPreviewSession(scope, 'shared'), true)
  scope = clearPreviewSessionId(scope, 'shared')
  assert.equal(scopeHoldsPreviewSession(scope, 'shared'), false)
  assert.equal(readPreviewSessionId(scope, 'left'), '')
  assert.equal(readPreviewSessionId(scope, 'right'), 'other')
})

test('pruning drops unknown ids from every scope', () => {
  let scope = createPreviewSessionScope('gone-default')
  scope = writePreviewSessionId(scope, 'left', 'kept')
  scope = writePreviewSessionId(scope, 'right', 'gone')

  scope = prunePreviewSessionScope(scope, ['kept'])
  assert.equal(scope.defaultId, '')
  assert.equal(readPreviewSessionId(scope, 'left'), 'kept')
  assert.equal(readPreviewSessionId(scope, 'right'), '')

  // Nothing to prune means the same object, so reactivity stays quiet.
  assert.equal(prunePreviewSessionScope(scope, ['kept']), scope)
})

test('renaming a preview session follows it in every pane', () => {
  let scope = createPreviewSessionScope('old')
  scope = writePreviewSessionId(scope, 'left', 'old')

  const renamed = renamePreviewSessionId(scope, 'old', 'new')
  assert.equal(renamed.replaced, true)
  scope = renamed.scope
  assert.equal(readPreviewSessionId(scope, 'left'), 'new')
  assert.equal(readPreviewSessionId(scope), 'new')

  const untouched = renamePreviewSessionId(scope, 'missing', 'newer')
  assert.equal(untouched.replaced, false)
  assert.equal(untouched.scope, scope)
})

test('the preview store reads and writes the active session per pane', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/stores/workspacePreview.ts'), 'utf8')
  const proxy = source.slice(source.indexOf('function scopedPreview'))

  assert.ok(
    proxy.includes("if (property === 'activeSessionId') return target.readActiveSessionIdForWindow(windowId())"),
  )
  assert.ok(proxy.includes("if (property === 'activeSession') return target.activeSessionForWindow(windowId())"))
  assert.ok(proxy.includes('return (sessionId: string) => target.selectSessionForWindow(windowId(), sessionId)'))
  assert.ok(proxy.includes('target.selectSessionForWindow(windowId(), session.id)'))
  assert.ok(source.includes('const pane = useWorkspacePaneContext()'))
})
