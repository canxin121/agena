import test from 'node:test'
import assert from 'node:assert/strict'

import {
  defaultModeValue,
  modelIdsFromProviderModels,
  speedModeOptionsForModel,
  thinkingModeOptionsForModel,
} from '../src/pages/chat/modelSelectionCatalog'

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
    ['off', 'high', 'deep'],
  )
  assert.equal(defaultModeValue(thinking), 'high')
  assert.deepEqual(
    speed.map((option) => option.value),
    ['normal', 'fast'],
  )
  assert.equal(defaultModeValue(speed), 'fast')
})
