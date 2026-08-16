import test from 'node:test'
import assert from 'node:assert/strict'

import { buildPlanToolInvocationRequest } from '../src/pages/chat/planViewerRequest'

test('plan viewer sends the backend a numeric session id', () => {
  const request = buildPlanToolInvocationRequest('42', 'get', { view: 'full' })

  assert.deepEqual(request, {
    plugin_id: 'agena.plan',
    tool: 'get',
    input: { view: 'full' },
    session_id: 42,
  })
  assert.equal(typeof request?.session_id, 'number')
})

test('plan viewer rejects missing and invalid session ids', () => {
  assert.equal(buildPlanToolInvocationRequest(null, 'get', { view: 'full' }), null)
  assert.equal(buildPlanToolInvocationRequest('not-a-session', 'phase', { autorun: true }), null)
  assert.equal(buildPlanToolInvocationRequest('0', 'get', { view: 'full' }), null)
})
