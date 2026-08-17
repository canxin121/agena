import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('chat opens with one recent page and delegates older loading to user scroll', () => {
  const store = readFileSync(resolve(import.meta.dir, '../src/stores/chat.ts'), 'utf8')
  const navigation = readFileSync(resolve(import.meta.dir, '../src/pages/chat/useChatScrollNav.ts'), 'utf8')
  const view = readFileSync(resolve(import.meta.dir, '../src/pages/chat/ChatPageView.vue'), 'utf8')

  assert.match(store, /const MESSAGE_PAGE_SIZE = 50/)
  assert.doesNotMatch(navigation, /maxAutoPages|ensureInitialHistoryScrollable/)
  assert.match(view, /@wheel="handleWheel"/)
})
