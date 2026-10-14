import assert from 'node:assert/strict'
import test from 'node:test'

import { closeAllComposerInputMenus, openComposerInputMenu } from '../src/pages/chat/composerInputMenus'

function makeClosers() {
  const calls: string[] = []
  return {
    calls,
    closers: {
      closeAttachments: () => calls.push('attachments'),
      closePicker: () => calls.push('picker'),
    },
  }
}

test('opening attachments closes the picker', () => {
  const { calls, closers } = makeClosers()
  openComposerInputMenu('attachments', closers)
  assert.deepEqual(calls, ['picker'])
})

test('opening the picker closes attachments', () => {
  const { calls, closers } = makeClosers()
  openComposerInputMenu('picker', closers)
  assert.deepEqual(calls, ['attachments'])
})

test('closeAllComposerInputMenus closes every remaining input menu', () => {
  const { calls, closers } = makeClosers()
  closeAllComposerInputMenus(closers)
  assert.deepEqual(calls, ['attachments', 'picker'])
})
