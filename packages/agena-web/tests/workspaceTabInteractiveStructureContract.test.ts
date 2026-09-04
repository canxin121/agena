import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

const source = readFileSync(new URL('../src/layout/WorkspaceEditorGroupPane.vue', import.meta.url), 'utf8')

test('workspace tabs keep the primary tab action and close action as sibling native buttons', () => {
  assert.doesNotMatch(source, /role="button"/)
  assert.match(source, /type="button"[\s\S]*:aria-pressed="isWindowActive\(windowTab\.id\)"/)
  assert.match(source, /group-focus-within:opacity-100/)
  assert.doesNotMatch(source, /@keydown\.enter/)
  assert.doesNotMatch(source, /@keydown\.space/)
})
