import { beforeEach, describe, expect, test } from 'bun:test'
import {
  defaultChatSidebarUiPrefs,
  loadChatSidebarUiPrefs,
  saveChatSidebarUiPrefs,
} from '../src/data/chatSidebarUiPrefs'
import { localStorageKeys } from '../src/lib/persistence/storageKeys'

class MemoryStorage {
  private values = new Map<string, string>()
  getItem(key: string) {
    return this.values.get(key) ?? null
  }
  setItem(key: string, value: string) {
    this.values.set(key, value)
  }
  removeItem(key: string) {
    this.values.delete(key)
  }
}

beforeEach(() => {
  Object.defineProperty(globalThis, 'localStorage', {
    value: new MemoryStorage(),
    configurable: true,
    writable: true,
  })
})

describe('chat sidebar chrome memory', () => {
  test('an untouched client starts from the defaults', () => {
    expect(loadChatSidebarUiPrefs()).toEqual(defaultChatSidebarUiPrefs())
  })

  test('every remembered place survives a reload', () => {
    const prefs = {
      ...defaultChatSidebarUiPrefs(),
      collapsedDirectoryIds: ['3', '7'],
      expandedParentSessionIds: ['42'],
      directoriesPage: 2,
      sessionRootPageByDirectoryId: { '3': 1, '7': 4 },
      pinnedSessionsOpen: true,
      pinnedSessionsPage: 3,
      favoriteSessionsOpen: true,
      favoriteSessionsPage: 2,
      recentSessionsOpen: true,
      recentSessionsPage: 5,
      runningSessionsOpen: true,
      runningSessionsPage: 1,
    }
    saveChatSidebarUiPrefs(prefs)
    // A fresh load is what a page reload does; all of it must come back.
    expect(loadChatSidebarUiPrefs()).toEqual(prefs)
  })

  test('the stored copy is the namespaced key, not an ad hoc literal', () => {
    saveChatSidebarUiPrefs({ ...defaultChatSidebarUiPrefs(), directoriesPage: 4 })
    expect(localStorage.getItem(localStorageKeys.chat.sidebarUiPrefs)).not.toBeNull()
  })

  test('corrupt storage falls back to the defaults instead of throwing', () => {
    localStorage.setItem(localStorageKeys.chat.sidebarUiPrefs, '{not json')
    expect(loadChatSidebarUiPrefs()).toEqual(defaultChatSidebarUiPrefs())
  })
})
