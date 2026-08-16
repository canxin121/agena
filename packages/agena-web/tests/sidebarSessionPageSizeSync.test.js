import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('sidebar session root page size is fixed to 10 end-to-end', () => {
  const sidebarSource = readFileSync(resolve(import.meta.dir, '../src/layout/ChatSidebar.vue'), 'utf8')
  const storeSource = readFileSync(resolve(import.meta.dir, '../src/stores/directorySessionStore.ts'), 'utf8')

  assert.ok(sidebarSource.includes('const SESSION_ROOTS_PAGE_SIZE = 10'))
  assert.ok(storeSource.includes('limitPerDirectory'))
  assert.ok(storeSource.includes('SIDEBAR_DIRECTORY_SESSIONS_PAGE_SIZE = 10'))
})
