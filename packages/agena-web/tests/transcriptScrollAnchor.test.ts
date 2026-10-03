import assert from 'node:assert/strict'
import test from 'node:test'
import {
  captureTranscriptScrollAnchor,
  restoreTranscriptScrollAnchor,
} from '../src/composables/chat/transcriptScrollAnchor'

test('prepending preserves a visible part even when the reply tail grows and the user keeps scrolling', () => {
  let inserted = 0
  const scroller = {
    scrollTop: 300,
    scrollHeight: 1800,
    clientHeight: 600,
    getBoundingClientRect: () => ({ top: 40 }),
    contains: (element: unknown) => element === leaf,
    querySelectorAll: () => [reply, leaf],
  }
  const leaf = {
    querySelector: () => null,
    getBoundingClientRect: () => ({
      top: 350 + inserted - scroller.scrollTop,
      bottom: 550 + inserted - scroller.scrollTop,
    }),
  }
  const reply = {
    querySelector: () => leaf,
    getBoundingClientRect: () => ({ top: -200, bottom: 1600 }),
  }
  const anchor = captureTranscriptScrollAnchor(scroller as unknown as HTMLElement)
  assert.equal(anchor.element, leaf as unknown as HTMLElement)
  inserted = 400
  scroller.scrollHeight += 700 // 400 prepended + 300 streamed at the tail
  scroller.scrollTop -= 50 // continued upward gesture during the request
  restoreTranscriptScrollAnchor(scroller as unknown as HTMLElement, anchor)
  assert.equal(scroller.scrollTop, 650)
})

test('missing anchors fall back to height compensation', () => {
  const scroller = { scrollTop: 12, scrollHeight: 900, contains: () => false }
  restoreTranscriptScrollAnchor(scroller as unknown as HTMLElement, { element: null, top: 12, height: 500, offset: 0 })
  assert.equal(scroller.scrollTop, 412)
})
