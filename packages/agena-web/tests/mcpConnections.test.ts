import { describe, expect, test } from 'bun:test'
import {
  MCP_REDACTED,
  changeMcpValue,
  filterMcpConnections,
  mcpCallbackMatches,
  mcpConfigProblems,
  mcpDraftFromDetail,
  mcpEndpoint,
  mcpPointerKey,
  mcpValueAt,
  newMcpConfig,
  parseMcpImport,
  preservedMcpSecrets,
  readMcpPendingAuth,
  type McpDraft,
} from '../src/lib/mcpConnections'
import { normalizeMcpStatus } from '../src/lib/mcpStatus'
import type { McpConnection, McpConnectionDetail } from '../src/lib/mcpConnectionsApi'

function connection(overrides: Partial<McpConnection> = {}): McpConnection {
  return { id: 'opaque-global', name: 'evidence', scope: 'global', transport: 'http', enabled: true, effective: true, state: 'connected', tool_count: 3, revision: 'r1', error: null, ...overrides }
}
function detail(): McpConnectionDetail {
  return {
    ...connection(),
    config: { transport: 'http', endpoint: { url: 'https://example.test/mcp', headers: { Authorization: MCP_REDACTED } }, auth: { kind: 'bearer', token: MCP_REDACTED }, enabled: true },
    secret_paths: ['/endpoint/headers/Authorization', '/auth/token'],
    protocol_version: '2025-11-25', server_info: null, capabilities: null, oauth_health: null,
  }
}

describe('MCP configuration editing', () => {
  test('ordinary edits preserve explicit secrets without returning or inventing values', () => {
    const source = detail()
    const draft = changeMcpValue(mcpDraftFromDetail(source), '/endpoint/url', 'https://example.test/new-mcp')
    expect(preservedMcpSecrets(draft)).toEqual(['/endpoint/headers/Authorization', '/auth/token'])
    expect(mcpValueAt(draft.config, '/auth/token')).toBe(MCP_REDACTED)
    expect(mcpValueAt(source.config, '/endpoint/url')).toBe('https://example.test/mcp')
    expect(mcpConfigProblems(draft)).toEqual([])
  })

  test('replacing or deleting a secret retires only its preservation pointer', () => {
    const changed = changeMcpValue(mcpDraftFromDetail(detail()), '/auth/token', 'new-token')
    expect(preservedMcpSecrets(changed)).toEqual(['/endpoint/headers/Authorization'])
    const cleared = changeMcpValue(changed, '/endpoint/headers/Authorization', undefined, true)
    expect(preservedMcpSecrets(cleared)).toEqual([])
    expect(mcpValueAt(cleared.config, '/endpoint/headers/Authorization')).toBeUndefined()
    expect(mcpValueAt(cleared.config, '/auth/token')).toBe('new-token')
  })

  test('replacing authentication removes its child pointers and keeps unrelated headers', () => {
    const draft = changeMcpValue(mcpDraftFromDetail(detail()), '/auth', { kind: 'o_auth', scopes: ['read'] })
    expect(preservedMcpSecrets(draft)).toEqual(['/endpoint/headers/Authorization'])
    expect(mcpValueAt(draft.config, '/auth/token')).toBeUndefined()
  })

  test('JSON pointers preserve literal slash and tilde names without prototype mutation', () => {
    const pointer = '/process/env/' + mcpPointerKey('A/B~C')
    const draft = changeMcpValue({ config: newMcpConfig(), preserveSecrets: [] }, pointer, 'value')
    expect(mcpValueAt(draft.config, pointer)).toBe('value')
    const safe = changeMcpValue(draft, '/process/env/__proto__', 'ordinary-string')
    expect(mcpValueAt(safe.config, '/process/env/__proto__')).toBe('ordinary-string')
    expect(Object.getPrototypeOf(mcpValueAt(safe.config, '/process/env'))).toBe(Object.prototype)
    expect(mcpValueAt(safe.config, '/process/env/constructor')).toBeUndefined()
  })

  test('redacted placeholders cannot silently create new credentials', () => {
    const draft = mcpDraftFromDetail(detail())
    expect(mcpConfigProblems(draft, true)).toContain('Replace or remove redacted values that cannot be preserved.')
    const missing: McpDraft = { config: draft.config, preserveSecrets: [] }
    expect(mcpConfigProblems(missing)).toContain('Replace or remove redacted values that cannot be preserved.')
  })

  test('stdio arguments preserve spaces and empty strings and timeouts use milliseconds', () => {
    let draft: McpDraft = { config: newMcpConfig(), preserveSecrets: [] }
    draft = changeMcpValue(draft, '/process/command', '/path with spaces/mcp')
    draft = changeMcpValue(draft, '/process/args', ['--name', 'A B', ''])
    draft = changeMcpValue(draft, '/timeouts/connect_ms', 300000)
    draft = changeMcpValue(draft, '/timeouts/request_ms', 3600000)
    expect(mcpConfigProblems(draft)).toEqual([])
    expect(mcpValueAt(draft.config, '/process/args')).toEqual(['--name', 'A B', ''])
    expect(mcpConfigProblems(changeMcpValue(draft, '/timeouts/connect_ms', 0))).toContain('Connect timeout must be between 1 and 300000 milliseconds.')
  })

  test('HTTP display omits URL credentials and query tokens', () => {
    expect(mcpEndpoint({ config: { transport: 'http', endpoint: { url: 'https://user:secret@example.test/mcp?token=private#frag' } } })).toBe('https://example.test/mcp')
  })

  test('import parsing preserves common configuration and rejects non-objects', () => {
    const document = { mcpServers: { evidence: { command: 'service', args: ['--read'], env: { API_KEY: 'provided-value' } } } }
    expect(parseMcpImport(JSON.stringify(document))).toEqual(document)
    for (const input of ['[]', 'null', 'true', '"text"', '{']) expect(() => parseMcpImport(input)).toThrow()
  })
})

