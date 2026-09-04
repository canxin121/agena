import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

const source = readFileSync(new URL('../src/components/ui/OptionMenu.vue', import.meta.url), 'utf8')

test('option menus render above standard dialogs and below confirmation overlays', () => {
  const z75 = source.match(/z-\[75\]/g) || []
  assert.ok(z75.length >= 2)
  assert.doesNotMatch(source, /z-\[60\]/)
  assert.doesNotMatch(source, /class="fixed inset-0 z-50"/)
})

test('mobile option sheets use a nested Radix dialog focus scope', () => {
  assert.match(source, /<DialogRoot/)
  assert.match(source, /<DialogContent as-child>/)
  assert.match(source, /<DialogTitle/)
  assert.match(source, /tabindex="-1"/)
  assert.match(source, /focusMobilePanel/)
  assert.match(source, /--oc-safe-area-left/)
  assert.match(source, /--oc-safe-area-right/)
  assert.match(source, /visualViewport\?\.offsetLeft/)
  assert.match(source, /panelCenter/)
  assert.doesNotMatch(source, /document\.addEventListener\('keydown'/)
})

test('desktop option menus use a Radix popover focus scope and preserve virtual anchors', () => {
  assert.match(source, /<PopoverRoot/)
  assert.match(source, /<PopoverAnchor :element="desktopPopoverAnchor"/)
  assert.match(source, /<PopoverContent/)
  assert.match(source, /resolveDesktopMeasurable/)
  assert.match(source, /getBoundingClientRect/)
  assert.doesNotMatch(source, /syncDesktopFixedPosition/)
  assert.doesNotMatch(source, /document\.addEventListener\('click'/)
})

test('option menus restore trigger focus without stealing focus from a newly opened surface', () => {
  assert.match(source, /captureReturnFocus/)
  assert.match(source, /restoreReturnFocus/)
  assert.match(source, /target\.isConnected/)
  assert.match(source, /active !== document\.body && active !== document\.documentElement/)
  assert.match(source, /target\.focus\(\{ preventScroll: true \}\)/)
})
