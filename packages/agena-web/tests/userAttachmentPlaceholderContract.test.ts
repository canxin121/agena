import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

const read = (path: string) => readFileSync(resolve(import.meta.dir, path), 'utf8')

test('a submitted user message keeps a placeholder per attachment and groups the attachments', () => {
  const page = read('../src/pages/ChatPage.vue')
  const placeholder = read('../src/pages/chat/attachmentPlaceholder.ts')

  assert.ok(placeholder.includes('export function attachmentPlaceholder'))
  assert.ok(page.includes("import { attachmentPlaceholder } from './chat/attachmentPlaceholder'"))
  assert.ok(page.includes('function composeOutgoingText('))
  assert.ok(page.includes('attachmentPlaceholder(file.pastePreview ?? file.filename, fileFallbackLabel)'))
  // One text part carries the whole body and the attachments follow as parts,
  // instead of splitting the text around each inline attachment.
  assert.ok(page.includes("if (composed.typedText) parts.push({ type: 'text', text: composed.text })"))
  assert.ok(page.includes('for (const id of composed.attachmentIds) {'))
  assert.ok(page.includes("text: composed.typedText ? composed.text : ''"))
  assert.equal(page.includes('const orderedSegments: ComposerSegment[]'), false)
})

test('the transcript shows a user message attachments above its text', () => {
  const item = read('../src/components/chat/MessageItem.vue')
  const projection = read('../src/pages/chat/transcriptProjection.ts')

  assert.ok(projection.includes('export function userInputOrder(parts: TranscriptDisplayPart[])'))
  assert.ok(item.includes('userInputOrder(props.displayParts)'))
})
