import assert from 'node:assert/strict'
import test from 'node:test'

import type { TranscriptDisplayPart } from '../src/components/chat/messageList.types'
import { resolveTranscriptPageTarget, transcriptPartNavigationText } from '../src/pages/chat/transcriptNavigation'

function part(overrides: Partial<TranscriptDisplayPart> = {}): TranscriptDisplayPart {
  return {
    key: 'part:1',
    id: '1',
    kind: 'operation',
    status: 'completed',
    role: 'assistant',
    source: {},
    title: 'fs.read',
    summary: 'README.md',
    copyText: 'line 1\nline 2\nline 3',
    toggleable: true,
    defaultExpanded: false,
    ...overrides,
  }
}

test('collapsed parts expose one visible navigation line instead of hidden body lines', () => {
  assert.equal(transcriptPartNavigationText(part(), false), 'fs.read · README.md')
  assert.equal(
    transcriptPartNavigationText(part({ title: 'fs.read\noperation', summary: 'README.md\n42 lines' }), false),
    'fs.read operation · README.md 42 lines',
  )
  assert.equal(transcriptPartNavigationText(part(), true), 'line 1\nline 2\nline 3')
})

test('page movement clamps to the transcript boundary', () => {
  assert.deepEqual(
    resolveTranscriptPageTarget({
      scrollTop: 850,
      clientHeight: 400,
      scrollHeight: 1_000,
      direction: 'down',
      half: false,
    }),
    { top: 600, boundary: 'end' },
  )
  assert.deepEqual(
    resolveTranscriptPageTarget({
      scrollTop: 40,
      clientHeight: 400,
      scrollHeight: 1_000,
      direction: 'up',
      half: true,
    }),
    { top: 0, boundary: 'start' },
  )
})
