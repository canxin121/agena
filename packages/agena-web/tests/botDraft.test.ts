import { describe, expect, test } from 'bun:test'
import { botDraftFields, instructionsBytes, MAX_INSTRUCTIONS_BYTES } from '../src/features/bots/draft'
import { emptyBotPersona } from '../src/lib/bots'

describe('Bot editor draft', () => {
  test('counts UTF-8 bytes at and beyond the limit', () => {
    // The storage ceiling is generous: a realistic long character sheet fits,
    // and only a pathological payload exceeds it.
    expect(instructionsBytes('a'.repeat(MAX_INSTRUCTIONS_BYTES))).toBe(MAX_INSTRUCTIONS_BYTES)
    const cjk = Math.floor(MAX_INSTRUCTIONS_BYTES / 3)
    expect(instructionsBytes('界'.repeat(cjk))).toBeLessThanOrEqual(MAX_INSTRUCTIONS_BYTES)
    expect(instructionsBytes('界'.repeat(cjk + 1))).toBeGreaterThan(MAX_INSTRUCTIONS_BYTES)
    expect(instructionsBytes('😀'.repeat(MAX_INSTRUCTIONS_BYTES / 4 + 1))).toBeGreaterThan(MAX_INSTRUCTIONS_BYTES)
    expect(instructionsBytes('你是一只鲸鱼娘。'.repeat(400))).toBeLessThan(MAX_INSTRUCTIONS_BYTES)
  })
  test('clearing the editor sends an empty instructions field and no other persona fields', () => {
    expect(emptyBotPersona()).toEqual({ instructions: '' })
    expect(botDraftFields(' Atlas ', '')).toEqual({ name: 'Atlas', persona: { instructions: '' } })
    expect(botDraftFields('Atlas', 'Precise')).toEqual({ name: 'Atlas', persona: { instructions: 'Precise' } })
  })
})
