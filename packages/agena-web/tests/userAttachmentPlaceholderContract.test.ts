import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

const read = (path: string) => readFileSync(resolve(import.meta.dir, path), 'utf8')

test('a submitted user message keeps a numbered placeholder per attachment and groups the attachments', () => {
  const page = read('../src/pages/ChatPage.vue')
  const placeholder = read('../src/pages/chat/attachmentPlaceholder.ts')

  assert.ok(placeholder.includes('export function attachmentPlaceholder'))
  assert.ok(page.includes("import { attachmentPlaceholder } from './chat/attachmentPlaceholder'"))
  assert.ok(page.includes('function composeOutgoingText('))
  assert.ok(page.includes('attachmentPlaceholder(file.pastePreview ?? file.filename, {'))
  assert.ok(page.includes('ordinal: attachmentIds.length'))
  // One text part carries the whole body and the attachments follow as parts,
  // instead of splitting the text around each inline attachment.
  assert.ok(page.includes("if (composed.typedText) parts.push({ type: 'text', text: composed.text })"))
  assert.ok(page.includes('for (const id of composed.attachmentIds) {'))
  assert.equal(page.includes('const orderedSegments: ComposerSegment[]'), false)
})

test('the composer chip and the transcript rows carry the same attachment number as the text', () => {
  const item = read('../src/components/chat/MessageItem.vue')
  const part = read('../src/components/chat/AgenaTranscriptPart.vue')
  const composer = read('../src/components/chat/Composer.vue')
  const projection = read('../src/pages/chat/transcriptProjection.ts')

  assert.ok(projection.includes('export function userInputOrder(parts: TranscriptDisplayPart[])'))
  assert.ok(item.includes('userInputOrder(props.displayParts)'))
  assert.ok(item.includes("ordinal: part.kind === 'resource' ? ++attachmentOrdinal : undefined"))
  assert.ok(item.includes(':ordinal="row.ordinal"'))
  assert.ok(part.includes('data-part-ordinal="true"'))
  // The chip numbers itself by its position among the editor's chips, which is
  // the order the submitted text and the attachment parts use.
  assert.ok(composer.includes('function chipOrdinal(chip: HTMLSpanElement): number'))
  assert.ok(composer.includes('const label = ordinal > 0 ? `#${ordinal} ${baseLabel}` : baseLabel'))
  assert.ok(composer.includes('function refreshChipLabels()'))
})
