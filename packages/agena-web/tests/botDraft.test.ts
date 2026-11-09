import { describe, expect, test } from 'bun:test'
import { botDraftFields, instructionsBytes, MAX_INSTRUCTIONS_BYTES } from '../src/features/bots/draft'
import { emptyBotPersona } from '../src/lib/bots'

describe('Bot editor draft', () => {
  test('counts UTF-8 bytes at and beyond the limit', () => {
    expect(instructionsBytes('a'.repeat(4096))).toBe(MAX_INSTRUCTIONS_BYTES)
    expect(instructionsBytes('界'.repeat(1365) + 'a')).toBe(MAX_INSTRUCTIONS_BYTES)
    expect(instructionsBytes('😀'.repeat(1025))).toBeGreaterThan(MAX_INSTRUCTIONS_BYTES)
  })
  test('clearing the editor sends an empty instructions field and no other persona fields', () => {
    expect(emptyBotPersona()).toEqual({ instructions: '' })
    expect(botDraftFields(' Atlas ', '')).toEqual({ name: 'Atlas', persona: { instructions: '' } })
    expect(botDraftFields('Atlas', 'Precise')).toEqual({ name: 'Atlas', persona: { instructions: 'Precise' } })
  })
})
