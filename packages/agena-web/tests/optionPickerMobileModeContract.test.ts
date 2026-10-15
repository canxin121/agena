import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

const source = readFileSync(new URL('../src/components/ui/OptionPicker.vue', import.meta.url), 'utf8')
const menuSource = readFileSync(new URL('../src/components/ui/OptionMenu.vue', import.meta.url), 'utf8')

test('OptionPicker inherits the global mobile-pointer signal unless explicitly overridden', () => {
  assert.match(source, /useUiStore\(\)/)
  assert.match(source, /props\.isCompactTouch \?\? ui\.isCompactTouch/)
  assert.match(source, /:is-compact-touch="effectiveIsMobilePointer"/)
  assert.doesNotMatch(source, /isCompactTouch:\s*false/)
})

test('OptionPicker trigger keeps native focus semantics and exposes popup state', () => {
  assert.doesNotMatch(source, /@mousedown\.prevent/)
  assert.match(source, /aria-haspopup="dialog"/)
  assert.match(source, /:aria-expanded="open"/)
  // The shared menu owns restoration after Radix releases its focus scope.
  // A picker-level close watcher would steal focus from a newly opened surface.
  assert.match(source, /<OptionMenu\b[^>]*:open="open"[^>]*@update:open="setOpen"/)
  assert.match(source, /:desktop-anchor-el="desktopFixed \? triggerEl : null"/)
  assert.match(menuSource, /@close-auto-focus="onCloseAutoFocus"/)
  assert.match(menuSource, /target\.focus\(\{ preventScroll: true \}\)/)
  assert.doesNotMatch(source, /triggerEl\.value\?\.focus\(/)
})
