import { describe, expect, test } from 'bun:test'
import {
  minuteText,
  parseMinute,
  profileChanged,
  validPreferences,
  preferencePatch,
  rebasePreferences,
} from '../src/features/assistant/profileForm'
import type { AssistantPreferences } from '../src/generated/agenaState'

function profile(): AssistantPreferences {
  return {
    name: 'Agena',
    address_user: '',
    tone: 'balanced',
    language: 'auto',
    instructions: '',
    memory_enabled: true,
    proactive_enabled: true,
    timezone: 'UTC',
    quiet_enabled: false,
    quiet_start_minute: 1320,
    quiet_end_minute: 480,
    daily_limit: 4,
    cooldown_seconds: 300,
  }
}

describe('workspace assistant settings', () => {
  test('explicit conflict reconciliation preserves unrelated newer preferences', () => {
    const before = profile(),
      draft = { ...before, name: '小鱼' }
    const latest = { ...before, timezone: 'Asia/Shanghai', daily_limit: 2 }
    expect(preferencePatch(before, draft)).toEqual({ name: '小鱼' })
    expect(rebasePreferences(before, draft, latest)).toEqual({ ...latest, name: '小鱼' })
  })
  test('quiet-hour editing round-trips without accepting malformed local times', () => {
    for (const minutes of [0, 1, 59, 60, 480, 1320, 1439]) expect(parseMinute(minuteText(minutes))).toBe(minutes)
    for (const text of ['24:00', '12:60', '-1:00', '8:00', '08:0', '', '08:00:00', '08:00Z'])
      expect(parseMinute(text)).toBeNull()
  })
  test('invalid or unbounded contact settings cannot be saved from the form', () => {
    expect(validPreferences(profile())).toBe(true)
    for (const patch of [
      { name: '' },
      { daily_limit: 0 },
      { daily_limit: 25 },
      { cooldown_seconds: 1 },
      { cooldown_seconds: 86401 },
      { quiet_start_minute: 1440 },
      { quiet_enabled: true, quiet_start_minute: 480, quiet_end_minute: 480 },
      { instructions: '中'.repeat(1400) },
    ])
      expect(validPreferences({ ...profile(), ...patch })).toBe(false)
    expect(validPreferences({ ...profile(), quiet_enabled: true })).toBe(true)
  })
  test('turning off recall or proactive contact is an explicit dirty profile', () => {
    const original = profile()
    expect(profileChanged(original, { ...original })).toBe(false)
    expect(profileChanged(original, { ...original, memory_enabled: false })).toBe(true)
    expect(profileChanged(original, { ...original, proactive_enabled: false })).toBe(true)
    expect(original.memory_enabled).toBe(true)
  })
})
