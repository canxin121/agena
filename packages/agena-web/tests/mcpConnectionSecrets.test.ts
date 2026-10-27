import { describe, expect, test } from 'bun:test'
import { MCP_REDACTED, changeMcpValue, mcpConfigProblems, mcpValueAt, preservedMcpSecrets, type McpDraft } from '../src/lib/mcpConnections'

function hiddenEndpoint(): McpDraft {
  return {
    config: {
      transport: 'http', enabled: true,
      endpoint: { url: MCP_REDACTED, headers: { Authorization: MCP_REDACTED } },
      auth: { kind: 'o_auth', scopes: ['read'], client_id: 'registered-client', client_secret: MCP_REDACTED },
    },
    preserveSecrets: ['/endpoint/url', '/endpoint/headers/Authorization', '/auth/client_secret'],
  }
}

describe('MCP hidden endpoint and OAuth registration edits', () => {
  test('ordinary edits preserve a hidden endpoint without requiring its secret URL', () => {
    const draft = changeMcpValue(hiddenEndpoint(), '/timeouts/request_ms', 45000)
    expect(mcpConfigProblems(draft)).toEqual([])
    expect(preservedMcpSecrets(draft)).toEqual(['/endpoint/url', '/endpoint/headers/Authorization', '/auth/client_secret'])
  })

  test('a hidden endpoint can only be preserved on an existing connection', () => {
    expect(mcpConfigProblems(hiddenEndpoint(), true)).toContain('Enter an HTTP or HTTPS MCP endpoint.')
    const untrusted = { ...hiddenEndpoint(), preserveSecrets: ['/auth/client_secret', '/endpoint/headers/Authorization'] }
    expect(mcpConfigProblems(untrusted)).toContain('Replace or remove redacted values that cannot be preserved.')
  })

  test('replacing the URL retires its pointer and validates the replacement', () => {
    const empty = changeMcpValue(hiddenEndpoint(), '/endpoint/url', '')
    expect(mcpConfigProblems(empty)).toContain('Enter an HTTP or HTTPS MCP endpoint.')
    const replacement = changeMcpValue(empty, '/endpoint/url', 'https://example.test/mcp?key=new-value')
    expect(mcpConfigProblems(replacement)).toEqual([])
    expect(preservedMcpSecrets(replacement)).toEqual(['/endpoint/headers/Authorization', '/auth/client_secret'])
  })

  test('OAuth client secrets can be replaced, removed or retired by an auth change', () => {
    const replacement = changeMcpValue(hiddenEndpoint(), '/auth/client_secret', 'new-client-secret')
    expect(mcpValueAt(replacement.config, '/auth/client_secret')).toBe('new-client-secret')
    expect(preservedMcpSecrets(replacement)).toEqual(['/endpoint/url', '/endpoint/headers/Authorization'])
    const removed = changeMcpValue(hiddenEndpoint(), '/auth/client_secret', undefined, true)
    expect(mcpValueAt(removed.config, '/auth/client_secret')).toBeUndefined()
    expect(preservedMcpSecrets(removed)).toEqual(['/endpoint/url', '/endpoint/headers/Authorization'])
    const switched = changeMcpValue(hiddenEndpoint(), '/auth', { kind: 'bearer_from_env', env: 'MCP_TOKEN' })
    expect(mcpConfigProblems(switched)).toEqual([])
    expect(preservedMcpSecrets(switched)).toEqual(['/endpoint/url', '/endpoint/headers/Authorization'])
  })
})
