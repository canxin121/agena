import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

test('fullscreen dialogs keep content and close controls inside mobile safe areas', () => {
  const source = readFileSync(new URL('../src/components/ui/Dialog.vue', import.meta.url), 'utf8')
  assert.match(source, /props\.mobileFullscreen && ui\.isMobilePointer/)
  assert.match(source, /--oc-safe-area-top/)
  assert.match(source, /--oc-safe-area-right/)
  assert.match(source, /--oc-safe-area-bottom/)
  assert.match(source, /--oc-safe-area-left/)
  assert.match(source, /right-\[calc\(0\.75rem\+var\(--oc-safe-area-right,0px\)\)\]/)
  assert.match(source, /top-\[calc\(0\.75rem\+var\(--oc-safe-area-top,0px\)\)\]/)
  assert.doesNotMatch(source, /sm:right-3 sm:top-3/)
})

test('mobile form sheets respect horizontal safe-area insets', () => {
  const source = readFileSync(new URL('../src/components/ui/FormDialog.vue', import.meta.url), 'utf8')
  assert.match(source, /visualViewport\?\.offsetLeft/)
  assert.match(source, /--oc-safe-area-left/)
  assert.match(source, /--oc-safe-area-right/)
  assert.match(source, /panelCenter/)
  assert.match(source, /panelWidth/)
  assert.match(source, /-translate-x-1\/2/)
  assert.match(source, /visualViewport\?\.offsetTop/)
  assert.match(source, /viewportTop \+ safeTop/)
  assert.match(source, /Math\.max\(0, bottomEdge - topInset\)/)
  assert.doesNotMatch(source, /MOBILE_SHEET_MIN_MAX_HEIGHT_PX/)
})
