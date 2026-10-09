import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

const read = (path: string) => readFileSync(resolve(import.meta.dir, path), 'utf8')

test('attachment rows use the UI vocabulary instead of a hardcoded English title', () => {
  const en = read('../src/i18n/messages/en-US.ts')
  const zh = read('../src/i18n/messages/zh-CN.ts')
  assert.ok(en.includes("rowTitle: 'Attachment',"))
  assert.ok(zh.includes("rowTitle: '附件',"))

  const chatPage = read('../src/pages/ChatPage.vue')
  const messageList = read('../src/components/chat/MessageList.vue')
  // Part rows the runtime did not present itself fall back to UI copy.
  assert.ok(chatPage.includes("attachment: String(t('chat.attachments.rowTitle')),"))
  assert.ok(chatPage.includes("operation: String(t('chat.partTitles.operation')),"))
  assert.ok(chatPage.includes("reasoning: String(t('chat.partTitles.reasoning')),"))
  assert.ok(chatPage.includes("responseFailed: String(t('chat.partTitles.responseFailed')),"))
  assert.ok(chatPage.includes("attachmentTitle: String(t('chat.attachments.rowTitle')).trim()"))
  assert.ok(messageList.includes("attachmentTitle: String(t('chat.attachments.rowTitle')).trim()"))

  const projection = read('../src/pages/chat/transcriptProjection.ts')
  assert.ok(projection.includes('export type TranscriptProjectionLabels'))
  assert.ok(projection.includes("defaultExpanded: kind === 'answer' || kind === 'text'"))
  // The derived titles are UI copy, never a hardcoded English label.
  assert.ok(projection.includes("text(presentationLabels?.answer) || 'Answer'"))
  assert.ok(projection.includes("text(presentationLabels?.reasoning) || 'thinking'"))
  assert.ok(projection.includes("text(labels?.compaction) || 'Compaction'"))
})
