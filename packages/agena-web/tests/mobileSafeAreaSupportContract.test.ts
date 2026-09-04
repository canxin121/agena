import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

const source = readFileSync(new URL('../src/styles/mobile.css', import.meta.url), 'utf8')

test('safe-area variables use the standard env capability instead of an iOS-only feature gate', () => {
  assert.match(source, /@supports\s*\(padding:\s*env\(safe-area-inset-top\)\)/)
  assert.doesNotMatch(source, /@supports\s*\(-webkit-touch-callout:\s*none\)/)
  assert.match(source, /--oc-safe-area-top:\s*env\(safe-area-inset-top, 0\)/)
  assert.match(source, /--oc-safe-area-right:\s*env\(safe-area-inset-right, 0\)/)
  assert.match(source, /--oc-safe-area-bottom:\s*env\(safe-area-inset-bottom, 0\)/)
  assert.match(source, /--oc-safe-area-left:\s*env\(safe-area-inset-left, 0\)/)
})
