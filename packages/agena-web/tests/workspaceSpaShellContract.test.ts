import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

import { mainTabPath, normalizeMainTabPath } from '../src/app/navigation/mainTabs.ts'

test('workspace tab locations preserve valid nested SPA routes', () => {
  assert.equal(mainTabPath('files'), '/files')
  assert.equal(normalizeMainTabPath('settings', '/settings/models-providers'), '/settings/models-providers')
  assert.equal(normalizeMainTabPath('chat', '/'), '/chat')
  assert.equal(normalizeMainTabPath('files', '/settings/models-providers'), '/files')
})

test('desktop workspace uses one Vue document and retains visited tab views', () => {
  const groupPane = readFileSync(resolve(import.meta.dir, '../src/layout/WorkspaceEditorGroupPane.vue'), 'utf8')
  const paneView = readFileSync(resolve(import.meta.dir, '../src/layout/WorkspacePaneView.vue'), 'utf8')
  const mainLayout = readFileSync(resolve(import.meta.dir, '../src/layout/MainLayout.vue'), 'utf8')
  const chatPage = readFileSync(resolve(import.meta.dir, '../src/pages/ChatPage.vue'), 'utf8')

  assert.ok(groupPane.includes('const mountedWindowIds = ref<string[]>([])'))
  assert.ok(groupPane.includes('v-for="windowTab in mountedTabs"'))
  assert.ok(groupPane.includes('v-show="isWindowActive(windowTab.id)"'))
  assert.ok(!groupPane.includes('<iframe'))
  assert.ok(paneView.includes('loadRouteLocation(next)'))
  assert.ok(paneView.includes('sequence !== routeLoadSequence'))
  assert.ok(paneView.includes('loadedRouteFullPath === next.fullPath'))
  assert.ok(paneView.includes('globalRouter.currentRoute.value.fullPath === shellRoute.fullPath'))
  assert.ok(paneView.includes('<RouterView v-if="routeReady" :route="scopedRoute"'))
  assert.ok(!chatPage.includes('chat.clearTranscriptCache()'))
  assert.equal((mainLayout.match(/useAppRuntime\(\)/g) || []).length, 1)
})

test('workspace tabs persist route, group, focus, and split ratio state', () => {
  const uiStore = readFileSync(resolve(import.meta.dir, '../src/stores/ui.ts'), 'utf8')

  for (const expected of [
    'persistWorkspaceShellJson(STORAGE_WORKSPACE_WINDOWS, list)',
    'persistWorkspaceShellJson(STORAGE_WORKSPACE_GROUPS, list)',
    'persistWorkspaceShellJson(STORAGE_WORKSPACE_GROUP_PANE_RATIOS, value)',
    "persistWorkspaceShellString(STORAGE_ACTIVE_WORKSPACE_WINDOW_ID, String(v || ''))",
    "persistWorkspaceShellString(STORAGE_ACTIVE_WORKSPACE_GROUP_ID, String(v || ''))",
    'routePath',
    'routeQuery',
    'routeHash',
  ]) {
    assert.ok(uiStore.includes(expected), `missing persisted workspace state: ${expected}`)
  }
})
