import { describe, expect, test } from 'bun:test'
import { settingsSessionId } from '../src/components/settings/settingsSessionScope'

describe('session-scoped settings target', () => {
  test('accepts one positive integer conversation id', () => {
    expect(settingsSessionId('42')).toBe('42')
    expect(settingsSessionId(42)).toBe('42')
    expect(settingsSessionId(' 7 ')).toBe('7')
  })

  test('rejects anything that is not a conversation id', () => {
    expect(settingsSessionId('')).toBe('')
    expect(settingsSessionId(null)).toBe('')
    expect(settingsSessionId(undefined)).toBe('')
    expect(settingsSessionId('abc')).toBe('')
    expect(settingsSessionId('0')).toBe('')
    expect(settingsSessionId('-1')).toBe('')
    expect(settingsSessionId('1.5')).toBe('')
  })

  test('takes the first usable entry of a repeated query value', () => {
    expect(settingsSessionId(['', '12'])).toBe('12')
  })
})
