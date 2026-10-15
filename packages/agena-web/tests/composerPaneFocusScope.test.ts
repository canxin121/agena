import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

import { composerFocusSelectors, pickComposerFocusTarget } from '../src/app/runtime/composerFocus'

// Cmd+I used to focus `document.querySelector('[data-chat-input]')`, which is the
// first composer in the document - a different pane, or a hidden tab of one.

const read = (relative: string) => readFileSync(resolve(import.meta.dir, relative), 'utf8')

test('composer focus is scoped to the focused pane and falls back to the page', () => {
  assert.deepEqual(composerFocusSelectors('window-2'), [
    '[data-workspace-pane-window="window-2"] [data-chat-input="true"]',
    '[data-chat-input="true"]',
  ])
  assert.deepEqual(composerFocusSelectors(''), ['[data-chat-input="true"]'])
})

test('the focused pane composer wins over another pane and over hidden tabs', () => {
  const focused = { id: 'focused' } as unknown as HTMLElement
  const other = { id: 'other' } as unknown as HTMLElement
  const hiddenInFocusedPane = { id: 'hidden' } as unknown as HTMLElement

  const picked = pickComposerFocusTarget({
    windowId: 'window-2',
    queryAll: (selector) =>
      selector.startsWith('[data-workspace-pane-window')
        ? [hiddenInFocusedPane, focused]
        : [other, hiddenInFocusedPane, focused],
    isVisible: (element) => element !== hiddenInFocusedPane,
  })

  assert.equal(picked, focused)
})

test('hidden composers are excluded so the caller can activate a chat pane', () => {
  const hidden = { id: 'hidden' } as unknown as HTMLElement
  const picked = pickComposerFocusTarget({
    windowId: 'window-2',
    queryAll: () => [hidden],
    isVisible: () => false,
  })

  assert.equal(picked, null)
})

test('the Cmd+I shortcut uses the pane-scoped picker', () => {
  const source = read('../src/app/runtime/installKeyboardShortcuts.ts')

  assert.ok(source.includes('pickComposerFocusTarget({'))
  assert.ok(source.includes('windowId: ui.focusedWorkspaceWindowId'))
  assert.ok(!source.includes('document.querySelector<HTMLElement>(\'[data-chat-input="true"]\')?.focus()'))
})
