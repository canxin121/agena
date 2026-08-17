import { describe, expect, test } from 'bun:test'

import {
  MAX_PROMPT_HISTORY_ENTRIES,
  addPromptHistoryEntry,
  filterPromptHistory,
  loadPromptHistory,
  normalizePromptHistoryItems,
  persistPromptHistory,
  useComposerPromptHistory,
} from '../src/pages/chat/composerPromptHistory'

function memoryStorage(): Storage {
  const data = new Map<string, string>()
  return {
    get length() {
      return data.size
    },
    clear() {
      data.clear()
    },
    getItem(key: string) {
      return data.get(key) ?? null
    },
    key(index: number) {
      return Array.from(data.keys())[index] ?? null
    },
    removeItem(key: string) {
      data.delete(key)
    },
    setItem(key: string, value: string) {
      data.set(key, String(value))
    },
  }
}

describe('composer prompt history', () => {
  test('normalizes newest-first data and removes duplicates', () => {
    expect(normalizePromptHistoryItems([' newest ', '', 'old', 'newest', 42])).toEqual(['newest', 'old'])
    expect(addPromptHistoryEntry(['newest', 'old'], 'old')).toEqual(['old', 'newest'])
  })

  test('keeps at most the TUI-sized history and persists successful entries', () => {
    const storage = memoryStorage()
    const entries = Array.from({ length: MAX_PROMPT_HISTORY_ENTRIES + 10 }, (_, index) => `prompt-${index}`)
    persistPromptHistory(entries, storage)
    const loaded = loadPromptHistory(storage)
    expect(loaded).toHaveLength(MAX_PROMPT_HISTORY_ENTRIES)
    expect(loaded[0]).toBe('prompt-0')
    expect(loaded.at(-1)).toBe(`prompt-${MAX_PROMPT_HISTORY_ENTRIES - 1}`)
  })

  test('filters without changing newest-first order', () => {
    expect(filterPromptHistory(['newest fix', 'old note', 'another fix'], 'FIX')).toEqual(['newest fix', 'another fix'])
  })

  test('opens with the newest entry, navigates older/newer, and accepts explicitly', () => {
    const storage = memoryStorage()
    persistPromptHistory(['newest', 'middle', 'oldest'], storage)
    const history = useComposerPromptHistory({ storage })

    expect(history.openHistory()).toBe(true)
    expect(history.filteredEntries.value).toEqual(['newest', 'middle', 'oldest'])
    expect(history.focus.value).toBe('input')

    history.focusResults()
    history.moveOlder()
    expect(history.activeIndex.value).toBe(1)
    expect(history.selectedText()).toBe('middle')
    history.moveNewer()
    expect(history.activeIndex.value).toBe(0)
    history.moveNewer()
    expect(history.focus.value).toBe('input')
    expect(history.open.value).toBe(true)

    history.focusResults()
    expect(history.accept()).toBe('newest')
    expect(history.open.value).toBe(false)
  })

  test('recording moves an existing prompt to newest and does not duplicate it', () => {
    const storage = memoryStorage()
    persistPromptHistory(['newest', 'old'], storage)
    const history = useComposerPromptHistory({ storage })

    expect(history.record('old')).toBe(true)
    expect(history.entries.value).toEqual(['old', 'newest'])
    expect(history.record('old')).toBe(false)
    expect(loadPromptHistory(storage)).toEqual(['old', 'newest'])
  })
})
