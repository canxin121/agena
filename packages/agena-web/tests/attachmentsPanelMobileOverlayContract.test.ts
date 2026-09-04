import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

const source = readFileSync(new URL('../src/components/chat/AttachmentsPanel.vue', import.meta.url), 'utf8')

test('mobile attachments sheet uses a Radix dialog focus scope above bottom navigation', () => {
  assert.match(source, /<DialogRoot/)
  assert.match(source, /<DialogPortal>/)
  assert.match(source, /<DialogOverlay class="fixed inset-0 z-\[64\]/)
  assert.match(source, /<DialogContent as-child>/)
  assert.match(source, /<DialogTitle/)
  assert.match(source, /fixed z-\[65\]/)
  assert.match(source, /tabindex="-1"/)
})

test('mobile attachments sheet preserves focus restoration and visual viewport offsets', () => {
  assert.match(source, /if \(!props\.open \|\| isMobileSheet\.value\) return[\s\S]*event\.key !== 'Escape'/)
  assert.match(source, /visualViewport\?\.offsetTop/)
  assert.match(source, /visualViewport\?\.offsetLeft/)
  assert.match(source, /--oc-safe-area-left/)
  assert.match(source, /--oc-safe-area-right/)
  assert.match(source, /panelCenter/)
  assert.match(source, /Math\.max\(0, bottomEdge - topInset\)/)
  assert.doesNotMatch(source, /MOBILE_SHEET_MIN_MAX_HEIGHT_PX/)
  assert.doesNotMatch(source, /focusMobilePanel/)
  assert.match(source, /if \(!props\.open \|\| isMobileSheet\.value\) return/)
  assert.equal((source.match(/document\.addEventListener\('keydown', onDocumentKeydown\)/g) || []).length, 1)
  assert.match(source, /restoreReturnFocus/)
  assert.match(source, /target\.focus\(\{ preventScroll: true \}\)/)
})
