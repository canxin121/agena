import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

const source = readFileSync(new URL('../src/components/git/GitEditorDiffViewer.vue', import.meta.url), 'utf8')

test('git image diff preview tracks the dynamic viewport', () => {
  assert.match(source, /max-height:\s*calc\(100dvh - 260px\)/)
  assert.doesNotMatch(source, /max-height:\s*calc\(100vh - 260px\)/)
})
