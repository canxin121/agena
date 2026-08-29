import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

const sidebarSource = readFileSync(resolve(import.meta.dir, '../src/layout/ChatSidebar.vue'), 'utf8')
const storeSource = readFileSync(resolve(import.meta.dir, '../src/stores/directorySessionStore.ts'), 'utf8')
const rowSource = readFileSync(resolve(import.meta.dir, '../src/layout/chatSidebar/components/SessionRow.vue'), 'utf8')
const favoriteFooterSource = readFileSync(
  resolve(import.meta.dir, '../src/layout/chatSidebar/components/FavoriteSessionsFooter.vue'),
  'utf8',
)

test('sidebar projects durable favorites into a first-class paged section', () => {
  assert.ok(storeSource.includes("type SidebarFooterKind = 'pinned' | 'favorite' | 'recent' | 'running'"))
  assert.ok(storeSource.includes('if (session.favorite === true) overview.favorites.push(session)'))
  assert.ok(storeSource.includes('favoriteFooterView'))
  assert.ok(storeSource.includes('favoriteFooter: footerView(overview.favorites, uiPrefs.value.favoriteSessionsPage)'))
  assert.ok(storeSource.includes("command.kind === 'favorite'"))
  assert.ok(sidebarSource.includes("commandSetFooterOpen('favorite'"))
  assert.ok(sidebarSource.includes("commandSetFooterPage('favorite'"))
  assert.ok(sidebarSource.includes('<FavoriteSessionsFooter'))
  assert.ok(sidebarSource.includes(':favoriteSessionRows="pagedFavoriteSessionRows"'))
  assert.ok(favoriteFooterSource.includes('chat.sidebar.footers.favorites.title'))
})

test('favorite state stays visibly marked and toggle actions use the row state instead of stale cache', () => {
  assert.ok(rowSource.includes('v-if="isFavorite"'))
  assert.ok(rowSource.includes('text-amber-500'))
  assert.ok(rowSource.includes("emit('toggle-favorite', isFavorite)"))
  assert.ok(sidebarSource.includes('favoriteOptimisticBySessionId'))
  assert.ok(sidebarSource.includes('[sid]: nextFavorite'))
  assert.ok(sidebarSource.includes('Object.prototype.hasOwnProperty.call(favoriteOptimisticBySessionId.value, sid)'))
  assert.ok(sidebarSource.includes("typeof rowSession?.favorite === 'boolean'"))
  assert.ok(sidebarSource.includes('await chat.updateSessionMetadata(sid, { favorite: nextFavorite })'))
})
