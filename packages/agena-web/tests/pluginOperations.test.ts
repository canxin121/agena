import assert from 'node:assert/strict'
import test from 'node:test'

import { pluginOperationInvocationBody } from '../src/lib/pluginOperations'

test('slash shorthand is preserved for the server-owned SettingsContract parser', () => {
  assert.deepEqual(
    pluginOperationInvocationBody({
      operation: { slash: 'memory-search' },
      sessionId: 12,
      rawArgs: '  query=release limit=5  ',
    }),
    {
      input: {},
      session_id: 12,
      slash: 'memory-search',
      raw: 'query=release limit=5',
    },
  )
})

test('sessionless navigation operations retain the same request shape', () => {
  assert.deepEqual(
    pluginOperationInvocationBody({ operation: { slash: 'memory' }, sessionId: null, rawArgs: '' }),
    { input: {}, session_id: null, slash: 'memory', raw: '' },
  )
})
