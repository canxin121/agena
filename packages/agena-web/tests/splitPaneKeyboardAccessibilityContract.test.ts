import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

const vertical = readFileSync(new URL('../src/components/ui/VerticalSplitPane.vue', import.meta.url), 'utf8')
const multi = readFileSync(new URL('../src/components/ui/MultiPaneHorizontalSplit.vue', import.meta.url), 'utf8')

test('chat vertical splitter is a focusable adjustable separator', () => {
  assert.match(vertical, /role="separator"/)
  assert.match(vertical, /aria-orientation="horizontal"/)
  assert.match(vertical, /:aria-valuenow="Math\.round\(modelValue\)"/)
  assert.match(vertical, /:tabindex="disabled \? -1 : 0"/)
  assert.match(vertical, /event\.key === 'ArrowUp'/)
  assert.match(vertical, /event\.key === 'ArrowDown'/)
  assert.match(vertical, /focus-visible:ring-2/)
})

test('workspace horizontal splitters support keyboard resizing', () => {
  assert.match(multi, /role="separator"/)
  assert.match(multi, /aria-orientation="vertical"/)
  assert.match(multi, /:aria-valuenow="separatorValueNow\(index\)"/)
  assert.match(multi, /event\.key === 'ArrowLeft'/)
  assert.match(multi, /event\.key === 'ArrowRight'/)
  assert.match(multi, /resizePairFromKeyboard/)
  assert.match(multi, /focus-visible:ring-2/)
})
