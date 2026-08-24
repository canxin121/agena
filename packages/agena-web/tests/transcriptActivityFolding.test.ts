import assert from 'node:assert/strict'
import test from 'node:test'

import { foldTranscriptActivityRun, transcriptActivityRunKey } from '../src/pages/chat/transcriptActivityFolding'

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

  assert.equal(transcriptActivityRunKey('message-1', initial, 0), 'activity-summary:message-1:10')
  assert.equal(
    transcriptActivityRunKey('message-1', afterExpansion, 0),
    transcriptActivityRunKey('message-1', initial, 0),
  )
})
