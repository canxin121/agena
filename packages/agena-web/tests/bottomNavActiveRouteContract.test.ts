import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

const source = readFileSync(new URL('../src/layout/BottomNav.vue', import.meta.url), 'utf8')

test('bottom navigation derives active state from the shared main-tab resolver', () => {
  assert.match(source, /mainTabFromPath\(route\.path\)/)
  assert.match(source, /activeMainTab\.value === tab/)
  assert.doesNotMatch(source, /route\.path\.startsWith\(path\)/)
})
