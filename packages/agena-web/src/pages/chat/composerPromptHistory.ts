import { computed, ref, type Ref } from 'vue'

import { localStorageKeys } from '../../lib/persistence/storageKeys'

/** Keep the Web prompt history bounded to the same size as the TUI history. */
export const MAX_PROMPT_HISTORY_ENTRIES = 200

export type PromptHistoryFocus = 'input' | 'results'

function browserStorage(): Storage | null {
  try {
    return typeof window !== 'undefined' ? window.localStorage : null
  } catch {
    // Private browsing and blocked storage can throw while reading the
    // property. Prompt history is a convenience, so it must not break chat.
    return null
  }
}

export function normalizePromptHistoryText(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

/**
 * Normalize persisted history while keeping the first occurrence newest.
 * Web stores newest-first; accepting only arrays also makes malformed storage
 * harmless instead of allowing arbitrary objects into the composer.
 */
export function normalizePromptHistoryItems(value: unknown): string[] {
  if (!Array.isArray(value)) return []
  const seen = new Set<string>()
  const items: string[] = []
  for (const candidate of value) {
    const text = normalizePromptHistoryText(candidate)
    if (!text || seen.has(text)) continue
    seen.add(text)
    items.push(text)
    if (items.length >= MAX_PROMPT_HISTORY_ENTRIES) break
  }
  return items
}

export function loadPromptHistory(storage: Storage | null = browserStorage()): string[] {
  if (!storage) return []
  try {
    const raw = storage.getItem(localStorageKeys.chat.promptHistory)
    if (!raw) return []
    return normalizePromptHistoryItems(JSON.parse(raw))
  } catch {
    return []
  }
}

export function persistPromptHistory(items: readonly string[], storage: Storage | null = browserStorage()): void {
  if (!storage) return
  try {
    storage.setItem(localStorageKeys.chat.promptHistory, JSON.stringify(normalizePromptHistoryItems(items)))
  } catch {
    // A full or unavailable localStorage should not make sending fail.
  }
}

/** Insert a successful plain-text prompt at the newest end of history. */
export function addPromptHistoryEntry(items: readonly string[], value: unknown): string[] {
  const text = normalizePromptHistoryText(value)
  if (!text) return normalizePromptHistoryItems(items)
  return normalizePromptHistoryItems([text, ...items.filter((item) => item !== text)])
}

export function filterPromptHistory(items: readonly string[], query: string): string[] {
  const normalizedQuery = query.trim().toLocaleLowerCase()
  if (!normalizedQuery) return [...items]
  return items.filter((item) => item.toLocaleLowerCase().includes(normalizedQuery))
}

export type ComposerPromptHistoryState = {
  entries: Ref<string[]>
  filteredEntries: Readonly<Ref<string[]>>
  open: Ref<boolean>
  query: Ref<string>
  activeIndex: Ref<number>
  focus: Ref<PromptHistoryFocus>
  openHistory: () => boolean
  closeHistory: () => void
  updateQuery: (value: string) => void
  focusInput: () => void
  focusResults: () => void
  moveOlder: () => void
  moveNewer: (keepResultsFocus?: boolean) => void
  selectedText: () => string | null
  accept: () => string | null
  record: (value: unknown) => boolean
  reload: () => void
}

export function useComposerPromptHistory(options: { storage?: Storage | null } = {}): ComposerPromptHistoryState {
  const storage = options.storage === undefined ? browserStorage() : options.storage
  const entries = ref(loadPromptHistory(storage))
  const open = ref(false)
  const query = ref('')
  const activeIndex = ref(0)
  const focus = ref<PromptHistoryFocus>('input')

  const filteredEntries = computed(() => filterPromptHistory(entries.value, query.value))

  function clampActiveIndex() {
    activeIndex.value = filteredEntries.value.length
      ? Math.max(0, Math.min(activeIndex.value, filteredEntries.value.length - 1))
      : 0
  }

  function reload() {
    entries.value = loadPromptHistory(storage)
    clampActiveIndex()
  }

  function closeHistory() {
    open.value = false
    query.value = ''
    activeIndex.value = 0
    focus.value = 'input'
  }

  function openHistory(): boolean {
    reload()
    if (!entries.value.length) return false
    open.value = true
    query.value = ''
    activeIndex.value = 0
    focus.value = 'input'
    return true
  }

  function updateQuery(value: string) {
    query.value = String(value || '')
    activeIndex.value = 0
    focus.value = 'input'
  }

  function focusInput() {
    focus.value = 'input'
  }

  function focusResults() {
    focus.value = 'results'
    clampActiveIndex()
  }

  function moveOlder() {
    focusResults()
    if (!filteredEntries.value.length) return
    activeIndex.value = Math.min(activeIndex.value + 1, filteredEntries.value.length - 1)
  }

  function moveNewer(keepResultsFocus = false) {
    if (!filteredEntries.value.length) return
    if (keepResultsFocus) {
      focusResults()
      activeIndex.value = Math.max(0, activeIndex.value - 1)
      return
    }
    if (focus.value !== 'results') return
    if (activeIndex.value === 0) {
      // SearchPicker returns focus to the search editor at the newest item.
      focusInput()
      return
    }
    activeIndex.value -= 1
  }

  function selectedText(): string | null {
    return filteredEntries.value[activeIndex.value] ?? null
  }

  function accept(): string | null {
    const text = selectedText()
    closeHistory()
    return text
  }

  function record(value: unknown): boolean {
    const next = addPromptHistoryEntry(entries.value, value)
    const changed = next.length !== entries.value.length || next.some((item, index) => item !== entries.value[index])
    entries.value = next
    if (changed) persistPromptHistory(next, storage)
    closeHistory()
    return changed
  }

  return {
    entries,
    filteredEntries,
    open,
    query,
    activeIndex,
    focus,
    openHistory,
    closeHistory,
    updateQuery,
    focusInput,
    focusResults,
    moveOlder,
    moveNewer,
    selectedText,
    accept,
    record,
    reload,
  }
}
