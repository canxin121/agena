import assert from 'node:assert/strict'
import test from 'node:test'

import {
  supportsParallelToolCallsForModel,
  verbosityOptionsForModel,
  type ProviderModel,
} from '../src/pages/chat/modelSelectionCatalog'

function model(overrides: Partial<ProviderModel>): ProviderModel {
  return { provider_id: 'test', id: 'model', ...overrides }
}

test('verbosity options follow Agena model metadata and preserve custom defaults', () => {
  assert.deepEqual(
    verbosityOptionsForModel(model({ supports_verbosity: true } as Partial<ProviderModel>)).map((item) => item.value),
    ['low', 'medium', 'high'],
  )
  assert.deepEqual(
    verbosityOptionsForModel(model({ metadata: { supports_verbosity: true, default_verbosity: 'verbose' } })).map(
      (item) => item.value,
    ),
    ['low', 'medium', 'high', 'verbose'],
  )
})

test('gpt-5 chat models expose the runtime-supported medium verbosity only', () => {
  assert.deepEqual(
    verbosityOptionsForModel(
      model({ id: 'gpt-5-chat-latest', supports_verbosity: true } as Partial<ProviderModel>),
    ).map((item) => item.value),
    ['medium'],
  )
})

test('parallel tool-call support is read from the model or metadata projection', () => {
  assert.equal(
    supportsParallelToolCallsForModel(model({ supports_parallel_tool_calls: true } as Partial<ProviderModel>)),
    true,
  )
  assert.equal(supportsParallelToolCallsForModel(model({ metadata: { supports_parallel_tool_calls: true } })), true)
})
