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
})
