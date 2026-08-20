import assert from 'node:assert/strict'
import test from 'node:test'

import {
  isAssistantMessageStreaming,
  isKnownRunState,
  isRunFailureState,
  isRunInFlight,
  isRunTerminal,
} from '../src/lib/chatRunState'

test('assistant streaming requires an explicit backend in-flight state', () => {
  assert.equal(isAssistantMessageStreaming({ role: 'assistant', runState: 'in_progress' }), true)
  assert.equal(isAssistantMessageStreaming({ role: 'assistant', runState: 'running' }), true)
  assert.equal(isAssistantMessageStreaming({ role: 'assistant' }), false)
  assert.equal(isAssistantMessageStreaming({ role: 'assistant', finish: '' }), false)
  assert.equal(isAssistantMessageStreaming({ role: 'assistant', runState: 'completed' }), false)
  assert.equal(isAssistantMessageStreaming({ role: 'assistant', runState: 'in_progress', error: {} }), false)
})

test('run-state helpers classify only explicit backend wire values', () => {
  assert.equal(isKnownRunState('future_state'), false)
  assert.equal(isRunInFlight('future_state'), false)
  assert.equal(isRunTerminal('future_state'), false)
  assert.equal(isRunFailureState('future_state'), false)
  assert.equal(isRunFailureState('failed'), true)
  assert.equal(isRunTerminal('cancelled'), true)
})
