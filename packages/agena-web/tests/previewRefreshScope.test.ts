import { describe, expect, test } from 'bun:test'
import {
  createPreviewRefreshScope,
  forgetPreviewRefreshSession,
  readPreviewRefreshToken,
  refreshPreviewSession,
  refreshPreviewWindow,
} from '../src/stores/previewRefreshScope'

describe('preview refresh ownership', () => {
  test('reloading one pane does not reload another pane, even for the same service', () => {
    let state = createPreviewRefreshScope()
    state = refreshPreviewWindow(state, 'left')
    expect(readPreviewRefreshToken(state, 'left', 'app-a')).toBe(1)
    expect(readPreviewRefreshToken(state, 'right', 'app-a')).toBe(0)
    expect(readPreviewRefreshToken(state, 'right', 'app-b')).toBe(0)
    state = refreshPreviewWindow(state)
    expect(readPreviewRefreshToken(state, null, 'app-a')).toBe(2)
    expect(readPreviewRefreshToken(state, 'left', 'app-a')).toBe(1)
    expect(readPreviewRefreshToken(state, 'right', 'app-a')).toBe(0)
  })

  test('a service mutation refreshes its viewers without reloading unrelated services', () => {
    let state = refreshPreviewWindow(createPreviewRefreshScope(), 'right')
    const before = readPreviewRefreshToken(state, 'right', 'app-b')
    state = refreshPreviewSession(state, 'app-a')
    expect(readPreviewRefreshToken(state, 'left', 'app-a')).toBe(2)
    expect(readPreviewRefreshToken(state, 'other', 'app-a')).toBe(2)
    expect(readPreviewRefreshToken(state, 'right', 'app-b')).toBe(before)
    state = forgetPreviewRefreshSession(state, 'app-a')
    expect(readPreviewRefreshToken(state, 'right', 'app-b')).toBe(before)
    expect(state.bySession['app-a']).toBeUndefined()
  })
})
