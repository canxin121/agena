import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

const source = readFileSync(new URL('../src/components/ui/ListItemFrame.vue', import.meta.url), 'utf8')

test('list item rows avoid nesting action buttons inside a button root', () => {
  assert.match(source, /const\s+usesButtonSurrogate\s*=\s*computed\([^\n]*slots\.actions/)
  assert.match(source, /const\s+resolvedAs\s*=\s*computed\(/)
  assert.match(source, /data-oc-list-item-primary/)
  assert.match(source, /v-if="usesButtonSurrogate"[\s\S]*type="button"/)
  assert.match(source, /@click\.stop="emit\('click', \$event\)"/)
  assert.doesNotMatch(source, /:role="usesButtonSurrogate \? 'button' : undefined"/)
  assert.doesNotMatch(source, /:tabindex="usesButtonSurrogate/)
})
