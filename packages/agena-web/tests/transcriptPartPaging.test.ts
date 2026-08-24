import assert from 'node:assert/strict'
import test from 'node:test'

import {
  DEFAULT_TRANSCRIPT_PART_PAGE_SIZE,
  MAX_TRANSCRIPT_PART_PAGE_SIZE,
  MIN_TRANSCRIPT_PART_PAGE_SIZE,
  normalizeTranscriptPartPageSize,
} from '../src/pages/chat/transcriptPartPaging'

test('transcript part page size is clamped to the server-supported range', () => {
  assert.equal(normalizeTranscriptPartPageSize(undefined), DEFAULT_TRANSCRIPT_PART_PAGE_SIZE)
  assert.equal(normalizeTranscriptPartPageSize(0), MIN_TRANSCRIPT_PART_PAGE_SIZE)
  assert.equal(normalizeTranscriptPartPageSize(12.9), 12)
  assert.equal(normalizeTranscriptPartPageSize(999), MAX_TRANSCRIPT_PART_PAGE_SIZE)
})
