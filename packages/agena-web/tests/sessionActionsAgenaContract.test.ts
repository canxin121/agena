import test from 'node:test'
import assert from 'node:assert/strict'

import { buildSessionActionItemsForSessionI18n } from '../src/layout/chatSidebar/useSessionActionMenu'

test('session actions expose Agena fork and omit unsupported share-link commands', () => {
  const items = buildSessionActionItemsForSessionI18n((key) => key, { id: '42' })
  const ids = items.map((item) => item.id)

  assert.equal(ids.includes('fork'), true)
  assert.equal(ids.includes('share'), false)
  assert.equal(ids.includes('unshare'), false)
  assert.equal(ids.includes('copy-share'), false)
  assert.equal(ids.includes('open-share'), false)
})
