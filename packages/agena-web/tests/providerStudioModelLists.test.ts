import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

import { normalizeProviderAdapterModels } from '../src/components/settings/providerStudioModelLists'

test('configured adapters require the current models array', () => {
  assert.throws(
    () => normalizeProviderAdapterModels<{ id: string }>([{ adapter_id: 'openai_responses', enabled: true }]),
    /missing its models array/,
  )

  assert.deepEqual(
    normalizeProviderAdapterModels<{ id: string }>([
      { adapter_id: 'anthropic', enabled: false, models: [{ id: 'claude-test' }] },
    ]),
    [{ adapter_id: 'anthropic', enabled: false, models: [{ id: 'claude-test' }] }],
  )
})

test('live provider model response envelopes require the same current adapter shape', () => {
  assert.throws(
    () =>
      normalizeProviderAdapterModels<{ id: string }>({
        provider_id: 'example',
        adapters: [{ adapter_id: 'openai_responses', enabled: true }],
      }),
    /missing its models array/,
  )
})

test('Provider Studio normalizes both saved and live model-list responses', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/components/settings/ProviderStudioPanel.vue'), 'utf8')
  assert.match(source, /normalizeProviderAdapterModels<ProviderModel>\(configuredResponse\)/)
  assert.match(source, /normalizeProviderAdapterModels<ProviderModel>\(response\)/)
})
