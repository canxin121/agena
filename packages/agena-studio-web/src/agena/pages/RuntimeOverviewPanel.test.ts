import { describe, expect, test } from 'bun:test'

import { renderVueSsr } from './test/renderVueSsr'

describe('RuntimeOverviewPanel', () => {
  test('renders catalog entry variant summaries and variant editor controls', async () => {
    const html = await renderVueSsr('/src/agena/pages/RuntimeOverviewPanel.vue', {
      catalogEntries: [
        {
          provider_id: 'anthropic',
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
          provider_id: 'openai',
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
          default_model: 'gpt-5',
          default_model_ref: 'openai/gpt-5',
          catalog_default_model: 'gpt-5',
        },
      ],
      providerModels: {
        openai: [
          {
            provider_id: 'openai',
            id: 'gpt-5',
            display_name: 'GPT-5',
          },
          {
            provider_id: 'openai',
            id: 'gpt-5-mini',
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
})
