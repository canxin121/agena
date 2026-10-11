import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

function extractFunctionSource(source: string, signature: string): string {
  const start = source.indexOf(signature)
  assert.ok(start >= 0, `missing function signature: ${signature}`)

  const blockStart = source.indexOf('{', start)
  assert.ok(blockStart >= 0, `missing block start for: ${signature}`)

  let depth = 0
  for (let idx = blockStart; idx < source.length; idx += 1) {
    const character = source[idx]
    if (character === '{') depth += 1
    else if (character === '}') depth -= 1
    if (depth === 0) return source.slice(start, idx + 1)
  }

  throw new Error(`unterminated function block: ${signature}`)
}

const composer = readFileSync(resolve(import.meta.dir, '../src/components/chat/Composer.vue'), 'utf8')
const historyModel = readFileSync(resolve(import.meta.dir, '../src/pages/chat/composerUndoHistory.ts'), 'utf8')

// The composer is the only contenteditable in the app, and every workspace pane
// renders one of them into the same document.  A contenteditable that leaves
// undo to the browser therefore lets Cmd/Ctrl+Z in one pane revert the draft the
// other pane owns: the reproduction is two sessions side by side, typing in the
// right one, then clicking into the left one and undoing there.
test('the composer owns undo instead of the document-wide browser stack', () => {
  assert.ok(composer.includes("from '@/pages/chat/composerUndoHistory'"))
  assert.ok(composer.includes('@keydown="handleEditorKeydown"'))
  assert.ok(!composer.includes('@keydown="$emit(\'draftKeydown\', $event)"'))
  assert.ok(historyModel.includes('composer instance keeps undo local to the pane'))
})

test('history chords are claimed before the browser can apply its own undo', () => {
  const handler = extractFunctionSource(composer, 'function handleHistoryKeydown(event: KeyboardEvent): boolean')

  assert.ok(handler.includes('resolveComposerHistoryCommand(event)'))
  assert.ok(handler.includes('event.preventDefault()'))
  assert.ok(handler.includes('event.stopPropagation()'))
  assert.ok(handler.includes('undoComposerEdit()'))
  assert.ok(handler.includes('redoComposerEdit()'))
  // The chord stays claimed even with nothing to apply; falling through would
  // hand the keystroke to the shared stack.
  assert.ok(handler.indexOf('if (!command) return false') < handler.indexOf('event.preventDefault()'))
  assert.ok(handler.indexOf('event.preventDefault()') < handler.indexOf('undoComposerEdit()'))

  const beforeInput = extractFunctionSource(composer, 'function handleBeforeInput(event: InputEvent)')
  assert.ok(beforeInput.includes("event.inputType === 'historyUndo'"))
  assert.ok(beforeInput.includes("event.inputType === 'historyRedo'"))
})

test('undo and redo restore this composer document and nothing else', () => {
  const undo = extractFunctionSource(composer, 'function undoComposerEdit(): boolean')
  const redo = extractFunctionSource(composer, 'function redoComposerEdit(): boolean')
  const apply = extractFunctionSource(composer, 'function applyHistoryStep(step: ComposerHistoryStep | null): boolean')
  const record = extractFunctionSource(composer, 'function recordUndoHistory(coalesce: boolean)')

  assert.ok(undo.includes('undoComposerHistory(undoHistory)'))
  assert.ok(redo.includes('redoComposerHistory(undoHistory)'))
  assert.ok(apply.includes('restorableComposerSegments(step.segments, known)'))
  assert.ok(apply.includes('applySegments(segments)'))
  // Staged attachments follow the restored document, so a file the snapshot does
  // not contain leaves the draft with it.
  assert.ok(apply.includes("emit('removeAttachment', id)"))
  // Writing a snapshot back must not be recorded as a new step.
  assert.ok(apply.includes('applyingHistory = true'))
  assert.ok(record.includes('if (applyingHistory) return'))
  assert.ok(record.includes('readSegments()'))
})

test('every edit path feeds the local history', () => {
  const typing = extractFunctionSource(composer, 'function handleInput()')
  const replaceRange = extractFunctionSource(
    composer,
    'function replaceRange(\n  start: number,\n  end: number,\n  replacement: string,\n  selectionMode:',
  )
  const insertChip = extractFunctionSource(composer, 'function insertChip(id: string)')
  const removeChipIds = extractFunctionSource(composer, 'function removeChipIds(ids: Set<string>)')
  const replaceAttachment = extractFunctionSource(
    composer,
    'function replaceAttachmentWithText(id: string, text: string)',
  )

  // Typing coalesces into one step; the structural edits below never do.
  assert.ok(typing.includes('recordUndoHistory(true)'))
  assert.ok(replaceRange.includes('recordUndoHistory(false)'))
  assert.ok(insertChip.includes('recordUndoHistory(false)'))
  assert.ok(removeChipIds.includes('recordUndoHistory(false)'))
  assert.ok(replaceAttachment.includes('recordUndoHistory(false)'))
})

test('a document the user no longer sees starts a new baseline', () => {
  const restore = extractFunctionSource(composer, 'function restoreSegments(segments: ComposerSegment[])')
  assert.ok(restore.includes('applySegments(segments)'))
  assert.ok(restore.includes('resetUndoHistory()'))
  assert.ok(composer.includes('function resetUndoHistory()'))

  const mounted = extractFunctionSource(composer, 'onMounted(() => {')
  assert.ok(mounted.includes('resetUndoHistory()'))

  const draftWatcher = composer.slice(
    composer.indexOf('watch(\n  () => props.draft,'),
    composer.indexOf('watch(() => [props.attachedFiles, props.pendingAttachments]'),
  )
  assert.ok(draftWatcher.includes('syncAttachments()'))
  assert.ok(draftWatcher.indexOf('resetUndoHistory()') > draftWatcher.indexOf('syncAttachments()'))
})
