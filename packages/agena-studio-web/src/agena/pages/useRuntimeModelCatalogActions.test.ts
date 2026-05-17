import { describe, expect, test } from 'bun:test'
import { ref } from 'vue'

import type { ModelCatalogEntry, ModelCatalogEntryWriteRequest, ProviderModel } from '../lib/agenaApi'
import {
  buildConfiguredProviderModelFromDraft,
  buildModelCatalogWriteRequest,
  catalogLookupIdForProviderModel,
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
      output_modalities: ['text', 'image'],
      pricing: {
        input_usd_per_million_tokens: '1.25',
        output_usd_per_million_tokens: '10',
      },
      limits: {
        context_window_tokens: 400000,
        max_input_tokens: 300000,
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
    thinking_modes: {
      high: {
        display_name: 'High',
        description: 'More reasoning',
      },
    },
    speed_modes: {
      fast: {
        display_name: 'Fast',
        description: 'Priority route',
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
    max_input_tokens: 200000,
    max_output_tokens: 8192,
    description: 'Catalog metadata',
    capabilities: {
      features: {
        supported: ['tool_calling', 'streaming'],
      },
    },
    thinking_modes: {
      balanced: {
        display_name: 'Balanced',
        description: 'Default reasoning profile',
      },
    },
    speed_modes: {
      fast: {
        display_name: 'Fast',
        description: 'Priority route',
      },
    },
    ...overrides,
  }
}

describe('useRuntimeModelCatalogActions', () => {
  test('createModelCatalogDraftFromEntry hydrates thinking and speed modes for editing', () => {
    const draft = createModelCatalogDraftFromEntry(
      sampleCatalogEntry({
        model_id: 'claude-sonnet-4-6',
        display_name: 'Claude Sonnet 4.6',
        thinking_modes: {
          deep: {
            display_name: 'Deep',
            description: 'More reasoning',
            thinking: { type: 'budget', budget_tokens: 30000 },
            disabled: true,
          },
        },
        speed_modes: {
          fast: {
            display_name: 'Fast',
            description: 'Priority route',
            request_override: {
              headers: { 'anthropic-beta': 'fast-mode-2026-02-01' },
            },
            adapter_overrides: {
              openai: {
                body_patch: { service_tier: 'priority' },
              },
            },
          },
        },
      }),
    )

    expect(draft.thinking_modes).toEqual([
      {
        name: 'deep',
        display_name: 'Deep',
        description: 'More reasoning',
        disabled: true,
        thinking_json: JSON.stringify({ type: 'budget', budget_tokens: 30000 }, null, 2),
      },
    ])
    expect(draft.speed_modes).toEqual([
      {
        name: 'fast',
        display_name: 'Fast',
        description: 'Priority route',
        disabled: false,
        request_override_json: JSON.stringify(
          {
            headers: { 'anthropic-beta': 'fast-mode-2026-02-01' },
          },
          null,
          2,
        ),
        adapter_overrides_json: JSON.stringify(
          {
            openai: {
              body_patch: { service_tier: 'priority' },
            },
          },
          null,
          2,
        ),
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
      max_input_tokens: '300000',
      max_output_tokens: '16384',
      display_name: 'GPT-5',
      origin: '',
      description: 'Latest flagship model',
      output_modalities_json: JSON.stringify(['text', 'image'], null, 2),
      pricing_json: JSON.stringify(
        {
          input_usd_per_million_tokens: '1.25',
          output_usd_per_million_tokens: '10',
        },
        null,
        2,
      ),
      tool_calling: true,
      streaming: true,
      reasoning: true,
      structured_output: true,
      temperature_supported: false,
      thinking_modes: [
        {
          name: 'high',
          display_name: 'High',
          description: 'More reasoning',
          disabled: false,
          thinking_json: '',
        },
      ],
      speed_modes: [
        {
          name: 'fast',
          display_name: 'Fast',
          description: 'Priority route',
          disabled: false,
          request_override_json: '',
          adapter_overrides_json: '',
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
    expect(fromCustom.max_input_tokens).toBe('200000')

    const fromCatalog = createModelCatalogDraftFromProviderSelection([sampleCatalogEntry()], sampleProviderModel())
    expect(fromCatalog.display_name).toBe('GPT-5 Catalog')
    expect(fromCatalog.origin).toBe('OpenAI')
    expect(fromCatalog.lifecycle).toBe('preview')
    expect(fromCatalog.context_window_tokens).toBe('256000')
    expect(fromCatalog.max_input_tokens).toBe('200000')
    expect(fromCatalog.thinking_modes[0]?.name).toBe('balanced')
    expect(fromCatalog.speed_modes[0]?.name).toBe('fast')
  })

  test('catalogLookupIdForProviderModel prefers canonical ids when provided', () => {
    expect(
      catalogLookupIdForProviderModel(
        sampleProviderModel({
          id: 'openai/gpt-oss-120b',
          catalog_model_id: 'gpt-oss-120b',
        }),
      ),
    ).toBe('gpt-oss-120b')
  })

  test('buildModelCatalogWriteRequest preserves split thinking and speed modes and omits them when absent', () => {
    const draft = createEmptyModelCatalogDraft('anthropic', 'claude-sonnet-4-6')
    draft.origin = 'Anthropic'
    draft.max_input_tokens = '200000'
    draft.output_modalities_json = '["text","audio"]'
    draft.pricing_json = '{"input_usd_per_million_tokens":"3","output_usd_per_million_tokens":"15"}'
    draft.thinking_modes.push({
      name: ' deep ',
      display_name: ' Deep ',
      description: ' More reasoning ',
      disabled: true,
      thinking_json: '{"type":"budget","budget_tokens":30000}',
    })
    draft.speed_modes.push({
      name: ' fast ',
      display_name: ' Fast ',
      description: ' Priority route ',
      disabled: true,
      request_override_json:
        '{"headers":{"anthropic-beta":"fast-mode-2026-02-01"},"body_patch":{"service_tier":"priority"}}',
      adapter_overrides_json:
        '{"anthropic":{"headers":{"anthropic-beta":"fast-mode-2026-02-01"}},"openai":{"body_patch":{"service_tier":"priority"}}}',
    })

    const request = buildModelCatalogWriteRequest(draft)

    expect(request.origin).toBe('Anthropic')
    expect(request.max_input_tokens).toBe(200000)
    expect(request.output_modalities).toEqual(['text', 'audio'])
    expect(request.pricing).toEqual({
      input_usd_per_million_tokens: '3',
      output_usd_per_million_tokens: '15',
    })
    expect(request.thinking_modes).toEqual({
      deep: {
        display_name: 'Deep',
        description: 'More reasoning',
        thinking: { type: 'budget', budget_tokens: 30000 },
        disabled: true,
      },
    })
    expect(request.speed_modes).toEqual({
      fast: {
        display_name: 'Fast',
        description: 'Priority route',
        request_override: {
          headers: { 'anthropic-beta': 'fast-mode-2026-02-01' },
          body_patch: { service_tier: 'priority' },
        },
        adapter_overrides: {
          anthropic: {
            headers: { 'anthropic-beta': 'fast-mode-2026-02-01' },
          },
          openai: {
            body_patch: { service_tier: 'priority' },
          },
        },
        disabled: true,
      },
    })

    const emptyRequest = buildModelCatalogWriteRequest(createEmptyModelCatalogDraft('shared', 'openai/gpt-5'))
    expect(emptyRequest.thinking_modes).toBe(undefined)
    expect(emptyRequest.speed_modes).toBe(undefined)
  })

  test('buildConfiguredProviderModelFromDraft drops display-only origin metadata', () => {
    const draft = createEmptyModelCatalogDraft('openai', 'gpt-5')
    draft.display_name = 'GPT-5'
    draft.origin = 'OpenAI'

    expect(buildConfiguredProviderModelFromDraft(draft)).toEqual({
      display_name: 'GPT-5',
    })
  })

  test('saveCatalogEntryAction validates thinking and speed mode payloads before submitting', async () => {
    const calls: string[] = []
    const state = {
      actionError: ref(''),
      actionMessage: ref(''),
      load: async () => {
        calls.push('load')
      },
    }
    const emptyResponse = {
      entry_count: 0,
      official_entry_count: 0,
      custom_entry_count: 0,
    }
    const actions = useRuntimeModelCatalogActions(state, {
      deleteModelCatalogEntry: async () => emptyResponse,
      refreshModelCatalog: async () => emptyResponse,
      upsertModelCatalogEntry: async () => {
        calls.push('upsert')
        return emptyResponse
      },
    })

    const invalidThinkingDraft = createEmptyModelCatalogDraft('shared', 'openai/gpt-5')
    invalidThinkingDraft.thinking_modes.push({
      name: 'deep',
      display_name: '',
      description: '',
      disabled: false,
      thinking_json: '{',
    })
    await actions.saveCatalogEntryAction(invalidThinkingDraft)
    expect(calls).toEqual([])
    expect(state.actionError.value).toBe('Thinking mode deep must be valid JSON.')

    state.actionError.value = ''
    const invalidSpeedDraft = createEmptyModelCatalogDraft('shared', 'openai/gpt-5')
    invalidSpeedDraft.speed_modes.push({
      name: 'fast',
      display_name: '',
      description: '',
      disabled: false,
      request_override_json: '{',
      adapter_overrides_json: '',
    })
    await actions.saveCatalogEntryAction(invalidSpeedDraft)
    expect(calls).toEqual([])
    expect(state.actionError.value).toBe('Speed mode fast request override must be valid JSON.')
  })

  test('saveCatalogEntryAction preserves live-model modes when saving an override', async () => {
    const calls: string[] = []
    const actionError = ref('')
    const actionMessage = ref('')
    let capturedRequest: ModelCatalogEntryWriteRequest | null = null

    const actions = useRuntimeModelCatalogActions(
      {
        actionError,
        actionMessage,
        load: async () => {
          calls.push('load')
        },
      },
      {
        deleteModelCatalogEntry: async () => ({ entry_count: 0, official_entry_count: 0, custom_entry_count: 0 }),
        refreshModelCatalog: async () => ({ entry_count: 0, official_entry_count: 0, custom_entry_count: 0 }),
        upsertModelCatalogEntry: async (request) => {
          capturedRequest = request
          calls.push('upsert')
          return {
            entry_count: 1,
            official_entry_count: 0,
            custom_entry_count: 1,
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
      max_input_tokens: 300000,
      max_output_tokens: 16384,
      display_name: 'GPT-5',
      origin: null,
      description: 'Latest flagship model',
      output_modalities: ['text', 'image'],
      pricing: {
        input_usd_per_million_tokens: '1.25',
        output_usd_per_million_tokens: '10',
      },
      features: {
        supported: ['tool_calling', 'streaming', 'reasoning', 'structured_output'],
      },
      thinking_modes: {
        high: {
          display_name: 'High',
          description: 'More reasoning',
        },
      },
      speed_modes: {
        fast: {
          display_name: 'Fast',
          description: 'Priority route',
        },
      },
    })
    expect(actionError.value).toBe('')
    expect(actionMessage.value).toBe('Saved catalog entry gpt-5.')
  })
})
