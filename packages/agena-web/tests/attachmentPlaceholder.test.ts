import assert from 'node:assert/strict'
import test from 'node:test'

import { ATTACHMENT_PLACEHOLDER_MAX_CHARS, attachmentPlaceholder } from '../src/pages/chat/attachmentPlaceholder'

test('an attachment placeholder keeps the composer chip label in brackets', () => {
  assert.equal(attachmentPlaceholder('image.png'), '[image.png]')
  assert.equal(attachmentPlaceholder('  report.pdf  '), '[report.pdf]')
})

test('a long pasted-text preview is truncated by code point', () => {
  const placeholder = attachmentPlaceholder('あ'.repeat(40))
  assert.equal(Array.from(placeholder).length, ATTACHMENT_PLACEHOLDER_MAX_CHARS + 3)
  assert.ok(placeholder.startsWith(`[${'あ'.repeat(ATTACHMENT_PLACEHOLDER_MAX_CHARS)}`))
  assert.ok(placeholder.endsWith('…]'))
})

test('an unlabeled attachment never leaves a bare placeholder in the text', () => {
  assert.equal(attachmentPlaceholder('', ''), '')
  assert.equal(attachmentPlaceholder('   ', '   '), '')
  assert.equal(attachmentPlaceholder('', 'attachment'), '[attachment]')
})
