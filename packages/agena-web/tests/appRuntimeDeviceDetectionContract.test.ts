import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

const appRuntimeSource = readFileSync(new URL('../src/app/runtime/useAppRuntime.ts', import.meta.url), 'utf8')
const deviceRuntimeSource = readFileSync(new URL('../src/app/runtime/useDeviceRuntime.ts', import.meta.url), 'utf8')
const appSource = readFileSync(new URL('../src/App.vue', import.meta.url), 'utf8')

test('root device runtime owns the shared detector for loading, login, and workspace UI', () => {
  assert.match(appSource, /useDeviceRuntime\(\)/)
  assert.match(deviceRuntimeSource, /applyDeviceClasses, getDeviceInfo/)
  assert.match(deviceRuntimeSource, /ui\.setIsCompactLayout\(info\.isCompactLayout\)/)
  assert.match(deviceRuntimeSource, /ui\.setIsMobileDevice\(info\.isMobileDevice\)/)
  assert.match(deviceRuntimeSource, /ui\.setIsTouchPointer\(info\.isTouchPointer\)/)
  assert.match(deviceRuntimeSource, /ui\.setIsMobilePointer\(info\.isMobilePointer\)/)
  assert.match(deviceRuntimeSource, /applyDevice\(\)[\s\S]*onMounted/)
  assert.match(deviceRuntimeSource, /addEventListener\?\.\('change', applyDevice\)/)
  assert.doesNotMatch(appRuntimeSource, /getDeviceInfo|applyDeviceClasses|function\s+applyDevice/)
})
