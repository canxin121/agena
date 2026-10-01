import assert from 'node:assert/strict'
import test from 'node:test'

import { pluginCommandInvocationBody } from '../src/lib/pluginOperations'

test('slash shorthand is preserved for the server-owned SettingsContract parser', () => {
  assert.deepEqual(
    pluginCommandInvocationBody({
      command: { slash: 'memory-search' },
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

test('sessionless navigation commands retain the same request shape', () => {
  assert.deepEqual(pluginCommandInvocationBody({ command: { slash: 'memory' }, sessionId: null, rawArgs: '' }), {
    input: {},
    session_id: null,
    slash: 'memory',
    raw: '',
  })
})
