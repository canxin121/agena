import { describe, expect, test } from 'bun:test'
import {
  BOT_INSTRUCTIONS_ID,
  botDraftFields,
  botPromptOrder,
  instructionsBytes,
  MAX_INSTRUCTIONS_BYTES,
} from '../src/features/bots/draft'
import { emptyPromptLayer, type BuiltinPrompt } from '../src/lib/promptLayers'
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
  test('legacy Bot instructions appear once and move alongside built-in and saved prompts', () => {
    const id = 'agena.system.identity'
    const builtins: BuiltinPrompt[] = [{ id, title: 'Identity', body: 'Agena', when: 'always' }]
    const legacy = { instructions: 'Old Bot instructions', prompts: { ids: ['style'] } }
    expect(botPromptOrder(legacy, builtins, emptyPromptLayer())).toEqual([id, 'style', BOT_INSTRUCTIONS_ID])
    const ordered = { ...legacy, prompts: { ids: [BOT_INSTRUCTIONS_ID, id, 'style'] } }
    expect(botPromptOrder(ordered, builtins, emptyPromptLayer())).toEqual([BOT_INSTRUCTIONS_ID, id, 'style'])
    expect(botPromptOrder({ ...ordered, instructions: '' }, builtins, emptyPromptLayer())).toEqual([id, 'style'])
  })
})
