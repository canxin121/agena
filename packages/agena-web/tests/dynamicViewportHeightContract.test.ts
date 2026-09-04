import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

const files = [
  '../src/pages/files/components/FileViewerPane.vue',
  '../src/components/git/GitStashViewDialog.vue',
  '../src/components/HelpDialog.vue',
  '../src/components/ui/PathPicker.vue',
]

test('viewport-proportional panels use dynamic viewport height units', () => {
  for (const file of files) {
    const source = readFileSync(new URL(file, import.meta.url), 'utf8')
    assert.doesNotMatch(source, /(?<![dsl])vh/)
  }
})
