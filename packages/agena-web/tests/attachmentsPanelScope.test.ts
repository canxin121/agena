import assert from 'node:assert/strict'
import test from 'node:test'

import { escapeBelongsToPanel } from '../src/components/chat/attachmentsPanelScope'

// Every pane keeps its own attachments panel in the same document, so a plain
// Escape used to close every open panel at once.

// One marker object per pane, the way a real pane element is one node.
const paneMarkers = { left: { pane: 'left' }, right: { pane: 'right' } }

function paneNode(pane: keyof typeof paneMarkers | null) {
  return {
    closest: (selector: string) => (selector === '[data-workspace-pane-window]' && pane ? paneMarkers[pane] : null),
  }
}

test('Escape closes the attachments panel of the pane that owns the keystroke', () => {
  assert.equal(escapeBelongsToPanel({ panel: paneNode('left'), target: paneNode('left'), paneFocused: true }), true)
  assert.equal(escapeBelongsToPanel({ panel: paneNode('left'), target: paneNode('right'), paneFocused: true }), false)
})

test('Escape outside every pane follows the focused pane', () => {
  assert.equal(escapeBelongsToPanel({ panel: paneNode('left'), target: paneNode(null), paneFocused: true }), true)
  assert.equal(escapeBelongsToPanel({ panel: paneNode('left'), target: paneNode(null), paneFocused: false }), false)
})

test('a panel without a pane wrapper keeps the plain behaviour', () => {
  assert.equal(escapeBelongsToPanel({ panel: paneNode(null), target: paneNode('right'), paneFocused: false }), true)
})
