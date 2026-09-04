import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

const source = readFileSync(new URL('../src/components/ui/DirectoryPathDialog.vue', import.meta.url), 'utf8')

test('directory picker uses full available height on mobile regardless of landscape width', () => {
  assert.match(source, /useUiStore\(\)/)
  assert.match(source, /ui\.isCompactTouch[\s\S]*'flex h-full min-h-0 flex-col'/)
  assert.match(source, /h-\[min\(56dvh,34rem\)\]/)
  assert.doesNotMatch(source, /sm:h-\[min\(56vh,34rem\)\]/)
})
