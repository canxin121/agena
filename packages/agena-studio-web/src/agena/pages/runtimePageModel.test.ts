import { describe, expect, test } from 'bun:test'

import type { PluginStatus, RuntimeStatus } from '@/agena/lib/agenaApi'
import { buildOperatorCards, pickNextPluginId } from './runtimePageModel'

function sampleRuntime(): RuntimeStatus {
  return {
    generation: 7,
    loaded_at: '2026-05-05T00:00:00Z',
    workspace_root: '/workspace',
    config_path: '/workspace/.agena/config.toml',
    config_found: true,
    active_mode: 'default',
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
