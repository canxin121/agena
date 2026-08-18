import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

import { normalizeProviderAdapterModels } from '../src/components/settings/providerStudioModelLists'

test('configured adapters from older servers normalize omitted model lists to empty arrays', () => {
  const adapters = normalizeProviderAdapterModels<{ id: string }>([
    { adapter_id: 'openai_responses', enabled: true },
    { adapter_id: 'anthropic', enabled: false, models: [{ id: 'claude-test' }] },
  ])

  assert.deepEqual(adapters, [
    { adapter_id: 'openai_responses', enabled: true, models: [] },
    { adapter_id: 'anthropic', enabled: false, models: [{ id: 'claude-test' }] },
  ])
  assert.deepEqual(
    adapters.flatMap((adapter) => adapter.models.map((model) => `${adapter.adapter_id}:${model.id}`)),
    ['anthropic:claude-test'],
  )
})

test('live provider model response envelopes use the same normalization path', () => {
  assert.deepEqual(
    normalizeProviderAdapterModels<{ id: string }>({
      provider_id: 'example',
      adapters: [{ adapter_id: 'openai_responses', enabled: true }],
    }),
    [{ adapter_id: 'openai_responses', enabled: true, models: [] }],
  )
})

test('Provider Studio normalizes both saved and live model-list responses', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/components/settings/ProviderStudioPanel.vue'), 'utf8')
  assert.match(source, /normalizeProviderAdapterModels<ProviderModel>\(configuredResponse\)/)
  assert.match(source, /normalizeProviderAdapterModels<ProviderModel>\(response\)/)
})
