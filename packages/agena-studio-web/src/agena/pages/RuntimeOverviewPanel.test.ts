import { describe, expect, test } from 'bun:test'

import { renderVueSsr } from './test/renderVueSsr'

describe('RuntimeOverviewPanel', () => {
  test('renders catalog entry variant summaries and variant editor controls', async () => {
    const html = await renderVueSsr('/src/agena/pages/RuntimeOverviewPanel.vue', {
      catalogEntries: [
        {
          model_id: 'claude-sonnet-4-6',
          display_name: 'Claude Sonnet 4.6',
          description: 'Balanced model',
          variants: {
            deep: {
              display_name: 'Deep',
              description: 'More reasoning',
              thinking: { type: 'budget', budget_tokens: 20000 },
            },
          },
        },
      ],
      operatorCards: [],
      runtimeSnapshotFacts: [],
      runtime: null,
      providers: [],
      providerModels: {},
      sessionCacheFacts: [],
      formatProviderModel: (model: { id: string }) => model.id,
      load: async () => {},
    })

    expect(html.includes('Variants')).toBe(true)
    expect(html.includes('Add Variant')).toBe(true)
    expect(html.includes('deep')).toBe(true)
    expect(html.includes('More reasoning')).toBe(true)
    expect(html.includes('thinking')).toBe(true)
  })

  test('renders live provider models as one-click draft sources', async () => {
    const html = await renderVueSsr('/src/agena/pages/RuntimeOverviewPanel.vue', {
      catalogEntries: [
        {
          model_id: 'gpt-5',
          kind: 'official',
          source: 'remote',
          display_name: 'GPT-5 Catalog',
        },
      ],
      operatorCards: [{ label: 'Providers', value: '1' }],
      runtimeSnapshotFacts: [{ label: 'Workspace Root', value: '/repo', mono: true }],
      runtime: {
        reload: { enabled: true, interval_secs: 10 },
        janitor: { enabled: true, interval_secs: 60 },
        watch_paths: ['src'],
        automation: { recent_jobs: [], enabled: true, job_count: 0 },
        model_catalog: {
          remote_url: 'https://example.test/catalog.json',
          fallback_url: 'https://example.test/fallback.json',
          last_successful_source: 'remote',
          last_refresh_at: '2026-05-15T00:00:00Z',
          entries: [],
        },
      },
      providers: [
        {
          provider_id: 'openai',
          default_model: 'openai/gpt-5',
          adapters: [{ adapter_id: 'openai', enabled: true, configured_model_count: 2 }],
        },
      ],
      providerModels: {
        openai: [
          {
            provider_id: 'openai',
            id: 'openai/gpt-5',
            display_name: 'GPT-5',
          },
          {
            provider_id: 'openai',
            id: 'openai/gpt-5-mini',
            display_name: 'GPT-5 Mini',
          },
        ],
      },
      sessionCacheFacts: [{ label: 'Entries', value: '2' }],
      formatProviderModel: (model: { display_name?: string; id: string }) => model.display_name || model.id,
      load: async () => {},
    })

    expect(html.includes('Provider Defaults')).toBe(true)
    expect(html.includes('Live models:')).toBe(true)
    expect(html.includes('Bring to Draft: GPT-5')).toBe(true)
    expect(html.includes('Bring to Draft: GPT-5 Mini')).toBe(true)
    expect(html.includes('Blank Draft')).toBe(true)
    expect(html.includes('Use the live model buttons above for the fastest draft path')).toBe(true)
    expect(html.match(/Bring to Draft:/g)?.length).toBe(2)
  })

  test('renders official and custom entries for the same provider/model with distinct actions', async () => {
    const html = await renderVueSsr('/src/agena/pages/RuntimeOverviewPanel.vue', {
      catalogEntries: [
        {
          model_id: 'gpt-5',
          kind: 'official',
          source: 'remote',
          source_label: 'remote catalog',
          display_name: 'GPT-5 Official',
        },
        {
          model_id: 'gpt-5',
          kind: 'custom',
          source: 'custom',
          source_label: 'workspace override',
          display_name: 'GPT-5 Workspace',
        },
      ],
      operatorCards: [{ label: 'Providers', value: '1' }],
      runtimeSnapshotFacts: [{ label: 'Workspace Root', value: '/repo', mono: true }],
      runtime: {
        reload: { enabled: true, interval_secs: 10 },
        janitor: { enabled: true, interval_secs: 60 },
        watch_paths: ['src'],
        automation: { recent_jobs: [], enabled: true, job_count: 0 },
        model_catalog: {
          remote_url: 'https://example.test/catalog.json',
          fallback_url: 'https://example.test/fallback.json',
          last_successful_source: 'remote',
          last_refresh_at: '2026-05-15T00:00:00Z',
          entries: [],
        },
      },
      providers: [
        {
          provider_id: 'openai',
          default_model: 'openai/gpt-5',
          adapters: [{ adapter_id: 'openai', enabled: true, configured_model_count: 0 }],
        },
      ],
      providerModels: { openai: [] },
      sessionCacheFacts: [{ label: 'Entries', value: '2' }],
      formatProviderModel: (model: { display_name?: string; id: string }) => model.display_name || model.id,
      load: async () => {},
    })

    expect(html.includes('Create Custom Entry')).toBe(true)
    expect(html.includes('Edit Custom Entry')).toBe(true)
    expect(html.includes('Delete Custom Entry')).toBe(true)
    expect(html.match(/Delete Custom Entry/g)?.length).toBe(1)
    expect(html.includes('Find Entries')).toBe(true)
    expect(html.includes('Official only')).toBe(true)
    expect(html.includes('Custom only')).toBe(true)
    expect(html.includes('2/2')).toBe(true)
    expect(html.includes('remote catalog')).toBe(true)
    expect(html.includes('workspace override')).toBe(true)
  })
})
