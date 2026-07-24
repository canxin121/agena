import { describe, expect, test } from 'bun:test'

import { parseChatRunOptionValues } from './useChatSessionActions'

function capturedErrorMessage(action: () => unknown): string {
  try {
    action()
    return ''
  } catch (error) {
    return error instanceof Error ? error.message : String(error)
  }
}

describe('parseChatRunOptionValues', () => {
  test('normalizes all advanced TUI run settings', () => {
    expect(
      parseChatRunOptionValues({
        temperature: ' 0.25 ',
        maxOutput: ' 4096 ',
        system: '  Be concise.  ',
      }),
    ).toEqual({
      temperature: 0.25,
      maxOutputTokens: 4096,
      system: 'Be concise.',
    })
  })

  test('omits inherited values', () => {
    expect(parseChatRunOptionValues({ temperature: '', maxOutput: ' ', system: '\n' })).toEqual({
      temperature: undefined,
      maxOutputTokens: undefined,
      system: undefined,
    })
  })

  test('rejects invalid numeric overrides before submitting a run', () => {
    expect(
      capturedErrorMessage(() => parseChatRunOptionValues({ temperature: 'NaN', maxOutput: '', system: '' })),
    ).toBe('Temperature must be a finite number.')
    expect(
      capturedErrorMessage(() => parseChatRunOptionValues({ temperature: '', maxOutput: '1.5', system: '' })),
    ).toBe('Max output tokens must be a positive whole number.')
    expect(capturedErrorMessage(() => parseChatRunOptionValues({ temperature: '', maxOutput: '0', system: '' }))).toBe(
      'Max output tokens must be a positive whole number.',
    )
  })
})
