import { shallowRef } from 'vue'
import { SHORTCUT_ACTIONS, validateShortcut, type ShortcutOverrides } from './keyboardShortcutActions'

export const KEYBOARD_STORAGE_KEY = 'agena.keyboard-shortcuts.v1'

export function readShortcutOverrides(storage: Pick<Storage, 'getItem'>): ShortcutOverrides {
  try {
    const parsed = JSON.parse(storage.getItem(KEYBOARD_STORAGE_KEY) || 'null')
    if (parsed?.version !== 1 || !parsed.overrides || typeof parsed.overrides !== 'object') return {}
    const result: ShortcutOverrides = {}
    for (const action of SHORTCUT_ACTIONS) {
      const values: unknown = parsed.overrides[action]
      if (values === undefined) continue
      if (!Array.isArray(values) || values.length > 8 || values.some((value) => typeof value !== 'string')) return {}
      result[action] = [...new Set(values)] as string[]
    }
    for (const action of SHORTCUT_ACTIONS) {
      if (result[action]?.some((binding) => validateShortcut(binding, action, result))) return {}
    }
    return result
  } catch {
    return {}
  }
}

export function writeShortcutOverrides(storage: Pick<Storage, 'setItem'>, overrides: ShortcutOverrides): void {
  storage.setItem(KEYBOARD_STORAGE_KEY, JSON.stringify({ version: 1, overrides }))
}

function readBrowserPreferences(): ShortcutOverrides {
  try {
    return readShortcutOverrides(window.localStorage)
  } catch {
    return {}
  }
}

export const keyboardOverrides = shallowRef<ShortcutOverrides>(readBrowserPreferences())
export function saveKeyboardOverrides(overrides: ShortcutOverrides): boolean {
  try {
    writeShortcutOverrides(window.localStorage, overrides)
    keyboardOverrides.value = overrides
    return true
  } catch {
    return false
  }
}

if (typeof window !== 'undefined')
  window.addEventListener('storage', (event) => {
    if (event.key === KEYBOARD_STORAGE_KEY || event.key === null) keyboardOverrides.value = readBrowserPreferences()
  })