describe('MCP runtime presentation', () => {
  test('same-name copies remain distinct and disconnected is never presented as running', () => {
    const entries = normalizeMcpStatus({ connections: [connection({ state: 'disconnected', tool_count: 0 }), connection({ id: 'opaque-workspace', scope: 'workspace', state: 'disabled', enabled: false }), null, { name: 'missing-id' }] })
    expect(entries.map((entry) => entry.id)).toEqual(['opaque-global', 'opaque-workspace'])
    expect(entries.map((entry) => entry.status)).toEqual(['disconnected', 'disabled'])
    expect(entries[0]?.toolCount).toBe(0)
  })

  test('attention filtering separates inactive copies from failing connections', () => {
    const entries = [connection(), connection({ id: 'failed', state: 'authentication_required' }), connection({ id: 'inactive', state: 'shadowed' }), connection({ id: 'off', state: 'disabled' })]
    expect(filterMcpConnections(entries, '', 'all', 'attention').map((entry) => entry.id)).toEqual(['failed'])
    expect(filterMcpConnections(entries, 'EVIDENCE', 'global', 'connected').map((entry) => entry.id)).toEqual(['opaque-global'])
  })
})

describe('MCP OAuth continuation', () => {
  const flow = { flow_id: 'flow-1', connection_id: 'opaque-id', state: 'expected+/state', expires_at: '2099-01-01T00:00:00Z' }
  test('only nonsecret continuation metadata is restored from storage', () => {
    expect(readMcpPendingAuth(JSON.stringify({ ...flow, access_token: 'must-not-return', verifier: 'must-not-return' }))).toEqual(flow)
    expect(readMcpPendingAuth('{')).toBeNull()
    expect(readMcpPendingAuth(JSON.stringify({ ...flow, state: '' }))).toBeNull()
  })
  test('callback acceptance requires matching state, code and an HTTP URL', () => {
    const valid = 'https://agena.example/settings/mcp-connections?state=' + encodeURIComponent(flow.state) + '&code=code'
    expect(mcpCallbackMatches(flow, valid)).toBe(true)
    expect(mcpCallbackMatches(flow, valid + '&error=denied')).toBe(false)
    expect(mcpCallbackMatches(flow, 'https://agena.example/?state=wrong&code=code')).toBe(false)
    expect(mcpCallbackMatches(flow, 'javascript:alert(1)?state=' + encodeURIComponent(flow.state) + '&code=code')).toBe(false)
  })
})
