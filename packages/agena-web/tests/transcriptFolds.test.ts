import assert from 'node:assert/strict'
import test from 'node:test'
import type { MessageEntry, MessageFold } from '../src/types/chat'
import { reconcileTranscriptFolds } from '../src/stores/chat/transcriptFolds'

const fold = (anchor: number, hiddenCount: number, runId = 3): MessageFold => ({
  runId,
  runIds: [3, 20],
  anchorPartId: String(anchor),
  hiddenCount,
  nextCursor: `before-${anchor}`,
})
const entry = (runId: number, ids: number[], folds: MessageFold[] = []): MessageEntry => ({
  info: { id: String(runId), sessionID: '1', role: 'assistant', runId },
  parts: ids.map((id) => ({ id: String(id), type: 'text', text: String(id), agenaKind: 'text' })),
  folds,
})

test('refresh retains revealed parts and the older cursor across assistant runs', () => {
  const previous = [entry(3, [14, 15, 16, 17, 18], [fold(14, 10)]), entry(20, [21, 22, 23, 24, 25])]
  const snapshot = [entry(3, []), entry(20, [21, 22, 23, 24, 25], [fold(21, 15, 20)])]
  const result = reconcileTranscriptFolds(previous, snapshot, previous)
  assert.deepEqual(result[0]?.folds, [fold(14, 10)])
  assert.deepEqual(result[1]?.folds, [])
  assert.equal(result.flatMap((message) => message.parts).length, 10)
})

test('a streaming gap uses the new cursor without forgetting an older loaded prefix', () => {
  const previous = [entry(3, [14, 15, 16, 17, 18], [fold(14, 10)]), entry(20, [21, 22, 23, 24, 25])]
  const snapshot = [entry(3, []), entry(20, [31, 32, 33, 34, 35], [fold(31, 30, 20)])]
  const merged = [previous[0]!, entry(20, [21, 22, 23, 24, 25, 31, 32, 33, 34, 35])]
  const result = reconcileTranscriptFolds(merged, snapshot, previous)
  assert.deepEqual(result[1]?.folds, [fold(31, 20, 20)])
  assert.deepEqual(result[0]?.folds, [])
})

test('a complete cached reply retires its server fold, preserving unrelated older folds', () => {
  const older = entry(1, [2], [{ ...fold(2, 1, 1), runIds: [1] }])
  const current = entry(3, [4, 5, 6, 7, 8, 9, 10])
  const snapshot = [entry(3, [6, 7, 8, 9, 10], [fold(6, 2)])]
  const result = reconcileTranscriptFolds([older, current], snapshot, [older, current])
  assert.deepEqual(result[0]?.folds, older.folds)
  assert.deepEqual(result[1]?.folds, [])
})

test('an unchanged fold list keeps the entry object the transcript already rendered', () => {
  const stable = entry(1, [2], [{ ...fold(2, 1, 1), runIds: [1] }])
  const current = entry(3, [6, 7])
  const snapshot = [entry(3, [6, 7])]
  const result = reconcileTranscriptFolds([stable, current], snapshot, [stable, current])
  assert.equal(result[0], stable)
  assert.equal(result[1], current)
  assert.deepEqual(result[0]?.folds, stable.folds)
  assert.deepEqual(result[1]?.folds, [])
})

test('a new streaming gap still pushes onto the retained entry and reconciles its prefix', () => {
  const stable = entry(1, [2], [{ ...fold(2, 1, 1), runIds: [1] }])
  const current = entry(3, [6, 7])
  const incoming = entry(3, [6, 7], [fold(7, 4)])
  const result = reconcileTranscriptFolds([stable, current], [incoming], [stable, current])
  assert.equal(result[0], stable)
  assert.equal(result[1], current)
  assert.deepEqual(result[1]?.folds, [
    { runId: 3, runIds: [3, 20], anchorPartId: '7', hiddenCount: 3, nextCursor: 'before-7' },
  ])
})
