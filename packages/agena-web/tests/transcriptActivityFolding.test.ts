import assert from 'node:assert/strict'
import test from 'node:test'

import { foldTranscriptActivityRun } from '../src/pages/chat/transcriptActivityFolding'

test('activity run folding depends only on part count', () => {
  const parts = Array.from({ length: 9 }, (_, index) => ({ id: index + 1, expanded: index < 4 }))

  const collapsed = foldTranscriptActivityRun(parts, false, 5)
  assert.equal(collapsed.hiddenCount, 4)
  assert.deepEqual(
    collapsed.visibleParts.map((part) => part.id),
    [5, 6, 7, 8, 9],
  )

  const expanded = foldTranscriptActivityRun(parts, true, 5)
  assert.equal(expanded.hiddenCount, 4)
  assert.deepEqual(
    expanded.visibleParts.map((part) => part.id),
    [1, 2, 3, 4, 5, 6, 7, 8, 9],
  )
})
