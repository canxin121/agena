import assert from 'node:assert/strict'
import test from 'node:test'

import { copyTextToClipboard } from '../src/lib/clipboard'
import { ensureBrowserTestRuntime } from './testRuntime'

// The fallback path (non-secure contexts) selects a temporary textarea, and that
// selection takes focus away from whatever the user was typing in.

type FakeFocusable = {
  isConnected: boolean
  focused: boolean
  focus: () => void
}

function installClipboardFallbackDom() {
  ensureBrowserTestRuntime()
  const globals = globalThis as unknown as Record<string, unknown>
  const documentRecord = globals.document as Record<string, unknown>
  const windowRecord = globals.window as Record<string, unknown>

  const originalCreateElement = documentRecord.createElement
  const originalActiveElement = documentRecord.activeElement
  const originalExecCommand = documentRecord.execCommand
  const originalBody = documentRecord.body
  const originalIsSecureContext = windowRecord.isSecureContext
  const originalHTMLElement = globals.HTMLElement

  class FakeHTMLElement implements FakeFocusable {
    isConnected = true
    focused = false
    focus() {
      this.focused = true
    }
  }

  const active = new FakeHTMLElement()
  const textarea = {
    style: {} as Record<string, string>,
    value: '',
    attributes: {} as Record<string, string>,
    isConnected: true,
    setAttribute(name: string, value: string) {
      textarea.attributes[name] = value
    },
    select() {},
    setSelectionRange() {},
  }
  const appended: unknown[] = []
  const removed: unknown[] = []

  globals.HTMLElement = FakeHTMLElement
  documentRecord.createElement = () => textarea
  documentRecord.body = {
    appendChild: (node: unknown) => {
      appended.push(node)
      return node
    },
    removeChild: (node: unknown) => {
      removed.push(node)
      return node
    },
  }
  documentRecord.activeElement = active
  documentRecord.execCommand = () => true
  // Without a secure context the modern clipboard API is skipped, which is the
  // fallback this test covers.
  windowRecord.isSecureContext = false

  return {
    active: active as FakeFocusable,
    textarea,
    appended,
    removed,
    restore() {
      globals.HTMLElement = originalHTMLElement
      documentRecord.createElement = originalCreateElement
      documentRecord.activeElement = originalActiveElement
      documentRecord.execCommand = originalExecCommand
      if (originalBody === undefined) delete documentRecord.body
      else documentRecord.body = originalBody
      windowRecord.isSecureContext = originalIsSecureContext
    },
  }
}

test('the clipboard fallback hands focus back to the element it borrowed it from', async () => {
  const dom = installClipboardFallbackDom()
  try {
    assert.equal(await copyTextToClipboard('pane text'), true)
    assert.deepEqual(dom.appended, [dom.textarea])
    assert.deepEqual(dom.removed, [dom.textarea])
    assert.equal(dom.active.focused, true)
  } finally {
    dom.restore()
  }
})

test('a disconnected previous element is not focused again', async () => {
  const dom = installClipboardFallbackDom()
  try {
    dom.active.isConnected = false
    assert.equal(await copyTextToClipboard('pane text'), true)
    assert.equal(dom.active.focused, false)
  } finally {
    dom.restore()
  }
})
