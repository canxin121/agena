import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

const directorySource = readFileSync(
  new URL('../src/layout/chatSidebar/components/DirectoryRow.vue', import.meta.url),
  'utf8',
)
const sessionSource = readFileSync(
  new URL('../src/layout/chatSidebar/components/SessionRow.vue', import.meta.url),
  'utf8',
)

test('chat sidebar expand/collapse affordances use native buttons', () => {
  assert.match(directorySource, /<button\s+type="button"[\s\S]*directoriesList\.expandDirectory/)
  assert.match(sessionSource, /<button\s+v-if="isParent"\s+type="button"[\s\S]*sessionRow\.threadToggle\.collapse/)
  assert.doesNotMatch(directorySource, /role="button"/)
  assert.doesNotMatch(sessionSource, /role="button"/)
})
