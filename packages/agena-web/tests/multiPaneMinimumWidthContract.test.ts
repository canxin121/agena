import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

const source = readFileSync(new URL('../src/components/ui/MultiPaneHorizontalSplit.vue', import.meta.url), 'utf8')

test('multi-pane layout preserves usable pane widths when the viewport is too narrow', () => {
  assert.match(source, /overflow-x-auto\s+overflow-y-hidden/)
  assert.match(source, /minWidth:\s*`\$\{Math\.max\(0, minPaneWidth\)\}px`/)
  assert.doesNotMatch(source, /class="h-full min-w-0 shrink-0 overflow-hidden"/)
})
