import test from 'node:test'
import assert from 'node:assert/strict'

import { encodeModelSelectionKey, parseModelSlug } from '../src/pages/chat/modelSelectionDefaults'

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
