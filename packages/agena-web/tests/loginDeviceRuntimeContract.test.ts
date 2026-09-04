import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

const appSource = readFileSync(new URL('../src/App.vue', import.meta.url), 'utf8')
const loginSource = readFileSync(new URL('../src/pages/LoginPage.vue', import.meta.url), 'utf8')

test('device runtime is initialized above the auth gate so LoginPage pickers receive mobile state', () => {
  assert.match(appSource, /useDeviceRuntime\(\)/)
  assert.match(appSource, /<LoginPage v-else-if="showLogin"/)
  assert.match(loginSource, /<OptionPicker/)
})
