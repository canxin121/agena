import assert from 'node:assert/strict'
import { test } from 'node:test'

import { buildRunRequestBody } from '../src/stores/chat/runOptions'

test('run options distinguish an omitted system override from explicit disable', () => {
  assert.deepEqual(buildRunRequestBody(), {})
  assert.deepEqual(buildRunRequestBody({ system: undefined }), {})
  assert.deepEqual(buildRunRequestBody({ system: '' }), { system: '' })
})

test('run options preserve exact system text while normalizing mode names', () => {
  const system = '  Keep this whitespace.\n\n'
  assert.deepEqual(buildRunRequestBody({ system, thinking_mode: ' high ', speed_mode: ' ', verbosity: ' low ' }), {
    system,
    thinking_mode: 'high',
    verbosity: 'low',
  })
})

test('system serialization keeps the existing model and numeric option rules', () => {
  assert.deepEqual(
    buildRunRequestBody({
      system: '',
      model: { provider_id: ' provider ', adapter_id: ' adapter ', model_id: ' model ' },
      temperature: 0,
      max_output_tokens: 200,
    }),
    {
      model: { provider_id: 'provider', adapter_id: 'adapter', model_id: 'model' },
      system: '',
      temperature: 0,
      max_output_tokens: 200,
    },
  )
  assert.deepEqual(buildRunRequestBody({ temperature: Number.NaN, max_output_tokens: 0 }), {})
})
