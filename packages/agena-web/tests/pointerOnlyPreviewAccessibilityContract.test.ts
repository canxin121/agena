import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

const attachment = readFileSync(new URL('../src/components/chat/AgenaAttachmentPreview.vue', import.meta.url), 'utf8')
const gitImage = readFileSync(new URL('../src/components/git/GitEditorDiffViewer.vue', import.meta.url), 'utf8')
const schema = readFileSync(new URL('../src/components/settings/plugins/JsonSchemaField.vue', import.meta.url), 'utf8')
const diffPane = readFileSync(new URL('../src/components/git/GitDiffPane.vue', import.meta.url), 'utf8')

test('chat and git image previews are keyboard-focusable native buttons', () => {
  assert.match(attachment, /<button[\s\S]*v-if="isImage && href"[\s\S]*:aria-label=/)
  assert.doesNotMatch(attachment, /<img[^>]+@click=/)

  assert.equal((gitImage.match(/class="preview-button"/g) || []).length, 2)
  assert.match(gitImage, /\.preview-button:focus-visible/)
  assert.doesNotMatch(gitImage, /<img[^>]+@click=/)
})

test('icon-only schema and compact diff controls expose accessible names', () => {
  assert.match(schema, /:aria-label="\$st\('Move up'\)"/)
  assert.match(schema, /:aria-label="\$st\('Move down'\)"/)
  assert.match(schema, /:aria-label="\$st\('Duplicate'\)"/)
  assert.match(diffPane, /:aria-label="t\('common\.back'\)"/)
})
