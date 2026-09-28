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
  assert.ok(chatPage.includes("labels: () => ({ attachment: String(t('chat.attachments.rowTitle')) })"))
  assert.ok(chatPage.includes("attachmentTitle: String(t('chat.attachments.rowTitle')).trim()"))
  assert.ok(messageList.includes("attachmentTitle: String(t('chat.attachments.rowTitle')).trim()"))

  const projection = read('../src/pages/chat/transcriptProjection.ts')
  assert.ok(projection.includes('export type TranscriptProjectionLabels'))
  assert.ok(projection.includes("role === 'user' && kind === 'resource'"))
})
