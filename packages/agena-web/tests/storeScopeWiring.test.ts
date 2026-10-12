import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

// A Pinia store is created once, so a store setup that captured a pane-scoped
// view would freeze one pane's window scope into every consumer afterwards.

const read = (relative: string) => readFileSync(resolve(import.meta.dir, relative), 'utf8')

test('singleton store setups read the raw store, never a pane-scoped view', () => {
  const chatStore = read('../src/stores/chat.ts')
  const directoryStore = read('../src/stores/directory.ts')
  const sessionStore = read('../src/stores/directorySessionStore.ts')
  const previewStore = read('../src/stores/workspacePreview.ts')

  assert.ok(chatStore.includes('const directoryStore = useDirectoryStoreUnscoped()'))
  assert.ok(sessionStore.includes('const chat = useChatStoreUnscoped()'))
  assert.ok(!chatStore.includes('const directoryStore = useDirectoryStore()'))
  assert.ok(!sessionStore.includes('const chat = useChatStore()'))

  // The raw stores stay reachable, so the next setup does not have to guess.
  assert.ok(directoryStore.includes('export function useDirectoryStoreUnscoped'))
  assert.ok(chatStore.includes('export function useChatStoreUnscoped'))
  assert.ok(previewStore.includes('export function useWorkspacePreviewStoreUnscoped'))
})
