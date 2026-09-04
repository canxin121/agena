import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

const bottomNav = readFileSync(new URL('../src/layout/BottomNav.vue', import.meta.url), 'utf8')
const app = readFileSync(new URL('../src/App.vue', import.meta.url), 'utf8')

test('bottom navigation stays inside horizontal and bottom safe areas', () => {
  assert.match(bottomNav, /--oc-safe-area-left/)
  assert.match(bottomNav, /--oc-safe-area-right/)
  assert.match(bottomNav, /--oc-safe-area-bottom/)
})

test('application boot loading state is announced to assistive technology', () => {
  assert.match(app, /role="status"/)
  assert.match(app, /aria-live="polite"/)
  assert.match(app, /:aria-label="String\(t\('common\.loading'\)\)"/)
})
