import { describe, expect, test } from 'bun:test'
import { ref } from 'vue'

import type {
  AuthProvider,
  ModelCatalogEntry,
  PermissionRuleResource,
  PluginInspect,
  PluginLogEntry,
  PluginStatus,
  ProviderModel,
  ProviderSummary,
  RuntimeStatus,
  SessionExecutionResource,
  SessionResource,
  TimelineEventRecord,
  WorkspaceResource,
} from '../lib/agenaApi'
import type { PluginsTab, RuntimeRouteSection, SettingsTab } from './runtimePageStateModel'
import { useRuntimeSectionLoadActions } from './useRuntimeSectionLoadActions'

function sampleRuntimeStatus(overrides: Partial<RuntimeStatus> = {}): RuntimeStatus {
  return {
    generation: 1,
    loaded_at: '',
    workspace_root: '/repo',
    config_path: '/repo/.agena/config.json',
    config_found: true,
    auth_store_path: '/repo/.agena/auth.json',
    provider_ids: [],
    plugin_count: 0,
    session_runtime_available: true,
    watch_paths: [],
    reload: { enabled: true, interval_secs: 1 },
    janitor: { enabled: true, interval_secs: 1 },
    model_catalog: {
      entry_count: 0,
      official_entry_count: 0,
      custom_entry_count: 0,
    },
    automation: { enabled: false, job_count: 0, recent_jobs: [] },
    operator: {
      mcp: { server_count: 0, tool_count: 0, servers: [] },
      lsp: { server_count: 0, diagnostics_count: 0, files_with_diagnostics: 0, servers: [] },
      agents: {
        default_agent: 'build',
        total_count: 0,
        primary_count: 0,
        subagent_count: 0,
        hidden_count: 0,
        agents: [],
      },
      skills: { skill_count: 0, command_count: 0, skills: [], commands: [] },
    },
    ...overrides,
  }
}

function createDeps(overrides: Partial<Parameters<typeof useRuntimeSectionLoadActions>[1]> = {}) {
  return {
    loadPluginsSectionData: async () => ({
      plugins: [],
      workspaces: [],
      selectedWorkspaceId: null,
      selectedPluginId: '',
    }),
    loadRuntimeSectionData: async () => ({
      runtime: null as never,
      providers: [],
      providerModels: {},
      workspaces: [],
      sessions: [],
      selectedWorkspaceId: null,
      selectedSessionId: null,
    }),
    loadSettingsSectionData: async () => ({
      authProviders: [],
      runtime: sampleRuntimeStatus(),
      providers: [],
      providerModels: {},
      permissionRules: [],
    }),
    ...overrides,
  }
}

function createState(section: RuntimeRouteSection = 'runtime') {
  const calls: string[] = []
  const state = {
    actionError: ref('stale'),
    activePluginsTab: ref<PluginsTab>('installed'),
    activeSettingsTab: ref<SettingsTab>('providers'),
    authProviders: ref<AuthProvider[]>([]),
    catalogEntries: ref<ModelCatalogEntry[]>([]),
    desktopEnabled: ref(true),
    loadDesktopPanel: async () => {
      calls.push('loadDesktopPanel')
    },
    loadMarketplacePanel: async () => {
      calls.push('loadMarketplacePanel')
    },
    loadPluginDetails: async (pluginId: string) => {
      calls.push(`loadPluginDetails:${pluginId}`)
    },
    loadSessionExecution: async (sessionId: number) => {
      calls.push(`loadSessionExecution:${sessionId}`)
    },
    loading: ref(false),
    permissionRules: ref<PermissionRuleResource[]>([]),
    permissionSearch: ref('bash'),
    pluginLogs: ref<PluginLogEntry[]>([
      { seq: 1, plugin_id: 'demo/plugin', level: 'info', message: 'old', timestamp_ms: 1 },
    ]),
    plugins: ref<PluginStatus[]>([]),
    providers: ref<ProviderSummary[]>([]),
    replaceProviderModels: (providerModels: Record<string, ProviderModel[]>) => {
      calls.push(`replaceProviderModels:${Object.keys(providerModels).join(',')}`)
    },
    routeSection: ref(section),
    runtime: ref<RuntimeStatus | null>(null),
    selectedPlugin: ref<PluginInspect | null>({
      manifest: { name: 'old' },
      status: { plugin_id: 'old', kind: 'wasm', state: 'ready', restart_count: 0 },
    }),
    selectedPluginId: ref('demo/plugin'),
    selectedSessionId: ref<number | null>(10),
    selectedWorkspaceId: ref<number | null>(1),
    sessionExecution: ref<SessionExecutionResource | null>({
      session: {
        id: 10,
        workspace_id: 1,
        title: 'Old',
        version: 1,
        created_at: '',
        updated_at: '',
        message_count: 0,
        child_session_count: 0,
      },
      blocked: false,
      run_state: 'idle',
      execution: { allowed_tools: [] },
      pending_permission_requests: [],
      pending_user_input_requests: [],
      usage: { current_tokens: 0 },
    }),
    sessionTimeline: ref<TimelineEventRecord[]>([{ seq_global: 1, kind: 'old', payload: {} }]),
    sessions: ref<SessionResource[]>([]),
    stopPluginLogPolling: () => {
      calls.push('stopPluginLogPolling')
    },
    workspaces: ref<WorkspaceResource[]>([]),
  }

  return { calls, state }
}

