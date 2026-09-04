import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

const source = readFileSync(new URL('../src/components/ui/ConfirmPopover.vue', import.meta.url), 'utf8')

test('mobile confirmation dialogs stay centered inside the visual safe area', () => {
  assert.match(source, /const\s+dialogContentClass\s*=\s*computed\(/)
  assert.match(source, /ui\.isCompactTouch/)
  assert.match(source, /--oc-safe-area-top/)
  assert.match(source, /--oc-safe-area-right/)
  assert.match(source, /--oc-safe-area-bottom/)
  assert.match(source, /--oc-safe-area-left/)
  assert.doesNotMatch(source, /sm:max-h-\[calc\(100dvh-3rem\)\]/)
  assert.match(source, /:class="dialogContentClass"/)
})
