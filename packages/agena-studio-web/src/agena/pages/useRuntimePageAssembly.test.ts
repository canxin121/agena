import { describe, expect, test } from 'bun:test'

import { replaceProviderModelsRecord } from './useRuntimePageAssembly'

describe('useRuntimePageAssembly', () => {
  test('replaceProviderModelsRecord replaces keys in place', () => {
    const providerModels = {
      anthropic: [{ provider_id: 'anthropic', id: 'claude-opus-4-7' }],
      openai: [{ provider_id: 'openai', id: 'gpt-4.1-mini' }],
    }

    replaceProviderModelsRecord(providerModels, {
      openai: [{ provider_id: 'openai', id: 'gpt-5.4' }],
    })

    expect(providerModels).toEqual({
      openai: [{ provider_id: 'openai', id: 'gpt-5.4' }],
    })
  })
})
