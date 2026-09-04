import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

const source = readFileSync(new URL('../src/components/ImageViewerDialog.vue', import.meta.url), 'utf8')

test('image viewer toolbar and viewport stay inside device safe areas', () => {
  assert.match(source, /--oc-safe-area-top/)
  assert.match(source, /--oc-safe-area-right/)
  assert.match(source, /--oc-safe-area-bottom/)
  assert.match(source, /--oc-safe-area-left/)
})

test('image sizing follows the actual remaining flex viewport instead of a fixed toolbar estimate', () => {
  assert.match(source, /class="flex h-full min-h-0 w-full items-center justify-center/)
  assert.match(source, /class="max-h-full max-w-full select-none object-contain"/)
  assert.doesNotMatch(source, /100dvh-7\.5rem/)
})
