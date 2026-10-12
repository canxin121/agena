import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

// Global selections are stored per workspace window, but the chat page recorded
// them under whichever window was active - not necessarily the pane writing.

test('chat page records global selections under its own window scope', () => {
  const chatPage = readFileSync(resolve(import.meta.dir, '../src/pages/ChatPage.vue'), 'utf8')
  const writes = chatPage.match(/ui\.setGlobalSelection\(/g) || []
  const scoped = chatPage.match(/ui\.setGlobalSelection\([\s\S]*?\{\n\s*\.\.\.paneWindowScope\(\),/g) || []

  assert.ok(writes.length > 0)
  // Every write carries the pane scope, so none of them can describe another pane.
  assert.equal(scoped.length, writes.length)
  assert.ok(chatPage.includes('function paneWindowScope()'))
  assert.ok(chatPage.includes('workspacePane?.windowId.value'))
})
