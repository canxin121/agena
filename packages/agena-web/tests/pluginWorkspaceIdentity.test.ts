import { expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { useUiStore } from '../src/stores/ui'

test('instance plugin deep links without a sessionId reuse their complete-query identity', () => {
  const pinia = createPinia()
  setActivePinia(pinia)
  try {
    const ui = useUiStore()
    // Preserve tabs left in shared mock storage by other test cases.
    const existingIds = ui.workspaceWindows.map((tab) => tab.id)
    const path = '/plugins/agena.telegram/manage'
    const old = ui.openWorkspaceWindow('plugins', { path, query: { bot: 'work', view: 'activity' } })
    const target = ui.openWorkspaceWindow('plugins', { path, query: { bot: 'work', source: '1', view: 'chats' } })
    expect(target).not.toBe(old)
    const reopened = ui.openWorkspaceWindow('plugins', { path, query: { bot: 'work', source: '1', view: 'chats' } })
    expect(reopened).toBe(target)
    expect(ui.workspaceWindows).toHaveLength(existingIds.length + 2)
    expect(existingIds.every((id) => ui.workspaceWindows.some((tab) => tab.id === id))).toBe(true)
    expect(ui.activeWorkspaceWindow?.routeQuery).toEqual({ bot: 'work', source: '1', view: 'chats' })
    ui.syncActiveWorkspaceWindowFromRoute('plugins', { bot: 'work', source: '1', view: 'chats' }, { path })
    expect(ui.activeWorkspaceWindow?.id).toBe(target)
    expect(ui.activeWorkspaceWindow?.routeQuery.view).toBe('chats')
    const other = ui.openWorkspaceWindow('plugins', { path, query: { bot: 'home', source: '1', view: 'chats' } })
    expect(other).not.toBe(target)
  } finally {
    disposePinia(pinia)
    setActivePinia(undefined)
  }
})
