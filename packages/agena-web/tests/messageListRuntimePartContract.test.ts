import { expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'

const messageList = readFileSync(new URL('../src/components/chat/MessageList.vue', import.meta.url), 'utf8')
const chatPageView = readFileSync(new URL('../src/pages/chat/ChatPageView.vue', import.meta.url), 'utf8')

test('compact and provider retry statuses render through transcript Parts, not composer status text', () => {
  expect(messageList).toContain("agenaKind: 'compaction'")
  expect(messageList).toContain("partState: 'in_progress'")
  expect(messageList).toContain("agenaKind: 'notice'")
  expect(messageList).toContain('AgenaTranscriptPart')
  expect(chatPageView).toContain(':is-compacting="compactingSessionId === chat.selectedSessionId"')
  expect(chatPageView).not.toContain(':retry="retryStatus"')
  expect(chatPageView).not.toContain(':countdown="retryCountdownLabel"')
})
