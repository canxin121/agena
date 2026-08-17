import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('FilesPage keeps an unfocused pane live from fs events', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/pages/FilesPage.vue'), 'utf8')
  assert.ok(source.includes('directoryStore.fsEventSeq'))
  assert.ok(source.includes('invalidateFileReadCache({ directory: rootPath, paths: affectedPaths })'))
  assert.ok(source.includes("void refreshCurrentFile({ source: 'manual', silent: true })"))
  assert.ok(source.includes('void refreshRoot()'))
})

test('Files explorer keeps a manual refresh action in the toolbar', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/pages/files/components/FilesExplorerPane.vue'), 'utf8')
  assert.ok(source.includes('@click="refreshRoot"'))
  assert.ok(source.includes("t('files.explorer.toolbar.refreshTree')"))
})
