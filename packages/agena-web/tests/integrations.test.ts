import { describe, expect, test } from 'bun:test'
import { integrationTemplate, parseIntegration, parseMcpAllowlist } from '../src/features/integrations/model'
import type { IntegrationProvider } from '../src/generated/agenaState'

const source: IntegrationProvider = {
  plugin_id: 'example.github_issues',
  fingerprint: 'fixture',
  adapter: {
    kind: 'event_source',
    title: 'Issues',
    description: 'External source',
    minimum_poll_seconds: 120,
    idempotent_send: false,
    config_schema: { type: 'object', properties: { repository: { type: 'string' } } },
  },
}

describe('plugin integration authorization proposals', () => {
  test('a source proposal defaults to notification and requires explicit session execution consent', () => {
    const draft = integrationTemplate(source, 1000)
    expect(draft.mode).toBe('notify')
    expect(draft.max_execution_seconds).toBe(300)
    expect('max_task_seconds' in draft).toBe(false)
    expect(draft.use_source_permissions).toBe(false)
    expect(draft.poll_seconds).toBe(120)
    expect(draft.expires_at_ms).toBe(1000 + 7 * 86400000)
    expect(draft.config).toEqual({ repository: 'owner/repository' })
    expect(draft.resource).toBe('owner/repository')
    expect(draft.event_types).toEqual(['issue.opened'])
  })
  test('a channel proposal requires replacing the exact operator placeholder and explicit permission consent', () => {
    const provider: IntegrationProvider = {
      ...source,
      plugin_id: 'custom.channel',
      adapter: {
        ...source.adapter,
        kind: 'channel',
        minimum_poll_seconds: 1,
        config_schema: { properties: { conversation: { type: 'string' } } },
      },
    }
    const draft = integrationTemplate(provider, 1000)
    expect(draft.mode).toBe('channel')
    expect(draft.actors).toEqual([])
    expect(draft.channel_policy?.members).toEqual([{ actor: 'owner-user-id', role: 'owner', expires_at_ms: null }])
    expect(draft.channel_policy?.session_mode).toBe('per_user')
    expect(draft.channel_policy?.allow_unlisted_chat).toBe(false)
    expect(draft.channel_policy?.grants).toEqual({})
    expect(draft.use_source_permissions).toBe(false)
    expect(draft.event_types).toEqual(['message'])
    expect(draft.config).toEqual({ conversation: 'account:conversation' })
  })
  test('unknown providers get a neutral config rather than GitHub-specific behavior', () => {
    const provider: IntegrationProvider = {
      ...source,
      adapter: { ...source.adapter, config_schema: { properties: { feed: { type: 'string' } } } },
    }
    expect(integrationTemplate(provider, 1).config).toEqual({})
    expect(integrationTemplate(provider, 1).resource).toBe('exact-resource')
  })
  test('JSON editor rejects scalar roots and leaves final schema validation to the actual provider contract', () => {
    for (const text of ['null', '[]', '"value"', '42', '{bad']) expect(() => parseIntegration(text)).toThrow()
    const draft = integrationTemplate(source, 1000)
    expect(parseIntegration(JSON.stringify(draft))).toEqual(draft)
  })
})

describe('channel MCP server/tool authorization', () => {
  test('accepts exact server/tool pairs and preserves the requested capability set', () => {
    expect(parseMcpAllowlist('{"docs":["search","read"]}')).toEqual({ docs: ['search', 'read'] })
    expect(parseMcpAllowlist('{}')).toEqual({})
  })
  test('rejects wildcard grants, missing tool names and malformed gateway authority', () => {
    for (const value of [
      '[]',
      'null',
      '{"*": ["search"]}',
      '{"docs":["*"]}',
      '{"docs":[]}',
      '{"docs":"search"}',
      '{"docs":["bad tool"]}',
    ])
      expect(() => parseMcpAllowlist(value)).toThrow()
  })
})

test('session execution proposals use the current JSON contract and reject retired grants', () => {
  const draft = { ...integrationTemplate(source, 1000), mode: 'session' }
  expect(parseIntegration(JSON.stringify(draft))).toEqual(draft)
  expect(() => parseIntegration(JSON.stringify({ ...draft, mode: 'task' }))).toThrow()
  expect(() => parseIntegration(JSON.stringify({ ...draft, max_task_seconds: 300 }))).toThrow()
  for (const max_execution_seconds of [0, -1, 1.5, null, '300'])
    expect(() => parseIntegration(JSON.stringify({ ...draft, max_execution_seconds }))).toThrow()
  const channel_policy = { grants: { operator: { tools: ['Subagent'], background_work: true } } }
  expect(() => parseIntegration(JSON.stringify({ ...draft, channel_policy }))).toThrow()
  expect(() =>
    parseIntegration(JSON.stringify({ ...draft, channel_policy: { service_grant: { background_work: false } } })),
  ).toThrow()
  const valid = { grants: { operator: { tools: ['Subagent'] } } }
  expect(parseIntegration(JSON.stringify({ ...draft, channel_policy: valid })).channel_policy).toEqual(valid)
})
