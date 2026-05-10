import { describe, expect, test } from 'bun:test'
import { ref } from 'vue'

import { useRuntimeProviderActions } from './useRuntimeProviderActions'

function createState() {
  const calls: string[] = []
  const state = {
    actionError: ref(''),
    actionMessage: ref(''),
    drafts: {
      anthropic: ' sk-ant-123 ',
      openai: ' ',
    } as Record<string, string>,
    load: async () => {
      calls.push('load')
    },
  }

  return { calls, state }
}

describe('useRuntimeProviderActions', () => {
  test('saveApiKey trims input, clears draft, and reloads', async () => {
    const { calls, state } = createState()
    const actions = useRuntimeProviderActions(state, {
      deleteProviderCredential: async () => {},
      refreshProviderCredential: async () => {},
      setProviderApiKey: async (providerId, apiKey) => {
        calls.push(`setProviderApiKey:${providerId}:${apiKey}`)
      },
    })

    await actions.saveApiKey('anthropic')
    await actions.saveApiKey('openai')

    expect(calls).toEqual([
      'setProviderApiKey:anthropic:sk-ant-123',
      'load',
    ])
    expect(state.drafts.anthropic).toBe('')
    expect(state.actionMessage.value).toBe('Saved API key for anthropic.')
  })

  test('clearCredential and refreshCredential reload state', async () => {
    const { calls, state } = createState()
    const actions = useRuntimeProviderActions(state, {
      deleteProviderCredential: async (providerId) => {
        calls.push(`deleteProviderCredential:${providerId}`)
      },
      refreshProviderCredential: async (providerId) => {
        calls.push(`refreshProviderCredential:${providerId}`)
      },
      setProviderApiKey: async () => {},
    })

    await actions.clearCredential('anthropic')
    await actions.refreshCredential('openai')

    expect(calls).toEqual([
      'deleteProviderCredential:anthropic',
      'load',
      'refreshProviderCredential:openai',
      'load',
    ])
    expect(state.actionMessage.value).toBe('Requested credential refresh for openai.')
  })
})
