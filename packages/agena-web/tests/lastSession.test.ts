import { beforeEach, describe, expect, test } from 'bun:test'
import {
  clearLastSessionId,
  readLastSessionId,
  reloadLastSessionId,
  writeLastSessionId,
} from '../src/lib/lastSession'
import { localStorageKeys } from '../src/lib/persistence/storageKeys'

// A minimal localStorage so the module's own key handling is exercised rather
// than mocked away.
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
  clear() {
    this.values.clear()
  }
}

beforeEach(() => {
  Object.defineProperty(globalThis, 'localStorage', {
    value: new MemoryStorage(),
    configurable: true,
    writable: true,
  })
  reloadLastSessionId()
})

describe('last used conversation', () => {
  test('records a positive integer id and ignores anything else', () => {
    writeLastSessionId('42')
    expect(readLastSessionId()).toBe('42')
    writeLastSessionId('not-a-session')
    expect(readLastSessionId()).toBe('42')
    writeLastSessionId('0')
    expect(readLastSessionId()).toBe('42')
    writeLastSessionId('-3')
    expect(readLastSessionId()).toBe('42')
  })

  test('adopts the pre-namespaced key written by earlier builds', () => {
    localStorage.setItem('agena.chat.selected-session-id', JSON.stringify('7'))
    reloadLastSessionId()
    expect(readLastSessionId()).toBe('7')
    expect(JSON.parse(localStorage.getItem(localStorageKeys.chat.lastSessionId) || '""')).toBe('7')
  })

  test('clearing removes the record so a new client starts empty', () => {
    writeLastSessionId('9')
    clearLastSessionId()
    expect(readLastSessionId()).toBe('')
  })
})
