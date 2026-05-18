import { describe, expect, test } from 'bun:test'

import type { ModelCatalogEntry, ProviderAdapterModels } from '../lib/agenaApi'
import {
  adapterModelsMatchedModels,
  adapterModelsUnmatchedModels,
  buildAdaptersPatchFromDraftSelection,
  configuredProviderModelDefinitions,
  preferredCatalogEntryForProviderModel,
  preferredCatalogEntryForModelId,
} from './providersSettingsModel'

function officialEntry(modelId: string, displayName: string): ModelCatalogEntry {
  return {
    model_id: modelId,
    kind: 'official',
    source: 'generated',
    display_name: displayName,
    description: `${displayName} description`,
    origin: 'OpenAI',
    capabilities: {
      features: {
        supported: ['streaming'],
      },
    },
  }
}

function customEntry(modelId: string, displayName: string): ModelCatalogEntry {
  return {
    model_id: modelId,
    kind: 'custom',
    source: 'custom',
    display_name: displayName,
    description: `${displayName} override`,
    origin: 'OpenAI',
  }
}

function adapterModels(adapterId: string): ProviderAdapterModels {
  return {
    adapter_id: adapterId,
    enabled: true,
    models: [
      { provider_id: 'gateway', adapter_id: adapterId, id: 'gpt-5', display_name: 'GPT-5' },
      { provider_id: 'gateway', adapter_id: adapterId, id: 'gpt-unknown', display_name: 'Unknown GPT' },
    ],
  }
}

describe('providersSettingsModel', () => {
  test('preferredCatalogEntryForModelId prefers custom overrides', () => {
    const selected = preferredCatalogEntryForModelId(
      [officialEntry('gpt-5', 'GPT-5 Official'), customEntry('gpt-5', 'GPT-5 Custom')],
      'gpt-5',
    )

    expect(selected?.kind).toBe('custom')
    expect(selected?.display_name).toBe('GPT-5 Custom')
  })

  test('adapterModelsMatchedModels and adapterModelsUnmatchedModels split model ids by catalog presence', () => {
    const entries = [officialEntry('gpt-5', 'GPT-5 Official')]
    const result = adapterModels('openai')

    expect(adapterModelsMatchedModels(entries, result).map((model) => model.id)).toEqual(['gpt-5'])
    expect(adapterModelsUnmatchedModels(entries, result).map((model) => model.id)).toEqual(['gpt-unknown'])
  })

  test('preferredCatalogEntryForProviderModel uses canonical catalog ids from listed adapter models', () => {
    const entry = preferredCatalogEntryForProviderModel(
      [officialEntry('gpt-oss-120b', 'GPT OSS 120B')],
      {
        provider_id: 'gateway',
        adapter_id: 'openai',
        id: 'openai/gpt-oss-120b',
        catalog_model_id: 'gpt-oss-120b',
        display_name: 'OpenAI GPT OSS 120B',
      },
    )

    expect(entry?.model_id).toBe('gpt-oss-120b')
  })

  test('buildAdaptersPatchFromDraftSelection auto-seeds catalog matches and preserves the default model', () => {
    const entries = [officialEntry('gpt-5', 'GPT-5 Official')]
    const patch = buildAdaptersPatchFromDraftSelection({
      catalogEntries: entries,
      adapterModelLists: [adapterModels('openai'), adapterModels('anthropic')],
      selectedAdapterIds: ['openai', 'anthropic'],
      defaultAdapterId: 'openai',
      defaultModelId: 'gpt-unknown',
    })

    expect(Object.keys(patch).sort()).toEqual(['anthropic', 'openai'])
    expect(patch.openai?.enabled).toBe(true)
    expect(patch.anthropic?.enabled).toBe(true)
    expect(patch.openai?.models?.['gpt-5']?.display_name).toBe('GPT-5 Official')
    expect(patch.openai?.models?.['gpt-5']?.description).toBe('GPT-5 Official description')
    expect(patch.anthropic?.models?.['gpt-5']?.display_name).toBe('GPT-5 Official')
    expect(patch.openai?.models?.['gpt-unknown']?.display_name).toBe('Unknown GPT')
    expect(patch.anthropic?.models?.['gpt-unknown']?.display_name).toBe('Unknown GPT')
  })

  test('buildAdaptersPatchFromDraftSelection matches canonical default model ids', () => {
    const patch = buildAdaptersPatchFromDraftSelection({
      catalogEntries: [officialEntry('gpt-oss-120b', 'GPT OSS 120B')],
      adapterModelLists: [
        {
          adapter_id: 'openai',
          enabled: true,
          models: [
            {
              provider_id: 'gateway',
              adapter_id: 'openai',
              id: 'openai/gpt-oss-120b',
              catalog_model_id: 'gpt-oss-120b',
              display_name: 'GPT OSS 120B',
            },
          ],
        },
      ],
      selectedAdapterIds: ['openai'],
      defaultAdapterId: 'openai',
      defaultModelId: 'openai/gpt-oss-120b',
      defaultCatalogModelId: 'gpt-oss-120b',
    })

    expect(patch.openai?.models?.['openai/gpt-oss-120b']?.display_name).toBe('GPT OSS 120B')
  })

  test('configuredProviderModelDefinitions falls back to weaker provider model metadata and empty objects', () => {
    const definitions = configuredProviderModelDefinitions([], [
      {
        provider_id: 'gateway',
        adapter_id: 'openai',
        id: 'fallback-model',
        display_name: 'Fallback Model',
      },
      {
        provider_id: 'gateway',
        adapter_id: 'openai',
        id: 'empty-model',
      },
    ])

    expect(definitions['fallback-model']?.display_name).toBe('Fallback Model')
    expect(definitions['empty-model']).toEqual({})
  })
})
