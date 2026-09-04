import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

const source = readFileSync(new URL('../src/components/ui/OptionPicker.vue', import.meta.url), 'utf8')

test('OptionPicker inherits the global mobile-pointer signal unless explicitly overridden', () => {
  assert.match(source, /useUiStore\(\)/)
  assert.match(source, /props\.isCompactTouch \?\? ui\.isCompactTouch/)
  assert.match(source, /:is-compact-touch="effectiveIsMobilePointer"/)
  assert.doesNotMatch(source, /isCompactTouch:\s*false/)
})

test('OptionPicker trigger keeps native focus semantics and exposes popup state', () => {
  assert.doesNotMatch(source, /@mousedown\.prevent/)
  assert.match(source, /aria-haspopup="menu"/)
  assert.match(source, /:aria-expanded="open"/)
  assert.match(source, /triggerEl\.value\?\.focus\(\{ preventScroll: true \}\)/)
})
