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

test('verbosity options use explicit levels and preserve user defaults', () => {
  assert.deepEqual(
    verbosityOptionsForModel(
      model({ supports_verbosity: true, metadata: { verbosity_levels: ['low', 'high'] } } as Partial<ProviderModel>),
    ).map((item) => item.value),
    ['low', 'high'],
  )
  assert.deepEqual(
    verbosityOptionsForModel(
      model({
        metadata: {
          supports_verbosity: true,
          verbosity_levels: ['low', 'medium', 'high'],
          default_verbosity: 'verbose',
        },
      }),
    ).map((item) => item.value),
    ['low', 'medium', 'high', 'verbose'],
  )
})

test('model names do not imply verbosity support or allowed values', () => {
  assert.deepEqual(
    verbosityOptionsForModel(model({ id: 'gpt-99-chat-latest' })).map((item) => item.value),
    [],
  )
})

test('an explicit unsupported profile suppresses configured verbosity values', () => {
  assert.deepEqual(
    verbosityOptionsForModel(
      model({
        metadata: {
          supports_verbosity: false,
          verbosity_levels: ['low', 'high'],
          default_verbosity: 'medium',
        },
      }),
    ),
    [],
  )
})

test('parallel tool-call support is read from the model or metadata projection', () => {
  assert.equal(
    supportsParallelToolCallsForModel(model({ supports_parallel_tool_calls: true } as Partial<ProviderModel>)),
    true,
  )
  assert.equal(supportsParallelToolCallsForModel(model({ metadata: { supports_parallel_tool_calls: true } })), true)
})
