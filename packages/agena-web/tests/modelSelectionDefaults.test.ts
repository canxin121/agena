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

test('parseModelSlug accepts legacy provider/model storage entries', () => {
  assert.deepEqual(parseModelSlug('anthropic/claude-sonnet'), {
    provider: 'anthropic',
    adapter: '',
    model: 'claude-sonnet',
  })
  assert.deepEqual(parseModelSlug('invalid'), { provider: '', adapter: '', model: '' })
})

test('resolveEffectiveDefaults uses the complete Agena runtime selection', () => {
  assert.deepEqual(
    resolveEffectiveDefaults({
      runtime: {
        provider: 'openai',
        adapter: 'responses',
        model: 'gpt-5',
        thinkingMode: 'high',
        speedMode: 'fast',
        verbosity: 'compact',
        parallelToolCalls: false,
      },
      fallback: { provider: 'anthropic', adapter: 'messages', model: 'claude-sonnet' },
    }),
    {
      provider: 'openai',
      adapter: 'responses',
      model: 'gpt-5',
      thinkingMode: 'high',
      speedMode: 'fast',
      verbosity: 'compact',
      parallelToolCalls: false,
    },
  )
})

test('resolveEffectiveDefaults falls back only when runtime identity is incomplete', () => {
  assert.deepEqual(
    resolveEffectiveDefaults({
      runtime: { provider: 'openai', thinkingMode: 'high' },
      fallback: { provider: 'anthropic', adapter: 'messages', model: 'claude-sonnet' },
    }),
    {
      provider: 'anthropic',
      adapter: 'messages',
      model: 'claude-sonnet',
      thinkingMode: 'high',
      speedMode: '',
      verbosity: '',
    },
  )
})
