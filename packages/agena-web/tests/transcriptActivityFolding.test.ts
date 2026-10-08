import assert from 'node:assert/strict'
import test from 'node:test'

import {
  foldTranscriptActivityRun,
  foldTranscriptReply,
  preserveActivityVisibility,
  transcriptActivityRunKey,
} from '../src/pages/chat/transcriptActivityFolding'

test('activity run folding depends only on part count', () => {
  const parts = Array.from({ length: 9 }, (_, index) => ({ id: index + 1, expanded: index < 4 }))

  const collapsed = foldTranscriptActivityRun(parts)
  assert.equal(collapsed.hiddenCount, 4)
  assert.deepEqual(
    collapsed.visibleParts.map((part) => part.id),
    [5, 6, 7, 8, 9],
  )

  const progressive = foldTranscriptActivityRun(parts, 7)
  assert.equal(progressive.hiddenCount, 2)
  assert.deepEqual(
    progressive.visibleParts.map((part) => part.id),
    [3, 4, 5, 6, 7, 8, 9],
  )

  const all = foldTranscriptActivityRun(parts, Number.MAX_SAFE_INTEGER)
  assert.equal(all.hiddenCount, 0)
  assert.deepEqual(
    all.visibleParts.map((part) => part.id),
    [1, 2, 3, 4, 5, 6, 7, 8, 9],
  )
})

test('activity run visibility key survives prepending older pages', () => {
  const initial = [{ id: 6 }, { id: 7 }, { id: 8 }, { id: 9 }, { id: 10 }]
  const afterExpansion = [{ id: 1 }, { id: 2 }, { id: 3 }, { id: 4 }, { id: 5 }, ...initial]

  assert.equal(transcriptActivityRunKey('message-1', initial, 0), 'activity-summary:message-1')
  assert.equal(
    transcriptActivityRunKey('message-1', afterExpansion, 0),
    transcriptActivityRunKey('message-1', initial, 0),
  )
})

test('a filtered reasoning anchor still exposes the remote reply loading control', () => {
  const parts = [7, 8, 9, 10].map((id) => ({ id: String(id), kind: 'answer' }))
  const fold = { runId: 3, runIds: [3], anchorPartId: '6', hiddenCount: 4, nextCursor: 'before-6' }
  const result = foldTranscriptReply(parts, [fold])
  assert.deepEqual(result, { visibleParts: parts, hiddenCount: 4, fold })
})

test('streaming appends preserve revealed rows and lifecycle chrome consumes no part budget', () => {
  const before = Array.from({ length: 10 }, (_, i) => String(i + 1))
  const after = [...before, '11']
  const visible = preserveActivityVisibility(after, before, 10)
  const parts = [...after.map((id) => ({ id, kind: 'operation' })), { id: 'lifecycle:3', kind: 'lifecycle' }]
  assert.equal(visible, 11)
  assert.equal(
    transcriptActivityRunKey(
      '3',
      before.map((id) => ({ id })),
      0,
    ),
    transcriptActivityRunKey('3', parts, 0),
  )
  assert.deepEqual(foldTranscriptReply(parts, [], visible).visibleParts, parts)
  assert.equal(preserveActivityVisibility(['0', ...before], before, 5), 5)
})

test('an unanswered interaction remains visible ahead of newer siblings', () => {
  const parts = Array.from({ length: 12 }, (_, i) => ({ id: String(i), kind: 'operation', pending: i === 2 }))
  const folded = foldTranscriptReply(parts, [], 5, (part) => part.pending)
  assert.equal(folded.hiddenCount, 2)
  assert.deepEqual(folded.visibleParts, parts.slice(2))
  const answered = parts.map((part) => ({ ...part, pending: false }))
  assert.equal(foldTranscriptReply(answered, [], 5, (part) => part.pending).hiddenCount, 7)
})
