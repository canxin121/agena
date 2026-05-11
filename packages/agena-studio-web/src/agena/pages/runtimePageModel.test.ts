import { describe, expect, test } from 'bun:test'

import type {
  AuthProvider,
  PluginLogEntry,
  PluginStatus,
  ProviderModel,
  RuntimeStatus,
  SessionExecutionResource,
  TimelineEventRecord,
} from '@/agena/lib/agenaApi'
import {
  buildAuthProviderFacts,
  buildExecutionFacts,
  buildOperatorCards,
  buildRuntimeSnapshotFacts,
  buildSessionCacheFacts,
  buildTimelineSummary,
  formatProviderModel,
  mergePluginLogs,
  pickNextPluginId,
  pluginLogCursor,
} from './runtimePageModel'

function sampleRuntime(): RuntimeStatus {
  return {
    generation: 7,
    loaded_at: '2026-05-05T00:00:00Z',
    workspace_root: '/workspace',
    config_path: '/workspace/.agena/config.toml',
    config_found: true,
    auth_store_path: '/workspace/.agena/auth.json',
    provider_ids: ['anthropic', 'openai'],
    plugin_count: 3,
    session_runtime_available: true,
    watch_paths: ['/workspace/.agena/config.toml'],
    reload: {
      enabled: true,
      interval_secs: 2,
    },
    janitor: {
      enabled: true,
      interval_secs: 30,
    },
    session_cache: {
      max_sessions: 128,
      ttl_secs: 900,
      max_bytes: 67108864,
      entry_count: 1,
      total_bytes: 512,
      hits: 2,
      misses: 1,
      inserts: 1,
      evictions: 0,
    },
    automation: {
      enabled: true,
      job_count: 1,
      recent_jobs: [],
    },
    operator: {
      mcp: {
        server_count: 4,
        tool_count: 9,
        servers: [{ name: 'filesystem', tool_count: 3 }],
      },
      lsp: {
        server_count: 2,
        diagnostics_count: 5,
        files_with_diagnostics: 2,
        servers: [
          {
            name: 'rust-analyzer',
            command: 'rust-analyzer',
            file_extensions: ['rs'],
            root_markers: ['Cargo.toml'],
          },
        ],
      },
      skills: {
        skill_count: 6,
        command_count: 1,
        skills: [],
        commands: [],
      },
    },
  }
}

function samplePlugins(): PluginStatus[] {
  return [
    {
      plugin_id: 'alpha',
      kind: 'native',
      state: 'running',
      pid: 1,
      restart_count: 0,
      last_exit_code: null,
      last_restart_at_ms: null,
      last_error: null,
    },
    {
      plugin_id: 'beta',
      kind: 'native',
      state: 'stopped',
      pid: null,
      restart_count: 1,
      last_exit_code: 1,
      last_restart_at_ms: null,
      last_error: 'boom',
    },
  ]
}

function sampleAuthProvider(): AuthProvider {
  return {
    provider_id: 'openai',
    configured: true,
    credential_present: true,
    credential_type: 'api',
    key_preview: 'sk-...abcd',
    expires_at: '2026-05-05T01:00:00Z',
    expired: false,
    account_id: 'acct_123',
    enterprise_url: 'https://example.internal',
  }
}

function sampleExecution(): SessionExecutionResource {
  return {
    session: {
      id: 42,
      workspace_id: 7,
      title: 'workflow demo',
      version: 3,
      created_at: '2026-05-05T00:00:00Z',
      updated_at: '2026-05-05T00:01:00Z',
      message_count: 4,
      child_session_count: 1,
      parent_id: null,
      last_message_at: '2026-05-05T00:01:00Z',
    },
    blocked: true,
    run_state: 'awaiting_model',
    latest_event_seq: 12,
    automation: null,
    execution: {
      agent_profile: 'planner',
      active_skill_name: 'edit',
      system_prompt_override: null,
      allowed_tools: ['Read', 'Edit'],
      model_provider_id: 'openai',
      model_id: 'gpt-4.1-mini',
      effective_workspace_root: '/workspace',
      task_id: 'task-1',
    },
    pending_permission_requests: [
      {
        request_id: 'perm-1',
        action: {},
        reason: 'need shell',
        explanation: 'matched static permission policy',
        source: 'static_policy',
        scope: null,
        operator: null,
        created_at: '2026-05-05T00:01:00Z',
        session_id: 42,
      },
    ],
    pending_user_input_requests: [],
  }
}

function sampleTimelineEvents(): TimelineEventRecord[] {
  return [
    {
      seq_global: 10,
      kind: 'run_started',
      payload: { summary: 'Run started' },
      created_at: '2026-05-05T00:00:10Z',
      session_id: 42,
    },
    {
      seq_global: 11,
      kind: 'command_begin',
      payload: { command: 'ls -la' },
      ts_ms: 1_746_400_000_000,
      session_id: 42,
    },
  ]
}

