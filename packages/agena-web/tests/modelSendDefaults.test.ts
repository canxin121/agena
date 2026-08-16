import test from 'node:test'
import assert from 'node:assert/strict'

import { deriveSendRunConfig } from '../src/pages/chat/modelSendDefaults'

test('deriveSendRunConfig emits provider, adapter, model, thinking, and speed fields', () => {
  assert.deepEqual(
    deriveSendRunConfig({
      selectedProviderId: 'anthropic',
      selectedAdapterId: 'messages',
      selectedModelId: 'claude-sonnet',
      selectedThinkingMode: 'high',
      selectedSpeedMode: 'fast',
      effectiveDefaults: { provider: 'openai', adapter: 'responses', model: 'gpt-5' },
    }),
    {
      providerID: 'anthropic',
      adapterID: 'messages',
      modelID: 'claude-sonnet',
      thinkingMode: 'high',
      speedMode: 'fast',
    },
  )
})

test('deriveSendRunConfig falls back to the complete runtime default', () => {
  assert.deepEqual(
    deriveSendRunConfig({
      effectiveDefaults: {
        provider: 'openai',
        adapter: 'responses',
        model: 'gpt-5',
        thinkingMode: 'high',
        speedMode: 'fast',
        verbosity: 'compact',
        parallelToolCalls: false,
      },
    }),
    {
      providerID: 'openai',
      adapterID: 'responses',
      modelID: 'gpt-5',
      thinkingMode: 'high',
      speedMode: 'fast',
      verbosity: 'compact',
      parallelToolCalls: false,
    },
  )
})

test('deriveSendRunConfig does not leak default modes into a different model', () => {
  assert.deepEqual(
    deriveSendRunConfig({
      selectedProviderId: 'anthropic',
      selectedAdapterId: 'messages',
      selectedModelId: 'claude-sonnet',
      effectiveDefaults: {
        provider: 'openai',
        adapter: 'responses',
        model: 'gpt-5',
        thinkingMode: 'high',
        speedMode: 'fast',
        verbosity: 'compact',
        parallelToolCalls: true,
      },
    }),
    { providerID: 'anthropic', adapterID: 'messages', modelID: 'claude-sonnet' },
  )
})

test('deriveSendRunConfig omits an incomplete model identity', () => {
  assert.deepEqual(deriveSendRunConfig({ selectedProviderId: 'openai' }), {})
})

test('deriveSendRunConfig never combines a partial selection with runtime defaults', () => {
  const effectiveDefaults = {
    provider: 'openai',
    adapter: 'responses',
    model: 'gpt-5',
    thinkingMode: 'high',
  }

  assert.deepEqual(
    deriveSendRunConfig({
      selectedProviderId: 'anthropic',
      effectiveDefaults,
    }),
    {
      providerID: 'openai',
      adapterID: 'responses',
      modelID: 'gpt-5',
      thinkingMode: 'high',
    },
  )

  assert.deepEqual(
    deriveSendRunConfig({
      selectedModelId: 'claude-sonnet',
      effectiveDefaults,
    }),
    {
      providerID: 'openai',
      adapterID: 'responses',
      modelID: 'gpt-5',
      thinkingMode: 'high',
    },
  )
})
