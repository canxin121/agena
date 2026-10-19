import test from 'node:test'
import assert from 'node:assert/strict'

import {
  defaultModeValue,
  modeOptionDisplayLabel,
  modelIdsFromProviderModels,
  modelSelectionIsConfigured,
  speedModeOptionsForModel,
  thinkingModeOptionsForModel,
} from '../src/pages/chat/modelSelectionCatalog'

test('modelSelectionIsConfigured matches the provider, adapter, and model route', () => {
  const providers = [
    {
      id: 'cpa',
      name: 'CPA',
      models: [
        { provider_id: 'cpa', adapter_id: 'openai_responses', id: 'deepseek-v4.1-flash' },
        { provider_id: 'cpa', adapter_id: 'openai_responses', id: 'gpt-5.5' },
      ],
    },
  ]

  assert.equal(
    modelSelectionIsConfigured(providers, {
      provider: 'cpa',
      adapter: 'openai_responses',
      model: 'deepseek-v4.1-flash',
    }),
    true,
  )
  // A model cached through another (now disabled) adapter is not offered.
  assert.equal(
    modelSelectionIsConfigured(providers, { provider: 'cpa', adapter: 'anthropic', model: 'deepseek-v4.1-flash' }),
    false,
  )
  assert.equal(
    modelSelectionIsConfigured(providers, { provider: 'cpa', adapter: 'anthropic', model: 'glm-5.3-flash' }),
    false,
  )
  assert.equal(modelSelectionIsConfigured(providers, { provider: 'cpa', model: 'gpt-5.5' }), true)
  assert.equal(modelSelectionIsConfigured(providers, { provider: 'other', model: 'gpt-5.5' }), false)
  assert.equal(modelSelectionIsConfigured(providers, { provider: 'cpa', model: '' }), false)
  assert.equal(modelSelectionIsConfigured([], { provider: 'cpa', model: 'gpt-5.5' }), false)
})

test('modelIdsFromProviderModels reads the configured-model array contract', () => {
  assert.deepEqual(
    modelIdsFromProviderModels([
      { provider_id: 'openai', id: 'gpt-5' },
      { provider_id: 'anthropic', id: 'claude-sonnet' },
      { provider_id: 'invalid', id: '   ' },
    ]),
    ['gpt-5', 'claude-sonnet'],
  )
  assert.deepEqual(modelIdsFromProviderModels(null), [])
})

test('thinking and speed modes follow Agena model resource selectors', () => {
  const model = {
    provider_id: 'openai',
    adapter_id: 'responses',
    id: 'gpt-5',
    thinking_modes: [
      { display_name: 'Off', thinking: { type: 'disabled' } },
      { default: true, display_name: 'High', thinking: { type: 'effort', effort: 'high' } },
      { display_name: 'Custom preset', preset: 'deep' },
    ],
    speed_modes: {
      normal: { display_name: 'Normal' },
      fast: { default: true, display_name: 'Fast' },
    },
  }

  const thinking = thinkingModeOptionsForModel(model)
  const speed = speedModeOptionsForModel(model)
  assert.deepEqual(
    thinking.map((option) => option.value),
    ['off', 'deep', 'high'],
  )
  assert.equal(defaultModeValue(thinking), 'high')
  assert.deepEqual(
    speed.map((option) => option.value),
    ['normal', 'fast'],
  )
  assert.equal(defaultModeValue(speed), 'fast')
  assert.equal(modeOptionDisplayLabel(speed, 'fast'), 'Fast')
  assert.equal(modeOptionDisplayLabel(speed, ''), '')
})

test('speed modes without an explicit model default preserve native model behavior', () => {
  const options = speedModeOptionsForModel({
    provider_id: 'openai',
    id: 'gpt-5.6-luna',
    speed_modes: {
      fast: { display_name: 'Fast' },
      pro: { display_name: 'Pro' },
    },
  })

  assert.equal(defaultModeValue(options), '')
})
