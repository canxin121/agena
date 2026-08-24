import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('chat opens with one recent page and delegates older loading to user scroll', () => {
  const store = readFileSync(resolve(import.meta.dir, '../src/stores/chat.ts'), 'utf8')
  const navigation = readFileSync(resolve(import.meta.dir, '../src/pages/chat/useChatScrollNav.ts'), 'utf8')
  const view = readFileSync(resolve(import.meta.dir, '../src/pages/chat/ChatPageView.vue'), 'utf8')
  const messageList = readFileSync(resolve(import.meta.dir, '../src/components/chat/MessageList.vue'), 'utf8')
  const messageItem = readFileSync(resolve(import.meta.dir, '../src/components/chat/MessageItem.vue'), 'utf8')
  const chatPage = readFileSync(resolve(import.meta.dir, '../src/pages/ChatPage.vue'), 'utf8')

  assert.match(store, /const MESSAGE_PAGE_SIZE = 2/)
  assert.doesNotMatch(navigation, /maxAutoPages|ensureInitialHistoryScrollable/)
  assert.match(view, /@wheel="handleWheel"/)
  assert.match(store, /historyOlderLoadedBySession/)
  assert.match(store, /transcriptCacheGeneration/)
  assert.match(store, /const MESSAGE_PAGE_SIZE = 2/)
  assert.doesNotMatch(store, /pruneSessionMessages/)
  assert.doesNotMatch(store, /loadAllMessages/)
  assert.match(store, /userMessageCount/)
  assert.match(store, /transcriptPartPageSize/)
  assert.match(store, /loadFoldedActivity/)
  assert.match(messageItem, /data-part-controls/)
  assert.match(messageItem, /data-part-expand-next/)
  assert.match(messageItem, /data-part-collect-all/)
  assert.match(messageItem, /id: 'expand-all'/)
  assert.match(messageItem, /id: 'collapse-all'/)
  assert.match(messageItem, /<OptionMenu/)
  assert.doesNotMatch(messageList, /data-part-controls/)
  assert.doesNotMatch(messageList, /loadAllHistory/)
  assert.match(chatPage, /function expandAllTranscriptParts/)
  assert.match(chatPage, /function collapseAllTranscriptParts/)
  assert.doesNotMatch(chatPage, /function expandNextTranscriptParts/)
  assert.doesNotMatch(chatPage, /function collectAllTranscriptParts/)
})
