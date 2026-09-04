import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

const source = readFileSync(new URL('../src/components/ui/NameInputPrompt.vue', import.meta.url), 'utf8')

test('mobile name prompts respect all safe areas and use a Radix dialog focus scope', () => {
  assert.match(source, /<DialogRoot/)
  assert.match(source, /<DialogContent as-child>/)
  assert.match(source, /<DialogTitle/)
  assert.match(source, /--oc-safe-area-top/)
  assert.match(source, /--oc-safe-area-right/)
  assert.match(source, /--oc-safe-area-bottom/)
  assert.match(source, /--oc-safe-area-left/)
})

test('Escape closes a mobile name prompt from any focused child', () => {
  assert.match(source, /@update:open="onMobileOpenChange"/)
  assert.doesNotMatch(source, /@keydown\.esc\.prevent\.stop="close"/)
})