function samplePluginLogs(): PluginLogEntry[] {
  return [
    {
      seq: 1,
      plugin_id: 'alpha',
      level: 'info',
      target: 'plugin',
      message: 'started',
      timestamp_ms: 1,
    },
    {
      seq: 2,
      plugin_id: 'alpha',
      level: 'warn',
      target: 'plugin',
      message: 'slow',
      timestamp_ms: 2,
    },
  ]
}

function sampleProviderModel(): ProviderModel {
  return {
    provider_id: 'openai',
    id: 'gpt-4.1-mini',
    display_name: 'GPT-4.1 Mini',
    capabilities: {},
    metadata: {},
  }
}

describe('runtimePageModel', () => {
  test('buildOperatorCards summarizes runtime operator counts', () => {
    expect(buildOperatorCards(sampleRuntime())).toEqual([
      { label: 'Generation', value: '7' },
      { label: 'Providers', value: '2' },
      { label: 'Plugins', value: '3' },
      { label: 'MCP Servers', value: '4' },
      { label: 'LSP Servers', value: '2' },
      { label: 'Skills', value: '6' },
    ])
  })

  test('buildOperatorCards returns empty list without runtime', () => {
    expect(buildOperatorCards(null)).toEqual([])
  })

  test('buildRuntimeSnapshotFacts omits removed mode facts', () => {
    const facts = buildRuntimeSnapshotFacts(sampleRuntime())
    expect(facts.find((fact) => fact.label === 'Mode Source')).toBeUndefined()
    expect(facts.find((fact) => fact.label === 'Active Mode')).toBeUndefined()
  })

  test('buildSessionCacheFacts includes max bytes', () => {
    const facts = buildSessionCacheFacts(sampleRuntime())
    expect(facts.find((fact) => fact.label === 'Max Bytes')?.value).toBe('67108864')
  })

  test('buildAuthProviderFacts includes auth detail fields', () => {
    const facts = buildAuthProviderFacts(sampleAuthProvider())
    expect(facts.find((fact) => fact.label === 'Account')?.value).toBe('acct_123')
    expect(facts.find((fact) => fact.label === 'Enterprise URL')?.value).toBe('https://example.internal')
  })

  test('buildExecutionFacts summarizes blocked workflow state', () => {
    const facts = buildExecutionFacts(sampleExecution())
    expect(facts.find((fact) => fact.label === 'Blocked')?.value).toBe('yes')
    expect(facts.find((fact) => fact.label === 'Pending Permissions')?.value).toBe('1')
    expect(facts.find((fact) => fact.label === 'Model')?.value).toBe('openai/gpt-4.1-mini')
  })

  test('buildTimelineSummary renders event summaries', () => {
    const summaries = buildTimelineSummary(sampleTimelineEvents())
    expect({ kind: summaries[0]?.kind, summary: summaries[0]?.summary }).toEqual({
      kind: 'run_started',
      summary: 'Run started',
    })
    expect({ kind: summaries[1]?.kind, summary: summaries[1]?.summary }).toEqual({
      kind: 'command_begin',
      summary: 'ls -la',
    })
  })

  test('mergePluginLogs deduplicates by sequence and keeps order', () => {
    const merged = mergePluginLogs(samplePluginLogs(), [
      samplePluginLogs()[1],
      {
        seq: 3,
        plugin_id: 'alpha',
        level: 'info',
        target: 'plugin',
        message: 'done',
        timestamp_ms: 3,
      },
    ])
    expect(merged.map((entry) => entry.seq)).toEqual([1, 2, 3])
  })

  test('pluginLogCursor returns the latest sequence', () => {
    expect(pluginLogCursor(samplePluginLogs())).toBe(2)
    expect(pluginLogCursor([])).toBe(null)
  })

  test('formatProviderModel prefers display name', () => {
    expect(formatProviderModel(sampleProviderModel())).toBe('GPT-4.1 Mini')
    expect(formatProviderModel({ ...sampleProviderModel(), display_name: '' })).toBe('gpt-4.1-mini')
  })

  test('pickNextPluginId keeps current plugin when still present', () => {
    expect(pickNextPluginId('beta', samplePlugins())).toBe('beta')
  })

  test('pickNextPluginId falls back to first plugin when selection is missing', () => {
    expect(pickNextPluginId('missing', samplePlugins())).toBe('alpha')
  })

  test('pickNextPluginId returns empty string when no plugins exist', () => {
    expect(pickNextPluginId('anything', [])).toBe('')
  })
})
