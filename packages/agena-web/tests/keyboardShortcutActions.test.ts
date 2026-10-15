import assert from 'node:assert/strict'
import test from 'node:test'
import { resolveAppShortcut } from '../src/app/runtime/keyboardShortcutActions'
import { navigationIndex } from '../src/lib/keyboardNavigation'

function key(key: string, extra: Record<string, unknown> = {}): KeyboardEvent {
  return {
    key,
    code: '',
    ctrlKey: false,
    metaKey: false,
    altKey: false,
    shiftKey: false,
    repeat: false,
    defaultPrevented: false,
    isComposing: false,
    keyCode: 0,
    ...extra,
  } as unknown as KeyboardEvent
}

test('application shortcuts leave handled events, IME candidates and held keys alone', () => {
  for (const extra of [{ defaultPrevented: true }, { isComposing: true }, { keyCode: 229 }, { repeat: true }]) {
    assert.equal(resolveAppShortcut(key('n', { ctrlKey: true, ...extra })), null)
  }
  assert.equal(resolveAppShortcut(key('n', { ctrlKey: true, shiftKey: true })), null)
  assert.equal(resolveAppShortcut(key('n', { ctrlKey: true, altKey: true })), null)
})

test('only mapped main-page numbers are consumed', () => {
  const tabs = ['chat', 'files', 'preview', 'terminal', 'git']
  for (const [index, tab] of tabs.entries()) {
    assert.deepEqual(resolveAppShortcut(key(String(index + 1), { metaKey: true })), { type: 'main-tab', tab })
    assert.deepEqual(resolveAppShortcut(key('!', { code: `Digit${index + 1}`, altKey: true, shiftKey: true })), {
      type: 'main-tab',
      tab,
    })
  }
  for (const digit of ['0', '6', '7', '8', '9', '1garbage']) {
    assert.equal(resolveAppShortcut(key(digit, { ctrlKey: true })), null)
  }
})

test('terminal input retains its control sequences and has application alternatives', () => {
  for (const letter of ['h', 'l', 'n', '1', '.', ',']) {
    assert.equal(resolveAppShortcut(key(letter, { ctrlKey: true }), true), null)
  }
  assert.deepEqual(resolveAppShortcut(key('.', { metaKey: true }), true), { type: 'help' })
  assert.deepEqual(resolveAppShortcut(key('I', { code: 'KeyI', altKey: true, shiftKey: true }), true), {
    type: 'composer',
  })
  assert.deepEqual(resolveAppShortcut(key('L', { code: 'KeyL', altKey: true, shiftKey: true }), true), {
    type: 'sidebar',
  })
  assert.equal(resolveAppShortcut(key('i', { ctrlKey: true })), null, 'Ctrl+I stays available to Vim')
})

test('list navigation enters from either edge, wraps and handles changing or empty lists', () => {
  assert.equal(navigationIndex('ArrowDown', -1, 4), 0)
  assert.equal(navigationIndex('ArrowUp', -1, 4), 3)
  assert.equal(navigationIndex('ArrowDown', 3, 4), 0)
  assert.equal(navigationIndex('ArrowUp', 0, 4), 3)
  assert.equal(navigationIndex('Home', 2, 4), 0)
  assert.equal(navigationIndex('End', 0, 4), 3)
  assert.equal(navigationIndex('ArrowDown', 0, 0), null)
  assert.equal(navigationIndex('Enter', 0, 4), null)
})
