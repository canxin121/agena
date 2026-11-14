import { afterEach, beforeEach, describe, expect, test } from 'bun:test'

import { useBotSidebarState } from '../src/features/bots/useBotSidebarState'
import { localStorageKeys } from '../src/lib/persistence/storageKeys'

const originalStorage = Object.getOwnPropertyDescriptor(globalThis, 'localStorage')
let values: Map<string, string>
beforeEach(() => {
  values = new Map()
  Object.defineProperty(globalThis, 'localStorage', {
    configurable: true,
    value: {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => values.set(key, value),
      removeItem: (key: string) => values.delete(key),
      clear: () => values.clear(),
      key: (index: number) => [...values.keys()][index] ?? null,
      get length() {
        return values.size
      },
    } satisfies Storage,
  })
})
afterEach(() => {
  if (originalStorage) Object.defineProperty(globalThis, 'localStorage', originalStorage)
  else Reflect.deleteProperty(globalThis, 'localStorage')
})

describe('Bot sidebar tree preferences', () => {
  test('persists independent Bot and entrance disclosure state with history initially folded', () => {
    const first = useBotSidebarState()
    expect(first.isBotOpen('atlas')).toBe(false)
    expect(first.isGroupOpen('atlas', 'local')).toBe(true)
    expect(first.isGroupOpen('atlas', 'local-history')).toBe(false)
    first.setBotOpen('atlas', true)
    first.setBotOpen('muse', true)
    first.setGroupOpen('atlas', 'direct', false)
    first.setGroupOpen('atlas', 'local-history', true)
    first.setBotOpen('muse', false)

    const restored = useBotSidebarState()
    expect(restored.isBotOpen('atlas')).toBe(true)
    expect(restored.isBotOpen('muse')).toBe(false)
    expect(restored.isGroupOpen('atlas', 'direct')).toBe(false)
    expect(restored.isGroupOpen('muse', 'direct')).toBe(true)
    expect(restored.isGroupOpen('atlas', 'local-history')).toBe(true)
  })

  test('malformed saved entries do not turn text into expansion flags', () => {
    values.set(
      localStorageKeys.ui.botSidebarTree,
      JSON.stringify({
        bots: { atlas: 'false', muse: true },
        groups: { atlas: { local: 'false', group: false, 'local-history': true }, muse: null },
      }),
    )
    const tree = useBotSidebarState()
    expect(tree.isBotOpen('atlas')).toBe(false)
    expect(tree.isBotOpen('muse')).toBe(true)
    expect(tree.isGroupOpen('atlas', 'local')).toBe(true)
    expect(tree.isGroupOpen('atlas', 'group')).toBe(false)
    expect(tree.isGroupOpen('atlas', 'local-history')).toBe(true)
  })
})
