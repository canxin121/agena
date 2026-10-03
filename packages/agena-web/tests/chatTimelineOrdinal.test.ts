import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

import { timelineCurrentOrdinal, timelineTotal, type NavigableMessage } from '../src/pages/chat/timelineOrdinal'

/** The loaded window: a suffix of the conversation with server ordinals. */
function windowed(ordinals: Array<number | null>): NavigableMessage[] {
  return ordinals.map((ordinal, index) => ({ id: String(index + 1), ordinal }))
}

function source(relative: string): string {
  return readFileSync(resolve(import.meta.dir, relative), 'utf8')
}

test('opening a long session shows the true position, not the window position', () => {
  // 40 user messages exist; the first page loads only the newest one.
  const entries = windowed([40])
  const total = timelineTotal(entries, 40)

  assert.equal(total, 40)
  assert.equal(timelineCurrentOrdinal(entries, 0, total), 40, 'the last message is number 40, not number 1')
})

test('the ordinal is used verbatim instead of the window position', () => {
  const entries = windowed([7, 8, 9])
  const total = timelineTotal(entries, 12)

  assert.equal(total, 12)
  assert.equal(timelineCurrentOrdinal(entries, 0, total), 7)
  assert.equal(timelineCurrentOrdinal(entries, 1, total), 8)
  assert.equal(timelineCurrentOrdinal(entries, 2, total), 9)
})

test('paging older history never renumbers the messages already on screen', () => {
  const newestPage = windowed([40])
  const beforePaging = timelineCurrentOrdinal(newestPage, 0, timelineTotal(newestPage, 40))

  const afterOlderPage = windowed([38, 39, 40])
  const total = timelineTotal(afterOlderPage, 40)

  assert.equal(beforePaging, 40)
  assert.equal(timelineCurrentOrdinal(afterOlderPage, 2, total), 40, 'the same message keeps its number')
  assert.equal(timelineCurrentOrdinal(afterOlderPage, 0, total), 38)
})

test('messages without an ordinal fall back to the total-derived position', () => {
  const entries = windowed([null, 39, 40])
  const total = timelineTotal(entries, 40)

  assert.equal(timelineCurrentOrdinal(entries, 0, total), 38)
  assert.equal(timelineCurrentOrdinal(entries, 1, total), 39)
  assert.equal(timelineCurrentOrdinal(entries, 2, total), 40)
})

test('a live-delivered ordinal raises the total but never lowers it', () => {
  assert.equal(timelineTotal(windowed([41]), 40), 41, "the newest user message's ordinal IS the total")
  assert.equal(timelineTotal(windowed([41]), 42), 42, 'the server count stays authoritative')
})

test('the total never reports fewer messages than are loaded', () => {
  assert.equal(timelineTotal(windowed([1, 2, 3]), null), 3, 'no server count yet')
  assert.equal(timelineTotal(windowed([1, 2, 3]), 2), 3, 'a stale count cannot hide loaded messages')
})

test('an empty window has no current position', () => {
  assert.equal(timelineTotal([], 0), 0)
  assert.equal(timelineCurrentOrdinal([], 0, 0), 0)
})

test('out-of-range and non-positive input is clamped instead of overflowing', () => {
  const entries = windowed([5])
  assert.equal(timelineCurrentOrdinal(entries, 9, 9), 5)
  assert.equal(timelineCurrentOrdinal(entries, -3, 9), 5)
  assert.equal(timelineTotal(windowed([null]), -4), 1)
  assert.equal(timelineCurrentOrdinal(windowed([null]), 0, -4), 1)
})

test('the navigator renders the absolute ordinal end to end', () => {
  const view = source('../src/pages/chat/ChatPageView.vue')
  const nav = source('../src/pages/chat/useChatScrollNav.ts')
  const api = source('../src/stores/chat/api.ts')
  const store = source('../src/stores/chat.ts')

  assert.match(view, /\{\{ navCurrentOrdinal \}\} \/ \{\{ navTotalLabel \}\}/)
  assert.doesNotMatch(view, /navIndex \+ 1/, 'the window index must never be displayed as the message number')
  assert.match(nav, /timelineCurrentOrdinal\(navigableMessages\.value, navIndex\.value, navTotal\.value\)/)
  assert.match(nav, /timelineTotal\(navigableMessages\.value, chat\.selectedHistory\.userMessageCount\)/)
  assert.match(api, /user_message_ordinal/, 'the transcript projection carries the server ordinal')
  assert.match(api, /info\.userMessageOrdinal = Math\.floor\(ordinalRaw\)/)
  assert.match(store, /user_message_ordinal/, 'live part events carry the server ordinal too')
  assert.match(store, /info\.userMessageOrdinal = Math\.floor\(ordinalRaw\)/)
})
