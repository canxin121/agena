import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

const source = readFileSync(new URL('../src/pages/LoginPage.vue', import.meta.url), 'utf8')

test('login surface stays inside device safe areas without a fixed viewport-height estimate', () => {
  assert.match(source, /--oc-safe-area-top/)
  assert.match(source, /--oc-safe-area-right/)
  assert.match(source, /--oc-safe-area-bottom/)
  assert.match(source, /--oc-safe-area-left/)
  assert.doesNotMatch(source, /min-h-\[calc\(100dvh-3rem\)\]/)
})

test('login status, password field, and errors expose stable accessible semantics', () => {
  assert.match(source, /t\('login\.unlockDescription'\)/)
  assert.match(source, /role="status"/)
  assert.match(source, /aria-live="polite"/)
  assert.match(source, /:aria-label="String\(t\('login\.passwordPlaceholder'\)\)"/)
  assert.match(source, /role="alert"/)
  assert.doesNotMatch(source, />Connecting to server\.\.\.<\/div>/)
})
