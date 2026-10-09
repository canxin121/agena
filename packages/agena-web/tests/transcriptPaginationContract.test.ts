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
  assert.match(messageItem, /data-part-page-size/)
  // The page-size number is edited inline inside the expand label instead of
  // rendering a separate input next to it.
  assert.match(messageItem, /data-part-expand-next="true"[\s\S]*?data-part-page-size="true"/)
  assert.match(messageItem, /data-part-page-size="true"[\s\S]*?data-part-collect-all/)
  assert.match(messageItem, /expandNextLead/)
  assert.match(messageItem, /expandNextTail/)
  assert.match(messageItem, /DEFAULT_TRANSCRIPT_PART_PAGE_SIZE/)
  assert.match(messageItem, /transcriptActivityRunKey/)
  assert.doesNotMatch(messageItem, /<OptionMenu/)
  assert.doesNotMatch(messageList, /data-part-controls/)
  assert.doesNotMatch(messageList, /loadAllHistory/)
  assert.doesNotMatch(chatPage, /function expandAllTranscriptParts/)
  assert.doesNotMatch(chatPage, /function collapseAllTranscriptParts/)
  assert.doesNotMatch(chatPage, /function expandNextTranscriptParts/)
  assert.doesNotMatch(chatPage, /function collectAllTranscriptParts/)
  // The recent page validates after a send even if its SSE notification was
  // lost; older pages remain lazy and use their explicit cursor.
  assert.match(store, /listMessages\(sid, limit, undefined, DEFAULT_TRANSCRIPT_PART_PAGE_SIZE, true\)/)
  assert.match(store, /listMessages\(sid, OLDER_MESSAGE_PAGE_SIZE, cursor, DEFAULT_TRANSCRIPT_PART_PAGE_SIZE\)/)
})

test('older-part controls load parts instead of showing them', () => {
  const zhLocale = readFileSync(resolve(import.meta.dir, '../src/i18n/messages/zh-CN.ts'), 'utf8')
  const enLocale = readFileSync(resolve(import.meta.dir, '../src/i18n/messages/en-US.ts'), 'utf8')

  assert.match(zhLocale, /expandNextLead: '再加载'/)
  assert.match(zhLocale, /expandNextTail: '个片段'/)
  assert.match(zhLocale, /collectAll: '加载全部片段'/)
  assert.match(zhLocale, /pageSizeInputLabel: '本次要多加载的片段数量（1–50）'/)
  assert.match(enLocale, /expandNextLead: 'Load'/)
  assert.match(enLocale, /collectAll: 'Load all parts'/)
})

test('the folded-activity row stays reachable and operable from the keyboard cursor', () => {
  const vim = readFileSync(resolve(import.meta.dir, '../src/pages/chat/useChatTranscriptVim.ts'), 'utf8')
  const messageItem = readFileSync(resolve(import.meta.dir, '../src/components/chat/MessageItem.vue'), 'utf8')

  // The "load more parts" row is a part row, so it has to stay in the cursor
  // model: excluding it left Up/Down unable to reach the row at all.
  assert.match(vim, /const parts = nodes\.filter\(\(element\) => element\.dataset\.transcriptNode === 'part'\)/)
  assert.doesNotMatch(vim, /dataset\.partKind !== 'activity_summary'/)
  // Chrome is excluded from the cursor's text projection, so the row and its
  // label must not be marked as chrome.
  assert.doesNotMatch(messageItem, /:data-transcript-chrome="row\.kind === 'summary'/)
  assert.doesNotMatch(messageItem, /data-part-controls="true"\s*\n\s*data-transcript-chrome="true"/)
  // Enter on the row clicks the load-more control inside it.
  assert.match(vim, /querySelector<HTMLButtonElement>\('\[data-transcript-toggle="true"\]'\)/)
  assert.match(messageItem, /data-part-expand-next="true"[\s\S]*?data-transcript-toggle="true"/)
})
