import { describe, expect, test } from 'bun:test'

import type { ModelCatalogEntry, ProviderAdapterDiscovery } from '../lib/agenaApi'
import {
  buildAdaptersPatchFromDraftSelection,
  discoveryMatchedModels,
  discoveryUnmatchedModels,
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

function discovery(adapterId: string): ProviderAdapterDiscovery {
  return {
    adapter_id: adapterId,
    enabled: true,
    supported: true,
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

  test('discoveryMatchedModels and discoveryUnmatchedModels split model ids by catalog presence', () => {
    const entries = [officialEntry('gpt-5', 'GPT-5 Official')]
    const result = discovery('openai')

    expect(discoveryMatchedModels(entries, result).map((model) => model.id)).toEqual(['gpt-5'])
    expect(discoveryUnmatchedModels(entries, result).map((model) => model.id)).toEqual(['gpt-unknown'])
  })

  test('preferredCatalogEntryForProviderModel uses canonical catalog ids from discovery models', () => {
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
      discoveries: [discovery('openai'), discovery('anthropic')],
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
    expect(patch.openai?.models?.['gpt-unknown']).toEqual({})
    expect('gpt-unknown' in (patch.anthropic?.models || {})).toBe(false)
  })

  test('buildAdaptersPatchFromDraftSelection matches canonical default model ids', () => {
    const patch = buildAdaptersPatchFromDraftSelection({
      catalogEntries: [officialEntry('gpt-oss-120b', 'GPT OSS 120B')],
      discoveries: [
        {
          adapter_id: 'openai',
          enabled: true,
          supported: true,
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
})
