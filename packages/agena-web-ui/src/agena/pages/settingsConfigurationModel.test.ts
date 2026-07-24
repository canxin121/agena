import { describe, expect, test } from 'bun:test'

import {
  createSettingsConfigurationDraft,
  parseSettingsConfigurationDraft,
  settingsConfigurationDraftChanged,
  valueAtSettingsPath,
  type SettingsConfigurationField,
} from './settingsConfigurationModel'

const integerField: SettingsConfigurationField = {
  path: 'runtime.session.cache.max_sessions',
  section: 'runtime',
  label: 'Cached sessions',
  description: 'test',
  kind: 'integer',
}

describe('settingsConfigurationModel', () => {
  test('reads nested settings without confusing missing and falsy values', () => {
    const root = { runtime: { reload: { enabled: false }, session: { cache: { max_sessions: 0 } } } }
    expect(valueAtSettingsPath(root, 'runtime.reload.enabled')).toBe(false)
    expect(valueAtSettingsPath(root, integerField.path)).toBe(0)
    expect(valueAtSettingsPath(root, 'runtime.missing')).toBe(undefined)
  })

  test('creates inherited and explicit drafts from the file settings', () => {
    expect(createSettingsConfigurationDraft({}, integerField)).toEqual({ override: false, value: '' })
    expect(
      createSettingsConfigurationDraft({ runtime: { session: { cache: { max_sessions: 12 } } } }, integerField),
    ).toEqual({ override: true, value: '12' })
  })

  test('parses integers and rejects partial numeric input', () => {
    expect(parseSettingsConfigurationDraft(integerField, { override: true, value: '42' })).toBe(42)
    let message = ''
    try {
      parseSettingsConfigurationDraft(integerField, { override: true, value: '4.2' })
    } catch (error) {
      message = error instanceof Error ? error.message : String(error)
    }
    expect(message.includes('must be a whole number')).toBe(true)
  })

  test('tracks changed overrides and inherited resets', () => {
    const root = { runtime: { session: { cache: { max_sessions: 12 } } } }
    expect(settingsConfigurationDraftChanged(root, integerField, { override: true, value: '12' })).toBe(false)
    expect(settingsConfigurationDraftChanged(root, integerField, { override: true, value: '13' })).toBe(true)
    expect(settingsConfigurationDraftChanged(root, integerField, { override: false, value: '' })).toBe(true)
  })
})
