import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

const source = readFileSync(new URL('../src/layout/MainLayout.vue', import.meta.url), 'utf8')

test('compact layouts always keep primary navigation visible', () => {
  assert.match(
    source,
    /const\s+showBottomNav\s*=\s*computed\(\(\)\s*=>\s*ui\.isCompactLayout\s*&&\s*!isEmbeddedWorkspacePane\.value\)/,
  )
  assert.doesNotMatch(source, /showBottomNav[^\n]*isMobileDevice/)
})
