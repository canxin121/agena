import assert from 'node:assert/strict'
import test from 'node:test'
import {
  KEYBOARD_STORAGE_KEY,
  readShortcutOverrides,
  writeShortcutOverrides,
} from '../src/app/runtime/keyboardPreferences'
import {
  DEFAULT_SHORTCUTS,
  resolveAppShortcut,
  shortcutFromEvent,
  validateShortcut,
  shortcutMayConflictWithBrowser,
} from '../src/app/runtime/keyboardShortcutActions'
import { keyboardViewportSize } from '../src/features/workspacePreview/model/previewKeyboard'

function key(value: string, options: Partial<KeyboardEvent> = {}): KeyboardEvent {
  return {
    key: value,
    code: '',
    ctrlKey: false,
    metaKey: false,
    altKey: false,
    shiftKey: false,
    defaultPrevented: false,
    repeat: false,
    isComposing: false,
    keyCode: 0,
    ...options,
  } as KeyboardEvent
}

test('custom shortcuts replace defaults immediately, can be disabled, and keep terminal and IME ownership', () => {
  const overrides = { sidebar: ['Alt+Shift+KeyB'], help: [] }
  assert.equal(resolveAppShortcut(key('l', { ctrlKey: true }), false, overrides), null)
  assert.equal(resolveAppShortcut(key('.', { metaKey: true }), false, overrides), null)
  const custom = key('B', { code: 'KeyB', altKey: true, shiftKey: true })
  assert.deepEqual(resolveAppShortcut(custom, true, overrides), { type: 'sidebar' })
  assert.equal(resolveAppShortcut({ ...custom, isComposing: true }, false, overrides), null)
  assert.equal(resolveAppShortcut(key('b', { ctrlKey: true }), true, { sidebar: ['Ctrl+KeyB'] }), null)
})

test('recording preserves physical keys across keyboard layouts and rejects duplicate or reserved keys', () => {
  assert.equal(shortcutFromEvent(key('¡', { code: 'Digit1', altKey: true, shiftKey: true })), 'Alt+Shift+Digit1')
  assert.equal(shortcutFromEvent(key('Meta', { metaKey: true })), null)
  assert.equal(shortcutFromEvent(key('a', { isComposing: true })), null)
  assert.deepEqual(validateShortcut('Ctrl+KeyL', 'help'), { type: 'duplicate', action: 'sidebar' })
  for (const binding of [
    'Tab',
    'Escape',
    'F6',
    'Alt+F6',
    'Ctrl+Escape',
    'Ctrl+Ctrl+KeyB',
    'Shift+Alt+KeyB',
    'Mod+Ctrl+KeyB',
  ])
    assert.equal(validateShortcut(binding, 'help')?.type, 'reserved')
  for (const binding of ['KeyA', 'Shift+KeyB', 'Ctrl+Alt+KeyE'])
    assert.equal(validateShortcut(binding, 'help')?.type, 'modifier')
  assert.equal(validateShortcut('Alt+Shift+KeyB', 'sidebar'), null)
  assert.equal(shortcutMayConflictWithBrowser('Mod+KeyN'), true)
  assert.equal(shortcutMayConflictWithBrowser('Alt+Shift+KeyB'), false)
  for (const [action, bindings] of Object.entries(DEFAULT_SHORTCUTS))
    for (const binding of bindings)
      assert.equal(validateShortcut(binding, action as keyof typeof DEFAULT_SHORTCUTS), null)
})

test('shortcut preferences roundtrip, tolerate unavailable storage, and reject corrupt or conflicting settings', () => {
  let stored = ''
  const storage = {
    getItem: (name: string) => (name === KEYBOARD_STORAGE_KEY ? stored : null),
    setItem: (_name: string, value: string) => {
      stored = value
    },
  }
  const overrides = { sidebar: ['Alt+Shift+KeyB'], help: [] }
  writeShortcutOverrides(storage, overrides)
  assert.deepEqual(readShortcutOverrides(storage), overrides)
  assert.deepEqual(
    readShortcutOverrides({
      getItem: () => {
        throw new Error('blocked')
      },
    }),
    {},
  )
  for (const value of [
    'broken',
    '{"version":2,"overrides":{}}',
    '{"version":1,"overrides":{"help":["Tab"]}}',
    '{"version":1,"overrides":{"help":["Ctrl+KeyL"]}}',
  ]) {
    stored = value
    assert.deepEqual(readShortcutOverrides(storage), {})
  }
})

test('preview resize uses axis ownership, small and large steps, bounds, and input-method guards', () => {
  assert.deepEqual(keyboardViewportSize(key('ArrowRight'), 'x', 390, 640), { width: 391, height: 640 })
  assert.equal(keyboardViewportSize(key('ArrowUp'), 'x', 390, 640), null)
  assert.deepEqual(keyboardViewportSize(key('ArrowDown', { shiftKey: true }), 'both', 390, 640), {
    width: 390,
    height: 650,
  })
  assert.deepEqual(keyboardViewportSize(key('Home'), 'both', 390, 640), { width: 240, height: 180 })
  assert.deepEqual(keyboardViewportSize(key('End'), 'y', 390, 640), { width: 390, height: 4096 })
  assert.deepEqual(keyboardViewportSize(key('ArrowLeft'), 'x', 240, 640), { width: 240, height: 640 })
  for (const extra of [
    { isComposing: true },
    { keyCode: 229 },
    { ctrlKey: true },
    { metaKey: true },
    { altKey: true },
    { defaultPrevented: true },
  ])
    assert.equal(keyboardViewportSize(key('ArrowRight', extra), 'both', 390, 640), null)
})
