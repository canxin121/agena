import { afterEach, expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import {
  isConversationSettingsPath,
  isMainTabId,
  isWorkspaceMainTabPath,
  normalizeMainTabPath,
} from '../src/app/navigation/mainTabs'
import { clientCommandFromCatalog } from '../src/pages/chat/chatCommandsCatalog'
import type { PluginCommandCatalogItem } from '../src/lib/pluginOperations'
import { useUiStore } from '../src/stores/ui'
import { localStorageKeys } from '../src/lib/persistence/storageKeys'

const originalStorage = Object.getOwnPropertyDescriptor(globalThis, 'localStorage')
afterEach(() => {
  if (originalStorage) Object.defineProperty(globalThis, 'localStorage', originalStorage)
  else Reflect.deleteProperty(globalThis, 'localStorage')
  setActivePinia(undefined)
})

test('retired Task routes and command actions cannot become Web destinations', () => {
  expect(isMainTabId('tasks')).toBe(false)
  expect(isConversationSettingsPath('/assistant')).toBe(true)
  expect(isConversationSettingsPath('/integrations')).toBe(true)
  expect(isConversationSettingsPath('/tasks')).toBe(false)
  expect(isWorkspaceMainTabPath('/tasks')).toBe(false)
  expect(normalizeMainTabPath('chat', '/tasks')).toBe('/chat')
  for (const action of ['task', 'tasks']) {
    expect(
      clientCommandFromCatalog({
        id: action,
        slash: `/${action}`,
        aliases: [],
        target: { kind: 'client', action },
        docs: {},
      } as PluginCommandCatalogItem),
    ).toBeNull()
  }
})

test('restoring old workspace state discards Task tabs and keeps canonical Bot and project sessions', () => {
  const values = new Map<string, string>([
    [localStorageKeys.ui.activeMainTab, 'tasks'],
    [
      localStorageKeys.ui.workspaceWindows,
      JSON.stringify([
        { id: 'old-task', mainTab: 'tasks', routePath: '/tasks', routeQuery: { source: '7', sessionId: '99' } },
        { id: 'bot', mainTab: 'bot', routePath: '/bot', routeQuery: { sessionId: '42' } },
        { id: 'project', mainTab: 'chat', routePath: '/chat', routeQuery: { sessionId: '7' } },
      ]),
    ],
  ])
  Object.defineProperty(globalThis, 'localStorage', {
    configurable: true,
    value: {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => values.set(key, value),
      removeItem: (key: string) => values.delete(key),
    },
  })
  const pinia = createPinia()
  setActivePinia(pinia)
  try {
    const ui = useUiStore()
    expect(ui.workspaceWindows.map((tab) => [tab.mainTab, tab.routePath, tab.routeQuery.sessionId])).toEqual([
      ['bot', '/bot', '42'],
      ['chat', '/chat', '7'],
    ])
  } finally {
    disposePinia(pinia)
  }
})
