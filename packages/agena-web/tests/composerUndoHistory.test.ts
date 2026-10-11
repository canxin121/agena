import assert from 'node:assert/strict'
import test from 'node:test'

import type { ComposerSegment } from '../src/pages/chat/composerInput'
import {
  COMPOSER_HISTORY_COALESCE_MS,
  COMPOSER_HISTORY_LIMIT,
  composerAttachmentIds,
  createComposerHistory,
  recordComposerHistoryStep,
  redoComposerHistory,
  resolveComposerHistoryCommand,
  restorableComposerSegments,
  undoComposerHistory,
} from '../src/pages/chat/composerUndoHistory'

const text = (value: string): ComposerSegment[] => [{ type: 'text', text: value }]
const withAttachment = (value: string, id: string): ComposerSegment[] => [
  { type: 'text', text: value },
  { type: 'attachment', id },
]

function record(
  state: ReturnType<typeof createComposerHistory>,
  segments: ComposerSegment[],
  at: number,
  coalesce = true,
) {
  return recordComposerHistoryStep(state, segments, { coalesce, at })
}

function undoSegments(state: ReturnType<typeof createComposerHistory>): ComposerSegment[] {
  return undoComposerHistory(state)?.segments ?? []
}

test('one typing burst undoes back to the text before it', () => {
  let history = createComposerHistory()
  history = record(history, text('a'), 1000)
  history = record(history, text('ab'), 1050)
  history = record(history, text('abc'), 1100)

  assert.deepEqual(undoSegments(history), [])
  assert.equal(undoComposerHistory(history)?.state.index, 0)
})

test('typing after the coalesce window becomes its own step', () => {
  let history = createComposerHistory(text('seed'))
  history = record(history, text('seed a'), 1000)
  history = record(history, text('seed ab'), 1000 + COMPOSER_HISTORY_COALESCE_MS + 1)

  const step = undoComposerHistory(history)
  assert.deepEqual(step?.segments, text('seed a'))
  assert.deepEqual(undoComposerHistory(step!.state)?.segments, text('seed'))
})

test('structural edits start a new step even inside the coalesce window', () => {
  let history = createComposerHistory(text('keep'))
  history = record(history, text('keep me'), 1000)
  // An attachment chip changes the document structurally, so it must not merge
  // with the typing that came just before it.
  history = record(history, withAttachment('keep me', 'file-1'), 1010, false)

  assert.deepEqual(undoSegments(history), text('keep me'))
  assert.deepEqual(undoComposerHistory(undoComposerHistory(history)!.state)?.segments, text('keep'))
})

test('typing after an undo drops the redo branch', () => {
  let history = createComposerHistory(text('one'))
  history = record(history, text('one two'), 1000, false)
  history = record(history, text('one two three'), 2000, false)

  const undone = undoComposerHistory(history)
  assert.equal(redoComposerHistory(undone!.state)?.segments.length, 1)

  const edited = record(undone!.state, text('one two!'), 3000, false)
  assert.equal(redoComposerHistory(edited), null)
  assert.deepEqual(undoSegments(edited), text('one two'))
})

test('an edit that leaves the document unchanged adds no step', () => {
  const history = record(createComposerHistory(text('same')), text('same'), 1000)
  assert.equal(history.entries.length, 1)
  assert.equal(undoComposerHistory(history), null)
})

test('attachment segments survive undo and redo', () => {
  let history = createComposerHistory()
  history = record(history, text('caption '), 1000, false)
  history = record(history, withAttachment('caption ', 'file-9'), 1010, false)

  const undone = undoComposerHistory(history)
  assert.deepEqual(undone?.segments, text('caption '))
  assert.deepEqual(redoComposerHistory(undone!.state)?.segments, withAttachment('caption ', 'file-9'))
})

test('two composer instances keep independent histories', () => {
  let left = createComposerHistory()
  let right = createComposerHistory()

  right = record(right, text('right pane'), 1000)
  left = record(left, text('left pane'), 1050)

  assert.deepEqual(undoSegments(left), [])
  // Undoing the left composer must leave the right composer's document alone.
  assert.deepEqual(right.entries[right.index], text('right pane'))
  assert.deepEqual(undoSegments(right), [])
})

test('history stays bounded', () => {
  let history = createComposerHistory()
  for (let step = 1; step <= COMPOSER_HISTORY_LIMIT + 5; step += 1) {
    history = record(history, text(`line ${step}`), step * 10, false)
  }

  assert.equal(history.entries.length, COMPOSER_HISTORY_LIMIT)
  assert.equal(history.index, COMPOSER_HISTORY_LIMIT - 1)
  assert.deepEqual(history.entries[history.index], text(`line ${COMPOSER_HISTORY_LIMIT + 5}`))
})

test('undo and redo boundaries are inert', () => {
  const history = createComposerHistory(text('only'))
  assert.equal(undoComposerHistory(history), null)
  assert.equal(redoComposerHistory(history), null)
})

test('a snapshot restores its text and the attachments the draft still holds', () => {
  const snapshot: ComposerSegment[] = [
    { type: 'text', text: 'before ' },
    { type: 'attachment', id: 'gone' },
    { type: 'text', text: ' after ' },
    { type: 'attachment', id: 'kept' },
  ]

  assert.deepEqual(restorableComposerSegments(snapshot, new Set(['kept'])), [
    { type: 'text', text: 'before ' },
    { type: 'text', text: ' after ' },
    { type: 'attachment', id: 'kept' },
  ])
  assert.deepEqual(composerAttachmentIds(snapshot), new Set(['gone', 'kept']))
})

test('a repeated attachment chip is restored once', () => {
  const snapshot: ComposerSegment[] = [
    { type: 'attachment', id: 'file-1' },
    { type: 'attachment', id: 'file-1' },
  ]

  assert.deepEqual(restorableComposerSegments(snapshot, new Set(['file-1'])), [{ type: 'attachment', id: 'file-1' }])
})

for (const [name, event, expected] of [
  ['Cmd+Z undoes', { key: 'z', metaKey: true }, 'undo'],
  ['Ctrl+Z undoes off macOS', { key: 'z', ctrlKey: true }, 'undo'],
  ['Shift+Cmd+Z redoes', { key: 'z', metaKey: true, shiftKey: true }, 'redo'],
  ['Shift+Ctrl+Z redoes off macOS', { key: 'z', ctrlKey: true, shiftKey: true }, 'redo'],
  ['Ctrl+Y redoes', { key: 'y', ctrlKey: true }, 'redo'],
  ['Ctrl+Shift+Y is not redo', { key: 'y', ctrlKey: true, shiftKey: true }, null],
  ['Cmd+Y is not redo', { key: 'y', metaKey: true }, null],
  ['plain Z is not a history command', { key: 'z' }, null],
  ['Alt+Cmd+Z is not a history command', { key: 'z', metaKey: true, altKey: true }, null],
  ['Cmd+A is not a history command', { key: 'a', metaKey: true }, null],
] as const) {
  test(`history chord: ${name}`, () => {
    assert.equal(resolveComposerHistoryCommand(event), expected)
  })
}
