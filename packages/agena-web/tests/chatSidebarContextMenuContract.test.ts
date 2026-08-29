import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

const sidebarSource = readFileSync(resolve(import.meta.dir, '../src/layout/ChatSidebar.vue'), 'utf8')
const directoryRowSource = readFileSync(
  resolve(import.meta.dir, '../src/layout/chatSidebar/components/DirectoryRow.vue'),
  'utf8',
)
const sessionRowSource = readFileSync(
  resolve(import.meta.dir, '../src/layout/chatSidebar/components/SessionRow.vue'),
  'utf8',
)
const directoriesListSource = readFileSync(
  resolve(import.meta.dir, '../src/layout/chatSidebar/components/DirectoriesList.vue'),
  'utf8',
)

test('directory rows reuse the existing action menu from a mouse-position context menu', () => {
  assert.ok(directoryRowSource.includes("(e: 'open-context-menu', event: MouseEvent): void"))
  assert.ok(directoryRowSource.includes('@contextmenu="handleContextMenu"'))
  assert.ok(
    directoriesListSource.includes('@open-context-menu="(event) => props.openDirectoryActions(directory, event)"'),
  )

  assert.ok(sidebarSource.includes("id: 'open-files'"))
  assert.ok(sidebarSource.includes("id: 'toggle-collapse'"))
  assert.ok(sidebarSource.includes("id: 'copy-path'"))
  assert.ok(sidebarSource.includes("id: 'new-session'"))
  assert.ok(sidebarSource.includes("id: 'remove'"))
  assert.ok(sidebarSource.includes('copyTextToClipboard(target.path)'))
  assert.ok(sidebarSource.includes("await workspaceNavigation.openMainTab('files')"))
  assert.ok(sidebarSource.includes(':desktop-anchor-el="directoryActionsAnchorRef"'))
})

test('session rows expose the full existing session action set on right click', () => {
  assert.ok(sessionRowSource.includes("(e: 'open-context-menu', event: MouseEvent): void"))
  assert.ok(sessionRowSource.includes('@contextmenu="handleRowContextMenu"'))
  assert.ok(directoriesListSource.includes('props.openSessionActions(row.directory, row.session, event)'))
  assert.ok(directoriesListSource.includes('props.openSessionActions(directory, row.session, event)'))

  for (const id of [
    'open',
    'toggle-pin',
    'toggle-favorite',
    'rename',
    'copy-transcript',
    'export-transcript',
    'fork',
    'delete',
  ]) {
    assert.ok(sidebarSource.includes(`'${id}'`), `missing session context action: ${id}`)
  }
  assert.ok(sidebarSource.includes(':desktop-anchor-el="sessionActionsAnchorRef"'))
})

test('sidebar background has bulk context actions and keeps only one context menu open', () => {
  assert.ok(sidebarSource.includes('@contextmenu="openSidebarContextMenu"'))
  assert.ok(sidebarSource.includes("id: 'add-directory'"))
  assert.ok(sidebarSource.includes("id: 'refresh'"))
  assert.ok(sidebarSource.includes("id: 'toggle-multi-select'"))
  assert.ok(sidebarSource.includes("id: 'expand-all'"))
  assert.ok(sidebarSource.includes("id: 'collapse-all'"))
  assert.ok(sidebarSource.includes('await setPagedDirectoriesCollapsed(false)'))
  assert.ok(sidebarSource.includes('await setPagedDirectoriesCollapsed(true)'))
  assert.ok(sidebarSource.includes(':desktop-anchor-el="sidebarActionsAnchorRef"'))

  assert.ok(sidebarSource.includes('closeDesktopSessionActionMenu()'))
  assert.ok(sidebarSource.includes('directoryActionsOpen.value = false'))
  assert.ok(sidebarSource.includes('sessionActionsOpen.value = false'))
  assert.ok(sidebarSource.includes('sidebarActionsOpen.value = false'))
})
