import { describe, expect, test } from 'bun:test'

import { openGlobalCommandPalette, setGlobalCommandPaletteOpenHandler } from './commandPaletteRegistry'

describe('commandPaletteRegistry', () => {
  test('invokes registered global open handler', () => {
    let calls = 0
    setGlobalCommandPaletteOpenHandler(() => {
      calls += 1
    })

    openGlobalCommandPalette()
    expect(calls).toBe(1)

    setGlobalCommandPaletteOpenHandler(null)
    openGlobalCommandPalette()
    expect(calls).toBe(1)
  })
})