describe('useRuntimeSectionLoadActions', () => {
  test('loadRuntimeSection hydrates runtime state and loads session execution', async () => {
    const { calls, state } = createState('runtime')
    const actions = useRuntimeSectionLoadActions(
      state,
      createDeps({
        loadRuntimeSectionData: async () => ({
          runtime: {
            generation: 3,
            loaded_at: '',
            workspace_root: '/repo',
            config_path: '/repo/.agena/config.json',
            config_found: true,
            auth_store_path: '/repo/.agena/auth.json',
            provider_ids: ['anthropic'],
            plugin_count: 1,
            session_runtime_available: true,
            watch_paths: [],
            reload: { enabled: true, interval_secs: 1 },
            janitor: { enabled: true, interval_secs: 1 },
            automation: { enabled: false, job_count: 0, recent_jobs: [] },
            operator: {
              mcp: { server_count: 0, tool_count: 0, servers: [] },
              lsp: { server_count: 0, diagnostics_count: 0, files_with_diagnostics: 0, servers: [] },
              agents: {
                default_agent: 'build',
                total_count: 8,
                primary_count: 7,
                subagent_count: 6,
                hidden_count: 0,
                agents: [],
              },
              skills: { skill_count: 0, command_count: 0, skills: [], commands: [] },
            },
          },
          providers: [
            {
              provider_id: 'anthropic',
              default_model: 'claude-opus-4-7',
            },
          ],
          providerModels: { anthropic: [] },
          workspaces: [{ id: 2, path: '/repo', created_at: '', updated_at: '' }],
          sessions: [
            {
              id: 22,
              workspace_id: 2,
              title: 'Session 22',
              version: 1,
              created_at: '',
              updated_at: '',
              message_count: 0,
              child_session_count: 0,
            },
          ],
          selectedWorkspaceId: 2,
          selectedSessionId: 22,
        }),
      }),
    )

    await actions.loadRuntimeSection()

    expect(calls).toEqual(['replaceProviderModels:anthropic', 'loadSessionExecution:22'])
    expect(state.selectedWorkspaceId.value).toBe(2)
    expect(state.selectedSessionId.value).toBe(22)
    expect(state.providers.value.map((provider) => provider.provider_id)).toEqual(['anthropic'])
  })

  test('loadSettingsSection hydrates settings state and desktop panel when needed', async () => {
    const { calls, state } = createState('settings')
    state.activeSettingsTab.value = 'desktop'
    const actions = useRuntimeSectionLoadActions(
      state,
      createDeps({
        loadSettingsSectionData: async (search) => {
          calls.push(`loadSettingsSectionData:${search}`)
          return {
            authProviders: [{ provider_id: 'anthropic', configured: true, credential_present: true }],
            runtime: sampleRuntimeStatus({
              model_catalog: {
                entry_count: 1,
                official_entry_count: 1,
                custom_entry_count: 0,
              },
            }),
            providers: [
              {
                provider_id: 'anthropic',
                default_model: 'claude-opus-4-7',
              },
            ],
            providerModels: { anthropic: [] },
            permissionRules: [
              {
                id: 1,
                action_key: 'Bash:ls',
                subject_kind: 'tool',
                tool_name: 'bash',
                qualifier: null,
                mode: 'ask',
                scope: 'workspace',
                source: 'api',
                created_at: '',
                updated_at: '',
              },
            ],
          }
        },
      }),
    )

    await actions.loadSettingsSection()

    expect(calls).toEqual(['loadSettingsSectionData:bash', 'replaceProviderModels:anthropic', 'loadDesktopPanel'])
    expect(state.authProviders.value.map((provider) => provider.provider_id)).toEqual(['anthropic'])
    expect(state.catalogEntries.value).toEqual([])
    expect(state.permissionRules.value.map((rule) => rule.id)).toEqual([1])
  })

  test('loadPluginsSection hydrates plugins and clears details when no plugin is selected', async () => {
    const { calls, state } = createState('plugins')
    state.activePluginsTab.value = 'marketplace'
    const actions = useRuntimeSectionLoadActions(
      state,
      createDeps({
        loadPluginsSectionData: async () => ({
          plugins: [{ plugin_id: 'demo/plugin', kind: 'wasm', state: 'ready', restart_count: 0 }],
          workspaces: [{ id: 3, path: '/repo', created_at: '', updated_at: '' }],
          selectedWorkspaceId: 3,
          selectedPluginId: '',
        }),
      }),
    )

    await actions.loadPluginsSection()

    expect(calls).toEqual(['stopPluginLogPolling', 'loadMarketplacePanel'])
    expect(state.selectedWorkspaceId.value).toBe(3)
    expect(state.selectedPlugin.value === null).toBe(true)
    expect(state.pluginLogs.value).toEqual([])
  })

  test('load dispatches by route section and clears stale error', async () => {
    const { calls, state } = createState('plugins')
    const actions = useRuntimeSectionLoadActions(
      state,
      createDeps({
        loadPluginsSectionData: async () => ({
          plugins: [],
          workspaces: [],
          selectedWorkspaceId: null,
          selectedPluginId: 'demo/plugin',
        }),
      }),
    )

    await actions.load()

    expect(calls).toEqual(['loadPluginDetails:demo/plugin'])
    expect(state.actionError.value).toBe('')
    expect(state.loading.value).toBe(false)
  })
})
