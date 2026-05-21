import { describe, expect, test } from 'bun:test'
import { ref } from 'vue'

import type { RuntimeStatus } from '../lib/agenaApi'
import { useSettingsAgentsState } from './useSettingsAgentsState'

function createRuntime(): RuntimeStatus {
  return {
    generation: 1,
    loaded_at: '2026-05-10T00:00:00Z',
    workspace_root: '/workspace',
    config_path: '/workspace/.agena/config.json',
    config_found: true,
    auth_store_path: '/workspace/.agena/auth.json',
    provider_ids: [],
    plugin_count: 0,
    session_runtime_available: true,
    watch_paths: [],
    reload: { enabled: true, interval_secs: 5 },
    janitor: { enabled: true, interval_secs: 60 },
    session_cache: null,
    model_catalog: null,
    automation: { enabled: false, job_count: 0, recent_jobs: [] },
    operator: {
      mcp: { server_count: 0, tool_count: 0, servers: [] },
      lsp: { server_count: 0, diagnostics_count: 0, files_with_diagnostics: 0, servers: [] },
      agents: {
        default_agent: 'build',
        total_count: 2,
        primary_count: 1,
        subagent_count: 1,
        hidden_count: 1,
        agents: [
          {
            name: 'build',
            description: 'Build agent',
            mode: 'primary',
            hidden: false,
            color: null,
            temperature: null,
            max_output_tokens: null,
            steps: null,
            allowed_tools: ['bash'],
            permission: { inherit: true },
            default: {},
            aliases: ['default'],
            scope: 'project',
            source_path: '/workspace/.agena/config.json',
          },
          {
            name: 'review',
            description: 'Review agent',
            mode: 'subagent',
            hidden: true,
            color: null,
            temperature: null,
            max_output_tokens: null,
            steps: null,
            allowed_tools: ['fs'],
            permission: { inherit: { tools: true, path: true } },
            default: { model: 'gpt-5' },
            aliases: [],
            scope: 'default',
            source_path: null,
          },
        ],
      },
      skills: { skill_count: 0, command_count: 0, skills: [], commands: [] },
    },
  }
}

describe('useSettingsAgentsState', () => {
  test('summarizes agents and patches config for editable ones', async () => {
    const calls: Array<Record<string, unknown>> = []
    const state = useSettingsAgentsState(
      {
        actionError: ref(''),
        actionMessage: ref(''),
        load: async () => {
          calls.push({ kind: 'load' })
        },
        runtime: ref(createRuntime()),
      },
      {
        patchSettings: async (input) => {
          calls.push(input)
          return {
            config_path: '/workspace/.agena/config.json',
            config_found: true,
            operation: 'patch',
            path: input.path ?? null,
            dry_run: false,
            changed: true,
            created: false,
            deleted: false,
            validated: true,
            reload_requested: true,
            reload_required: false,
            reload: null,
            previous: {},
            current: {},
          }
        },
      },
    )

    expect(state.summaryFacts.value[0]?.value).toBe('build')
    expect(state.agentCards.value[0]?.name).toBe('build')
    expect(state.agentCards.value[0]?.canToggleHidden).toBe(true)
    expect(state.agentCards.value[1]?.canToggleHidden).toBe(false)
    expect(state.agentCards.value[1]?.permissionSummary).toContain('inherit=tools,path')

    await state.setDefaultAgent('review')
    expect(calls[0]).toEqual({
      path: 'default',
      changes: { agent: 'review' },
      validate: true,
      reload: true,
    })
    expect(calls[1]).toEqual({ kind: 'load' })
    expect(state.actionMessage.value).toBe('Default agent set to review.')

    calls.length = 0
    await state.toggleAgentHidden(state.agentCards.value[0]!)
    expect(calls[0]).toEqual({
      path: 'agents',
      changes: { build: { hidden: true } },
      validate: true,
      reload: true,
    })
    expect(calls[1]).toEqual({ kind: 'load' })

    calls.length = 0
    await state.toggleAgentHidden(state.agentCards.value[1]!)
    expect(state.actionError.value).toContain('managed outside this config file')
    expect(calls).toEqual([])
  })
})
