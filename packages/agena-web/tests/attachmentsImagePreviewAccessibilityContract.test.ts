import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

const source = readFileSync(new URL('../src/components/chat/AttachmentsPanel.vue', import.meta.url), 'utf8')

test('attachment image thumbnails are native focusable buttons instead of hidden click targets', () => {
  const previewButtons = source.match(/<button\s+v-if="isImageFile\(f\)"/g) || []
  assert.equal(previewButtons.length, 2)
  assert.match(source, /:aria-label="`\$\{t\('common\.open'\)\}: \$\{f\.filename\}`"/)
  assert.match(source, /focus-visible:ring-2/)
  assert.doesNotMatch(source, /<img[\s\S]*:alt="f\.filename"[\s\S]*@click\.stop="openAttachmentImage\(f\)"/)
})
