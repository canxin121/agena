import { afterEach, describe, expect, test } from 'bun:test'

import { deleteModelCatalogEntry, listProviderModels } from './agenaApi'

const originalFetch = globalThis.fetch

afterEach(() => {
  globalThis.fetch = originalFetch
})

describe('deleteModelCatalogEntry', () => {
  test('sends slashful model ids in the query string instead of the URL path', async () => {
    let capturedUrl = ''
    let capturedInit: RequestInit | undefined

    globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit) => {
      capturedUrl = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url
      capturedInit = init
      return new Response(JSON.stringify({ remote_url: '', fallback_url: '', entries: [] }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      })
    }) as typeof fetch

    await deleteModelCatalogEntry('openai', 'openai/google/gemini-2.5-pro')

    expect(capturedUrl).toContain('/api/v1/model-catalog/entries?')
    expect(capturedUrl).toContain('provider_id=openai')
    expect(capturedUrl).toContain('model_id=openai%2Fgoogle%2Fgemini-2.5-pro')
    expect(capturedUrl).not.toContain('/providers/openai/models/')
    expect(capturedInit?.method).toBe('DELETE')
  })
})

describe('listProviderModels', () => {
  test('reads the backend provider models response envelope instead of assuming a raw array', async () => {
    let capturedUrl = ''

    globalThis.fetch = (async (input: string | URL | Request) => {
      capturedUrl = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url
      return new Response(
        JSON.stringify({
          provider_id: 'gateway',
          models: [
            {
              provider_id: 'gateway',
              id: 'openai/gpt-5',
              display_name: 'GPT-5',
            },
          ],
        }),
        {
          status: 200,
          headers: { 'content-type': 'application/json' },
        },
      )
    }) as typeof fetch

    const models = await listProviderModels('gateway')

    expect(capturedUrl).toContain('/api/v1/providers/gateway/models')
    expect(Array.isArray(models)).toBe(true)
    expect(models).toEqual([
      {
        provider_id: 'gateway',
        id: 'openai/gpt-5',
        display_name: 'GPT-5',
      },
    ])
  })
})
