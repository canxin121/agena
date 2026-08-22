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

test('deriveSendRunConfig emits no model when no explicit selection exists', () => {
  assert.deepEqual(
    deriveSendRunConfig({
      selectedThinkingMode: 'high',
      selectedSpeedMode: 'fast',
    }),
    {},
  )
})

test('deriveSendRunConfig emits only modes explicitly selected for the model', () => {
  assert.deepEqual(
    deriveSendRunConfig({
      selectedProviderId: 'anthropic',
      selectedAdapterId: 'messages',
      selectedModelId: 'claude-sonnet',
    }),
    { providerID: 'anthropic', adapterID: 'messages', modelID: 'claude-sonnet' },
  )
})

test('deriveSendRunConfig omits an incomplete model identity', () => {
  assert.deepEqual(deriveSendRunConfig({ selectedProviderId: 'openai' }), {})
})

test('deriveSendRunConfig never combines a partial identity into a model route', () => {
  assert.deepEqual(deriveSendRunConfig({ selectedProviderId: 'anthropic' }), {})
  assert.deepEqual(deriveSendRunConfig({ selectedModelId: 'claude-sonnet' }), {})
})
