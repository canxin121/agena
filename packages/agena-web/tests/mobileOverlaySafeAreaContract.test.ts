import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

test('fullscreen dialogs keep content and close controls inside mobile safe areas', () => {
  const source = readFileSync(new URL('../src/components/ui/Dialog.vue', import.meta.url), 'utf8')
  assert.match(source, /props\.mobileFullscreen && ui\.isCompactTouch/)
  assert.match(source, /--oc-safe-area-top/)
  assert.match(source, /--oc-safe-area-right/)
  assert.match(source, /--oc-safe-area-bottom/)
  assert.match(source, /--oc-safe-area-left/)
  // The shared header stays inside the safe-area-padded content instead of
  // positioning its close control relative to the screen edge.
  assert.match(source, /<DialogHeader/)
  const header = readFileSync(new URL('../src/components/ui/DialogHeader.vue', import.meta.url), 'utf8')
  assert.match(header, /shrink-0/)
  assert.doesNotMatch(header, /absolute/)
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
