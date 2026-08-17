import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('chat opens with one recent page and delegates older loading to user scroll', () => {
  const store = readFileSync(resolve(import.meta.dir, '../src/stores/chat.ts'), 'utf8')
  const navigation = readFileSync(resolve(import.meta.dir, '../src/pages/chat/useChatScrollNav.ts'), 'utf8')
  const view = readFileSync(resolve(import.meta.dir, '../src/pages/chat/ChatPageView.vue'), 'utf8')
  const messageList = readFileSync(resolve(import.meta.dir, '../src/components/chat/MessageList.vue'), 'utf8')
  const chatPage = readFileSync(resolve(import.meta.dir, '../src/pages/ChatPage.vue'), 'utf8')

  assert.match(store, /const MESSAGE_PAGE_SIZE = 50/)
  assert.doesNotMatch(navigation, /maxAutoPages|ensureInitialHistoryScrollable/)
  assert.match(view, /@wheel="handleWheel"/)
  assert.match(store, /historyOlderLoadedBySession/)
  assert.match(store, /transcriptCacheGeneration/)
  assert.match(store, /OLDER_MESSAGE_PAGE_SIZE = 200/)
  assert.match(store, /MAX_FOLD_SKIPPED_OLDER_PARTS/)
  assert.match(store, /foldedMessageCount/)
  assert.match(store, /messagePartCount/)
  assert.match(messageList, /data-transcript-expand-all/)
  assert.match(chatPage, /function expandAllTranscriptParts/)
})
