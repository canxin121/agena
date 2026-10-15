import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'
import en from '../src/i18n/messages/en-US'
import zh from '../src/i18n/messages/zh-CN'
import { projectLocalPart } from '../src/pages/chat/transcriptProjection'
import type { MessagePartLike } from '../src/components/chat/messageList.types'

test('attachment rows use the current UI language while runtime titles take precedence', () => {
  const chatPage = readFileSync(new URL('../src/pages/ChatPage.vue', import.meta.url), 'utf8')
  // ChatPage supplies the current UI vocabulary to the shared projection.
  assert.ok(chatPage.includes("attachment: String(t('chat.attachments.rowTitle')),"))
  const source: MessagePartLike = {
    id: '2',
    agenaKind: 'file_ref',
    agenaRole: 'user',
    partState: 'completed',
    agenaContent: { path: 'uploads/report.pdf', name: 'report.pdf', mime: 'application/pdf' },
  }
  for (const [messages, title] of [
    [en, 'Attachment'],
    [zh, '附件'],
  ] as const) {
    const labels = { attachment: messages.chat.attachments.rowTitle }
    assert.equal(labels.attachment, title)
    const projected = projectLocalPart(source, 'user', labels)
    assert.equal(projected.kind, 'resource')
    assert.equal(projected.title, title)
    assert.equal(projected.defaultExpanded, false)
    assert.match(projected.summary, /report\.pdf/)
    assert.equal(
      projectLocalPart({ ...source, agenaPresentation: { title: 'Shared report' } }, 'user', labels).title,
      'Shared report',
    )
  }
})
