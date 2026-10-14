import { expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'

const messageList = readFileSync(new URL('../src/components/chat/MessageList.vue', import.meta.url), 'utf8')
const chatPageView = readFileSync(new URL('../src/pages/chat/ChatPageView.vue', import.meta.url), 'utf8')
const runUi = readFileSync(new URL('../src/pages/chat/useChatRunUi.ts', import.meta.url), 'utf8')

test('compaction uses the durable transcript while provider retry retains its runtime notice', () => {
  expect(messageList).not.toContain('compaction:active:')
  expect(messageList).not.toContain('compactionPart')
  expect(messageList).toContain("partState: 'in_progress'")
  expect(messageList).toContain("agenaKind: 'notice'")
  expect(messageList).toContain('AgenaTranscriptPart')
  expect(chatPageView).not.toContain(':is-compacting=')
  expect(runUi).toContain('canonicalRunActive.value || compacting?.value')
  expect(chatPageView).not.toContain(':retry="retryStatus"')
  expect(chatPageView).not.toContain(':countdown="retryCountdownLabel"')
})
