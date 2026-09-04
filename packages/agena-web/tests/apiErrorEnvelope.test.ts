import assert from 'node:assert/strict'
import test from 'node:test'

import { ApiError, apiJson } from '../src/lib/api'

test('apiJson exposes Agena problem fallback and code', async () => {
  const originalFetch = globalThis.fetch
  globalThis.fetch = (async () =>
    new Response(
      JSON.stringify({
        problem: {
          code: 'provider.model_unavailable',
          category: 'configuration',
          user: { fallback: 'The selected model is unavailable.' },
        },
      }),
      { status: 422, headers: { 'content-type': 'application/json' } },
    )) as typeof fetch

  try {
    await assert.rejects(apiJson('/api/v1/example'), (error: unknown) => {
      assert.ok(error instanceof ApiError)
      assert.equal(error.status, 422)
      assert.equal(error.code, 'provider.model_unavailable')
      assert.equal(error.message, 'The selected model is unavailable.')
      return true
    })
  } finally {
    globalThis.fetch = originalFetch
  }
})

test('apiJson preserves current Workbench error and hint envelopes', async () => {
  const originalFetch = globalThis.fetch
  globalThis.fetch = (async () =>
    new Response(JSON.stringify({ error: 'File write failed', code: 'fs_write', hint: 'Check permissions.' }), {
      status: 500,
      headers: { 'content-type': 'application/json' },
    })) as typeof fetch

  try {
    await assert.rejects(apiJson('/api/v1/workbench/fs/write'), (error: unknown) => {
      assert.ok(error instanceof ApiError)
      assert.equal(error.code, 'fs_write')
      assert.equal(error.hint, 'Check permissions.')
      assert.equal(error.message, 'File write failed\nCheck permissions.')
      return true
    })
  } finally {
    globalThis.fetch = originalFetch
  }
})
