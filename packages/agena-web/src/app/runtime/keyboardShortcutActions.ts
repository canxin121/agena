import { isImeComposing } from '@/lib/imeKeyboard'
import { MAIN_TABS, type NavigationMainTabId } from '@/app/navigation/mainTabs'

export type ShortcutKey = Pick<
  KeyboardEvent,
  | 'key'
  | 'code'
  | 'ctrlKey'
  | 'metaKey'
  | 'altKey'
  | 'shiftKey'
  | 'repeat'
  | 'defaultPrevented'
  | 'isComposing'
  | 'keyCode'
>
export type AppShortcut =
  | { type: 'help' | 'settings' | 'sidebar' | 'composer' | 'new-session' | 'theme' }
  | { type: 'main-tab'; tab: NavigationMainTabId }

export const SHORTCUT_ACTIONS = [
  'help',
  'settings',
  'sidebar',
  'composer',
  'new-session',
  'theme',
  'tab-chat',
  'tab-files',
  'tab-preview',
  'tab-terminal',
  'tab-git',
] as const
export type ShortcutActionId = (typeof SHORTCUT_ACTIONS)[number]
export type ShortcutOverrides = Partial<Record<ShortcutActionId, string[]>>

export const DEFAULT_SHORTCUTS: Record<ShortcutActionId, string[]> = {
  help: ['Ctrl+KeyH', 'Mod+Period', 'Alt+Shift+KeyH'],
  settings: ['Mod+Comma', 'Alt+Shift+KeyS'],
  sidebar: ['Mod+KeyL', 'Alt+Shift+KeyL'],
  composer: ['Meta+KeyI', 'Alt+Shift+KeyI'],
  'new-session': ['Mod+KeyN', 'Alt+Shift+KeyN'],
  theme: ['Mod+Slash', 'Alt+Shift+KeyT'],
  'tab-chat': ['Mod+Digit1', 'Alt+Shift+Digit1'],
  'tab-files': ['Mod+Digit2', 'Alt+Shift+Digit2'],
  'tab-preview': ['Mod+Digit3', 'Alt+Shift+Digit3'],
  'tab-terminal': ['Mod+Digit4', 'Alt+Shift+Digit4'],
  'tab-git': ['Mod+Digit5', 'Alt+Shift+Digit5'],
}

const keyCodeFor = (event: ShortcutKey): string => {
  if (event.code) return event.code
  const key = event.key.toLowerCase()
  if (/^[a-z]$/.test(key)) return `Key${key.toUpperCase()}`
  if (/^[0-9]$/.test(key)) return `Digit${key}`
  return (
    (
      {
        '.': 'Period',
        ',': 'Comma',
        '/': 'Slash',
        ';': 'Semicolon',
        "'": 'Quote',
        '[': 'BracketLeft',
        ']': 'BracketRight',
        '-': 'Minus',
        '=': 'Equal',
        '`': 'Backquote',
        '\\': 'Backslash',
        ' ': 'Space',
      } as Record<string, string>
    )[key] || event.key
  )
}

export function shortcutFromEvent(event: ShortcutKey): string | null {
  if (isImeComposing(event) || /^(Control|Meta|Alt|Shift|OS|AltGraph)$/.test(event.key)) return null
  return [
    ...(event.ctrlKey ? ['Ctrl'] : []),
    ...(event.metaKey ? ['Meta'] : []),
    ...(event.altKey ? ['Alt'] : []),
    ...(event.shiftKey ? ['Shift'] : []),
    keyCodeFor(event),
  ].join('+')
}

export function shortcutMatches(binding: string, actual: string): boolean {
  return (
    binding === actual ||
    (binding.includes('Mod+') && [binding.replace('Mod+', 'Ctrl+'), binding.replace('Mod+', 'Meta+')].includes(actual))
  )
}

export function shortcutBindings(action: ShortcutActionId, overrides: ShortcutOverrides = {}): string[] {
  return overrides[action] ?? DEFAULT_SHORTCUTS[action]
}

export type ShortcutIssue = { type: 'reserved' | 'modifier' | 'duplicate'; action?: ShortcutActionId }
export function validateShortcut(
  binding: string,
  action: ShortcutActionId,
  overrides: ShortcutOverrides = {},
): ShortcutIssue | null {
  if (
    !/^(?:(?:Ctrl|Meta|Mod|Alt|Shift)\+)*(?:Key[A-Z]|Digit[0-9]|F(?:[1-9]|1[0-2])|Arrow(?:Up|Down|Left|Right)|Home|End|PageUp|PageDown|Space|Enter|Backspace|Delete|Insert|Period|Comma|Slash|Semicolon|Quote|BracketLeft|BracketRight|Backslash|Minus|Equal|Backquote)$/.test(
      binding,
    )
  )
    return { type: 'reserved' }
  if (binding.endsWith('+F6') || binding === 'F6') return { type: 'reserved' }
  const modifiers = binding.split('+').slice(0, -1)
  const canonical = ['Ctrl', 'Meta', 'Mod', 'Alt', 'Shift'].filter((key) => modifiers.includes(key))
  if (
    modifiers.join('+') !== canonical.join('+') ||
    (modifiers.includes('Mod') && (modifiers.includes('Ctrl') || modifiers.includes('Meta')))
  )
    return { type: 'reserved' }
  // Preserve text entry, IME/AltGr, and the application's region/dismissal keys.
  if (
    !/^(?:Ctrl|Meta|Mod|Alt)\+/.test(binding) ||
    (binding.includes('Ctrl+') && binding.includes('Alt+') && !binding.includes('Meta+'))
  )
    return { type: 'modifier' }
  for (const other of SHORTCUT_ACTIONS) {
    if (other === action) continue
    if (
      shortcutBindings(other, overrides).some(
        (existing) => shortcutMatches(existing, binding) || shortcutMatches(binding, existing),
      )
    )
      return { type: 'duplicate', action: other }
  }
  return null
}

export function shortcutMayConflictWithBrowser(binding: string): boolean {
  return /^(?:Ctrl|Meta|Mod)\+(?:Shift\+)?(?:Key[LNTRWQPHJKUDF]|Digit[0-9]|Comma|Period|Slash)$/.test(binding)
}

export function formatShortcut(binding: string): string {
  const mac = typeof navigator !== 'undefined' && /Macintosh|Mac OS X/.test(navigator.userAgent)
  return binding
    .replace('Mod+', mac ? 'Cmd+' : 'Ctrl+')
    .replace('Meta+', 'Cmd+')
    .replace(/Key([A-Z])$/, '$1')
    .replace(/Digit([0-9])$/, '$1')
    .replace(/Period$/, '.')
    .replace(/Comma$/, ',')
    .replace(/Slash$/, '/')
}

export function resolveAppShortcut(
  event: ShortcutKey,
  terminalOwnsInput = false,
  overrides: ShortcutOverrides = {},
): AppShortcut | null {
  if (event.defaultPrevented || event.repeat || isImeComposing(event)) return null
  if (terminalOwnsInput && !event.metaKey && !(event.altKey && event.shiftKey && !event.ctrlKey)) return null
  const actual = shortcutFromEvent(event)
  if (!actual) return null
  const action = SHORTCUT_ACTIONS.find((id) =>
    shortcutBindings(id, overrides).some((binding) => shortcutMatches(binding, actual)),
  )
  if (!action) return null
  if (action.startsWith('tab-')) {
    const tab = MAIN_TABS.find((tab) => tab.id === action.slice(4))
    return tab ? { type: 'main-tab', tab: tab.id } : null
  }
  return { type: action as Exclude<ShortcutActionId, `tab-${string}`> }
}
