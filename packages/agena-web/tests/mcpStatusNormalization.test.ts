import assert from 'node:assert/strict'
import test from 'node:test'

import { normalizeMcpStatus } from '../src/lib/mcpStatus'

test('normalizeMcpStatus projects the canonical runtime operator MCP shape', () => {
  const items = normalizeMcpStatus({
    operator: {
      mcp: {
        servers: [
          { name: 'github', tool_count: 7 },
          { name: 'local', tool_count: 2 },
        ],
      },
    },
  })

  assert.deepEqual(items, [
    { name: 'github', status: 'running', toolCount: 7 },
    { name: 'local', status: 'running', toolCount: 2 },
  ])
})

test('normalizeMcpStatus rejects alternate list and keyed-record payloads', () => {
  assert.deepEqual(normalizeMcpStatus([{ name: 'github', status: 'connected' }] as never), [])
  assert.deepEqual(normalizeMcpStatus({ github: { status: 'connected' } } as never), [])
})
