import assert from 'node:assert/strict'
import test from 'node:test'

import { ATTACHMENT_PLACEHOLDER_MAX_CHARS, attachmentPlaceholder } from '../src/pages/chat/attachmentPlaceholder'

test('an attachment placeholder keeps the composer chip label in brackets', () => {
  assert.equal(attachmentPlaceholder('image.png'), '[image.png]')
  assert.equal(attachmentPlaceholder('  report.pdf  '), '[report.pdf]')
})

test('the ordinal leads the label so equal names stay distinguishable', () => {
  assert.equal(attachmentPlaceholder('image.png', { ordinal: 2 }), '[#2 image.png]')
  assert.equal(attachmentPlaceholder('image.png', { ordinal: 12 }), '[#12 image.png]')
  // The number survives the label's own truncation.
  const long = attachmentPlaceholder('あ'.repeat(40), { ordinal: 3 })
  assert.ok(long.startsWith('[#3 '))
  assert.equal(Array.from(long).length, ATTACHMENT_PLACEHOLDER_MAX_CHARS + 6)
})

test('a long pasted-text preview is truncated by code point', () => {
  const placeholder = attachmentPlaceholder('あ'.repeat(40))
  assert.equal(Array.from(placeholder).length, ATTACHMENT_PLACEHOLDER_MAX_CHARS + 3)
  assert.ok(placeholder.startsWith(`[${'あ'.repeat(ATTACHMENT_PLACEHOLDER_MAX_CHARS)}`))
  assert.ok(placeholder.endsWith('…]'))
})

test('an unlabeled attachment never leaves a bare placeholder in the text', () => {
  assert.equal(attachmentPlaceholder('', {}), '')
  assert.equal(attachmentPlaceholder('   ', { fallback: '   ' }), '')
  assert.equal(attachmentPlaceholder('', { fallback: 'attachment' }), '[attachment]')
  // With no usable label the number is the only thing that still tells two
  // unnamed attachments apart.
  assert.equal(attachmentPlaceholder('', { ordinal: 4 }), '[#4]')
})
