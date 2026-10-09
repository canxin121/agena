import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'
import { fileURLToPath } from 'node:url'

import { isImeCommitEnter, isImeComposing } from '../src/lib/imeKeyboard'

function readSource(relative: string): string {
  return readFileSync(fileURLToPath(new URL(relative, import.meta.url)), 'utf8')
}

test('an IME confirmation key is recognized from either browser signal', () => {
  assert.equal(isImeComposing({ isComposing: true }), true)
  assert.equal(isImeComposing({ keyCode: 229 }), true)
  assert.equal(isImeComposing({ isComposing: false, keyCode: 13 }), false)
  assert.equal(isImeComposing(null), false)
  assert.equal(isImeComposing(undefined), false)
})

test('an Enter delivered next to the composition end still belongs to the IME', () => {
  const enterAt = (timeStamp: number) => ({ key: 'Enter', timeStamp })
  // Delivered right after `compositionend`, or before it in the same task.
  assert.equal(isImeCommitEnter(enterAt(100), 100), true)
  assert.equal(isImeCommitEnter(enterAt(1030), 1010), true)
  assert.equal(isImeCommitEnter(enterAt(990), 1000), true)
  // A deliberate later Enter, another key, or a chord is application input.
  assert.equal(isImeCommitEnter(enterAt(1130), 1010), false)
  assert.equal(isImeCommitEnter({ key: 'a', timeStamp: 100 }, 100), false)
  assert.equal(isImeCommitEnter({ key: 'Enter', timeStamp: 100, ctrlKey: true }, 100), false)
  // Without a composition timeline (or with synthetic zero timestamps) the
  // timing rule must stay silent.
  assert.equal(isImeCommitEnter(enterAt(100), Number.NEGATIVE_INFINITY), false)
  assert.equal(isImeCommitEnter({ key: 'Enter', timeStamp: 0 }, 0), false)
  assert.equal(isImeCommitEnter(null, 100), false)
})

test('the composition guard keeps composing keys away from application handlers', () => {
  const source = readSource('../src/lib/imeKeyboard.ts')
  const guard = source.slice(source.indexOf('export function installImeCompositionGuard'))
  // Capture phase at the window: the guard runs before any component handler.
  assert.ok(guard.includes("root.addEventListener('keydown', listener, true)"))
  assert.ok(guard.includes("root.removeEventListener('keydown', listener, true)"))
  assert.ok(guard.includes('event.stopPropagation()'))
  // Only propagation is stopped: the input method must keep the key so the
  // committed text still reaches the field.
  assert.ok(!guard.includes('preventDefault'))
  // The composition end is tracked so a confirmation key delivered after it is
  // still attributed to the input method.
  assert.ok(guard.includes("root.addEventListener('compositionend', onCompositionEnd, true)"))
  assert.ok(guard.includes('isImeCommitEnter(event, compositionEndedAt)'))
  // Limited to the text-entry targets an IME can compose into.
  assert.ok(source.includes("target.tagName === 'INPUT'"))
  assert.ok(source.includes("target.tagName === 'TEXTAREA'"))
  assert.ok(source.includes('target.isContentEditable'))
})

test('the app installs the composition guard before mounting', () => {
  const source = readSource('../src/main.ts')
  assert.ok(source.includes("from './lib/imeKeyboard'"))
  assert.ok(source.includes('installImeCompositionGuard()'))
  assert.ok(
    source.indexOf('installImeCompositionGuard()') < source.indexOf('createApp(App)'),
    'the guard must be installed before the app mounts',
  )
})

test('surfaces that own Enter during typing check the composition state', () => {
  for (const file of [
    '../src/components/chat/AgenaInteractionPart.vue',
    '../src/pages/chat/useChatCommands.ts',
    '../src/pages/chat/useChatTranscriptVim.ts',
  ]) {
    assert.ok(readSource(file).includes('isImeComposing(event)'), file)
  }
})
