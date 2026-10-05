import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('the wheel still moves the transcript cursor when nothing can scroll', () => {
  const vim = readFileSync(resolve(import.meta.dir, '../src/pages/chat/useChatTranscriptVim.ts'), 'utf8')

  // A transcript that fits its viewport never emits a scroll event, and a
  // viewport parked at the edge the gesture points at cannot scroll further
  // either, so the cursor used to freeze at the tail. The wheel has to move it
  // directly and stop at the first/last line.
  assert.match(vim, /const WHEEL_CURSOR_LINES = 3/)
  assert.match(vim, /function onTranscriptWheel\(event: WheelEvent\)/)
  assert.match(vim, /const maxScroll = Math\.max\(0, scroll\.scrollHeight - scroll\.clientHeight\)/)
  assert.match(vim, /if \(maxScroll > 1 && !atEdge\) return/)
  assert.match(vim, /moveVisualLines\(direction, WHEEL_CURSOR_LINES\)/)
  assert.match(vim, /mountedScroll\?\.addEventListener\('wheel', onTranscriptWheel, \{ passive: false \}\)/)
  assert.match(vim, /mountedScroll\?\.removeEventListener\('wheel', onTranscriptWheel\)/)
})
