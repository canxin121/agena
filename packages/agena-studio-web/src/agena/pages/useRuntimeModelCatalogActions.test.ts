import { describe, expect, test } from 'bun:test'
import { ref } from 'vue'

import type { ModelCatalogEntry, ModelCatalogEntryWriteRequest, ProviderModel } from '../lib/agenaApi'
import {
  buildConfiguredProviderModelFromDraft,
  buildModelCatalogWriteRequest,
  createEmptyModelCatalogDraft,
  createModelCatalogDraftFromEntry,
  createModelCatalogDraftFromProviderModel,
  createModelCatalogDraftFromProviderSelection,
  useRuntimeModelCatalogActions,
} from './useRuntimeModelCatalogActions'

function sampleProviderModel(overrides: Partial<ProviderModel> = {}): ProviderModel {
  return {
    provider_id: 'openai',
    adapter_id: 'openai',
    id: 'gpt-5',
    display_name: 'GPT-5',
    metadata: {
      lifecycle: 'active',
      description: 'Latest flagship model',
      limits: {
        context_window_tokens: 400000,
        max_output_tokens: 16384,
      },
    },
    capabilities: {
      tool_calling: 'supported',
      streaming: 'supported',
      reasoning: 'supported',
      structured_output: 'supported',
      temperature_supported: 'unsupported',
    },
    variants: {
      high: {
        display_name: 'High',
        description: 'More reasoning',
      },
    },
    ...overrides,
  }
}

function sampleCatalogEntry(overrides: Partial<ModelCatalogEntry> = {}): ModelCatalogEntry {
  return {
    model_id: 'gpt-5',
    kind: 'official',
    source: 'generated',
    source_label: 'generated catalog',
    display_name: 'GPT-5 Catalog',
    origin: 'OpenAI',
    lifecycle: 'preview',
    context_window_tokens: 256000,
    max_output_tokens: 8192,
    description: 'Catalog metadata',
    capabilities: {
      features: {
        supported: ['tool_calling', 'streaming'],
      },
    },
    variants: {
      balanced: {
        display_name: 'Balanced',
        description: 'Default profile',
      },
    },
    ...overrides,
  }
}

describe('useRuntimeModelCatalogActions', () => {
  test('createModelCatalogDraftFromEntry hydrates model variants for editing', () => {
    const entry = sampleCatalogEntry({
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
    })

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

  test('createModelCatalogDraftFromProviderModel maps live provider metadata into the editable draft shape', () => {
    const draft = createModelCatalogDraftFromProviderModel(sampleProviderModel())

    expect(draft).toEqual({
      adapter_id: 'openai',
      model_id: 'gpt-5',
      lifecycle: 'active',
      context_window_tokens: '400000',
      max_output_tokens: '16384',
      display_name: 'GPT-5',
      origin: '',
      description: 'Latest flagship model',
      tool_calling: true,
      streaming: true,
      reasoning: true,
      structured_output: true,
      temperature_supported: false,
      variants: [
        {
          name: 'high',
          display_name: 'High',
          description: 'More reasoning',
          disabled: false,
          thinking_json: '',
        },
      ],
    })
  })

  test('createModelCatalogDraftFromProviderSelection prefers a local override, then catalog metadata, before raw provider metadata', () => {
    const fromCustom = createModelCatalogDraftFromProviderSelection(
      [
        sampleCatalogEntry(),
        sampleCatalogEntry({
          kind: 'custom',
          source: 'custom',
          display_name: 'Workspace GPT-5',
          description: 'Local override',
          capabilities: {
            features: {
              supported: ['tool_calling', 'streaming', 'reasoning'],
            },
          },
        }),
      ],
      sampleProviderModel(),
    )

    expect(fromCustom.display_name).toBe('Workspace GPT-5')
    expect(fromCustom.origin).toBe('OpenAI')
    expect(fromCustom.description).toBe('Local override')
    expect(fromCustom.reasoning).toBe(true)
    expect(fromCustom.context_window_tokens).toBe('256000')

    const fromCatalog = createModelCatalogDraftFromProviderSelection([sampleCatalogEntry()], sampleProviderModel())
    expect(fromCatalog.display_name).toBe('GPT-5 Catalog')
    expect(fromCatalog.origin).toBe('OpenAI')
    expect(fromCatalog.lifecycle).toBe('preview')
    expect(fromCatalog.context_window_tokens).toBe('256000')
    expect(fromCatalog.variants).toEqual([
      {
        name: 'balanced',
        display_name: 'Balanced',
        description: 'Default profile',
        disabled: false,
        thinking_json: '',
      },
    ])
  })

  test('buildModelCatalogWriteRequest preserves variants and omits them when absent', () => {
    const draft = createEmptyModelCatalogDraft('anthropic', 'claude-sonnet-4-6')
    draft.origin = 'Anthropic'
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

    expect(request.origin).toBe('Anthropic')
    expect(request.variants).toEqual({
      deep: {
        display_name: 'Deep',
        description: 'More reasoning',
        thinking: { type: 'budget', budget_tokens: 30000 },
        disabled: true,
      },
    })

    expect(buildModelCatalogWriteRequest(createEmptyModelCatalogDraft('shared', 'openai/gpt-5')).variants).toBe(
      undefined,
    )
  })

  test('buildConfiguredProviderModelFromDraft drops display-only origin metadata', () => {
    const draft = createEmptyModelCatalogDraft('openai', 'gpt-5')
    draft.display_name = 'GPT-5'
    draft.origin = 'OpenAI'

    expect(buildConfiguredProviderModelFromDraft(draft)).toEqual({
      display_name: 'GPT-5',
    })
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
      entries: [],
    }
    const actions = useRuntimeModelCatalogActions(state, {
      deleteModelCatalogEntry: async () => emptyResponse,
      refreshModelCatalog: async () => emptyResponse,
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

  test('saveCatalogEntryAction preserves live-model variants when saving an override', async () => {
    const calls: string[] = []
    const actionError = ref('')
    const actionMessage = ref('')
    const catalogEntries = ref<ModelCatalogEntry[]>([])
    let capturedRequest: ModelCatalogEntryWriteRequest | null = null

    const actions = useRuntimeModelCatalogActions(
      {
        actionError,
        actionMessage,
        catalogEntries,
        load: async () => {
          calls.push('load')
        },
      },
      {
        deleteModelCatalogEntry: async () => ({ entries: [] }),
        refreshModelCatalog: async () => ({ entries: [] }),
        upsertModelCatalogEntry: async (request) => {
          capturedRequest = request
          calls.push('upsert')
          return {
            entries: [sampleCatalogEntry({ kind: 'custom', source: 'custom' })],
          }
        },
      },
    )

    await actions.saveCatalogEntryAction(createModelCatalogDraftFromProviderModel(sampleProviderModel()))

    expect(calls).toEqual(['upsert', 'load'])
    expect(capturedRequest).toEqual({
      model_id: 'gpt-5',
      lifecycle: 'active',
      context_window_tokens: 400000,
      max_output_tokens: 16384,
      display_name: 'GPT-5',
      origin: null,
      description: 'Latest flagship model',
      features: {
        supported: ['tool_calling', 'streaming', 'reasoning', 'structured_output'],
      },
      variants: {
        high: {
          display_name: 'High',
          description: 'More reasoning',
        },
      },
    })
    expect(actionError.value).toBe('')
    expect(actionMessage.value).toBe('Saved catalog entry gpt-5.')
  })
})
