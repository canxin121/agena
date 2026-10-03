import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

const source = readFileSync(new URL('../src/components/chat/PlanViewerDialog.vue', import.meta.url), 'utf8')

test('plan viewer fills mobile fullscreen dialogs even in wide landscape viewports', () => {
  assert.match(source, /useUiStore\(\)/)
  assert.match(source, /ui\.isCompactTouch \? 'h-full'/)
  assert.match(source, /:body-scroll="false"/)
  assert.match(source, /min-h-0 flex-1 overflow-auto overscroll-contain/)
  assert.doesNotMatch(source, /sm:max-h-/)
})
