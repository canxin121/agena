import { describe, expect, test } from 'bun:test'
import { ref } from 'vue'

import type { ModelCatalogEntry } from '../lib/agenaApi'

import {
  buildModelCatalogWriteRequest,
  createEmptyModelCatalogDraft,
  createModelCatalogDraftFromEntry,
  useRuntimeModelCatalogActions,
} from './useRuntimeModelCatalogActions'

describe('useRuntimeModelCatalogActions', () => {
  test('createModelCatalogDraftFromEntry hydrates model variants for editing', () => {
    const entry = {
      provider_id: 'anthropic',
      model_id: 'claude-sonnet-4-6',
      display_name: 'Claude Sonnet 4.6',
      variants: {
        deep: {
          display_name: 'Deep',
          description: 'More reasoning',
          thinking: { type: 'budget', budget_tokens: 30000 },
          disabled: true,
        },
        light: {
          display_name: 'Light',
          description: 'Faster responses',
        },
      },
    } as ModelCatalogEntry

    const draft = createModelCatalogDraftFromEntry(entry)

    expect(draft.variants).toEqual([
      {
        name: 'deep',
        display_name: 'Deep',
        description: 'More reasoning',
        disabled: true,
        thinking_json: JSON.stringify({ type: 'budget', budget_tokens: 30000 }, null, 2),
      },
      {
        name: 'light',
        display_name: 'Light',
        description: 'Faster responses',
        disabled: false,
        thinking_json: '',
      },
    ])
  })

  test('buildModelCatalogWriteRequest preserves variants and omits them when absent', () => {
    const draft = createEmptyModelCatalogDraft('anthropic', 'claude-sonnet-4-6')
    draft.variants.push({
      name: ' deep ',
      display_name: ' Deep ',
      description: ' More reasoning ',
      disabled: true,
      thinking_json: '{"type":"budget","budget_tokens":30000}',
    })
    draft.variants.push({
      name: '',
      display_name: '',
      description: '',
      disabled: false,
      thinking_json: '',
    })

    const request = buildModelCatalogWriteRequest(draft)

    expect(request.variants).toEqual({
      deep: {
        display_name: 'Deep',
        description: 'More reasoning',
        thinking: { type: 'budget', budget_tokens: 30000 },
        disabled: true,
      },
    })

    expect(buildModelCatalogWriteRequest(createEmptyModelCatalogDraft('shared', 'openai/gpt-5')).variants).toBe(undefined)
  })

  test('saveCatalogEntryAction reports local variant validation errors before submitting', async () => {
    const calls: string[] = []
    const state = {
      actionError: ref(''),
      actionMessage: ref(''),
      catalogEntries: ref<ModelCatalogEntry[]>([]),
      load: async () => {
        calls.push('load')
      },
    }
    const emptyResponse = {
      remote_url: '',
      fallback_url: '',
      entries: [],
    }
    const actions = useRuntimeModelCatalogActions(state, {
      deleteModelCatalogEntry: async () => emptyResponse,
      refreshModelCatalog: async () => emptyResponse,
      setModelCatalogProviderDefault: async () => emptyResponse,
      upsertModelCatalogEntry: async () => {
        calls.push('upsert')
        return emptyResponse
      },
    })
    const draft = createEmptyModelCatalogDraft('shared', 'openai/gpt-5')
    draft.variants.push({
      name: 'deep',
      display_name: '',
      description: '',
      disabled: false,
      thinking_json: '{',
    })

    await actions.saveCatalogEntryAction(draft)

    expect(calls).toEqual([])
    expect(state.actionMessage.value).toBe('')
    expect(state.actionError.value).toBe('Variant deep thinking must be valid JSON.')
  })
})
