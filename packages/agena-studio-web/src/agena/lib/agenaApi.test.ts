import { afterEach, describe, expect, test } from 'bun:test'

import { createPermissionRule, deleteModelCatalogEntry, listProviderModels } from './agenaApi'

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
      return new Response(JSON.stringify({ entries: [] }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      })
    }) as typeof fetch

    await deleteModelCatalogEntry('openai/google/gemini-2.5-pro')

    expect(capturedUrl).toContain('/api/v1/model-catalog/entries?')
    expect(capturedUrl).not.toContain('adapter_id=')
    expect(capturedUrl).toContain('model_id=openai%2Fgoogle%2Fgemini-2.5-pro')
    expect(capturedUrl).not.toContain('/providers/openai/models/')
    expect(capturedInit?.method).toBe('DELETE')
  })
})

describe('listProviderModels', () => {
  test('unwraps the provider models response envelope', async () => {
    let capturedUrl = ''

    globalThis.fetch = (async (input: string | URL | Request) => {
      capturedUrl = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url
      return new Response(
        JSON.stringify({
          provider_id: 'openai',
          models: [
            {
              provider_id: 'openai',
              id: 'gpt-5',
              display_name: 'GPT-5',
              metadata: {
                lifecycle: 'active',
                limits: {
                  context_window_tokens: 400000,
                  max_output_tokens: 16384,
                },
              },
              capabilities: {
                tool_calling: 'supported',
                streaming: 'supported',
              },
            },
          ],
        }),
        {
          status: 200,
          headers: { 'content-type': 'application/json' },
        },
      )
    }) as typeof fetch

    const models = await listProviderModels('openai')

    expect(capturedUrl).toContain('/api/v1/providers/openai/models')
    expect(models).toEqual([
      {
        provider_id: 'openai',
        id: 'gpt-5',
        display_name: 'GPT-5',
        metadata: {
          lifecycle: 'active',
          limits: {
            context_window_tokens: 400000,
            max_output_tokens: 16384,
          },
        },
        capabilities: {
          tool_calling: 'supported',
          streaming: 'supported',
        },
      },
    ])
  })
})

describe('createPermissionRule', () => {
  test('sends network access fields using the Agena permission wire shape', async () => {
    let capturedBody: Record<string, unknown> | null = null

    globalThis.fetch = (async (_input: string | URL | Request, init?: RequestInit) => {
      capturedBody = JSON.parse(String(init?.body || '{}')) as Record<string, unknown>
      return new Response(
        JSON.stringify({
          id: 1,
          action_key: 'network',
          subject_kind: 'network_access',
          network_target: 'api.example.com',
          network_host: 'api.example.com',
          network_port: 443,
          mode: 'deny',
          scope: 'global',
          source: 'api',
          created_at: '2026-05-10T00:00:00Z',
          updated_at: '2026-05-10T00:00:00Z',
        }),
        {
          status: 200,
          headers: { 'content-type': 'application/json' },
        },
      )
    }) as typeof fetch

    await createPermissionRule({
      subjectKind: 'network_access',
      networkTarget: 'api.example.com',
      networkPort: 443,
      scope: 'global',
      mode: 'deny',
    })

    expect(capturedBody).toEqual({
      subject_kind: 'network_access',
      network_target: 'api.example.com',
      network_port: 443,
      scope: 'global',
      mode: 'deny',
    })
  })
})
