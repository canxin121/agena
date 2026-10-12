import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

import { copySelectionBelongsToRoot } from '../src/pages/chat/transcriptCopyOwnership'

// The transcript listens for `copy` on the document. A mouse selection stays in
// the pane that made it, so a copy elsewhere must not be answered with it.

test('a transcript mouse selection only answers copies made inside its own pane', () => {
  const anchor = { id: 'anchor' } as unknown as Node
  const otherAnchor = { id: 'other-pane' } as unknown as Node
  const scroll = { contains: (node: Node | null) => node === anchor }

  assert.equal(copySelectionBelongsToRoot(scroll, { rangeCount: 1, anchorNode: anchor }), true)
  assert.equal(copySelectionBelongsToRoot(scroll, { rangeCount: 1, anchorNode: otherAnchor }), false)
  assert.equal(copySelectionBelongsToRoot(scroll, { rangeCount: 0, anchorNode: anchor }), false)
  assert.equal(copySelectionBelongsToRoot(scroll, null), false)
  assert.equal(copySelectionBelongsToRoot(null, { rangeCount: 1, anchorNode: anchor }), false)
})

test('the document copy handler checks ownership before stealing the copy', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/pages/chat/useChatTranscriptVim.ts'), 'utf8')
  const handlerStart = source.indexOf('function onTranscriptCopy')
  const ownerCheck = source.indexOf('if (!ownsDocumentCopy()) return')

  // The check has to run inside the copy handler, not only in its helper.
  assert.ok(handlerStart > 0)
  assert.ok(ownerCheck > handlerStart)
  assert.ok(source.includes('copySelectionBelongsToRoot(opts.scrollEl.value, window.getSelection())'))
})
