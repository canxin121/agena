import test from 'node:test'
import assert from 'node:assert/strict'

import {
  encodeModelSelectionKey,
  parseModelSlug,
  resolveEffectiveDefaults,
} from '../src/pages/chat/modelSelectionDefaults'

test('model selection keys preserve provider, adapter, and model identity', () => {
  const key = encodeModelSelectionKey({
    provider: ' openai ',
    adapter: ' responses/v1 ',
    model: ' gpt-5/codex ',
  })

  assert.equal(key, 'openai/responses%2Fv1/gpt-5%2Fcodex')
  assert.deepEqual(parseModelSlug(key), {
    provider: 'openai',
    adapter: 'responses/v1',
    model: 'gpt-5/codex',
  })
})

test('parseModelSlug rejects removed two-part and malformed storage entries', () => {
  assert.deepEqual(parseModelSlug('anthropic/claude-sonnet'), { provider: '', adapter: '', model: '' })
  assert.deepEqual(parseModelSlug('invalid'), { provider: '', adapter: '', model: '' })
})

test('resolveEffectiveDefaults prefers the runtime-wide default and keeps its modes', () => {
  assert.deepEqual(
    resolveEffectiveDefaults({
      runtime: {
        provider: 'openai',
        adapter: 'responses',
        model: 'gpt-5',
        thinkingMode: 'high',
        speedMode: 'fast',
        verbosity: 'compact',
        parallelToolCalls: true,
      },
      fallback: { provider: 'fallback', model: 'fallback-model' },
    }),
    {
      provider: 'openai',
      adapter: 'responses',
      model: 'gpt-5',
      thinkingMode: 'high',
      speedMode: 'fast',
      verbosity: 'compact',
      parallelToolCalls: true,
    },
  )
})

test('resolveEffectiveDefaults does not invent a provider or model', () => {
  assert.deepEqual(
    resolveEffectiveDefaults({ runtime: null, fallback: null }),
    {
      provider: '',
      adapter: '',
      model: '',
      thinkingMode: '',
      speedMode: '',
      verbosity: '',
    },
  )
})
