import { describe, expect, test } from 'bun:test'
import { ref } from 'vue'

import type {
  GlobalEventRecord,
  MarketplaceInstalledPluginResource,
  MarketplacePluginResource,
  PermissionRuleResource,
  PluginInspect,
  RuntimeStatus,
  SessionExecutionResource,
  TimelineEventRecord,
} from '../lib/agenaApi'
import type {
  DesktopBackendStatus,
  DesktopConfig,
  DesktopRuntimeInfo,
  DesktopUpdateProgress,
} from '../../lib/desktopConfig'
import { useRuntimeDerivedState } from './useRuntimeDerivedState'

function runtime(): RuntimeStatus {
  return {
    generation: 7,
    loaded_at: '2026-05-10T00:00:00Z',
    workspace_root: '/repo',
    config_path: '/repo/.agena/config.toml',
    config_found: true,
    auth_store_path: '/repo/.agena/auth.json',
    provider_ids: ['anthropic'],
    plugin_count: 2,
    session_runtime_available: true,
    watch_paths: ['/repo/.agena/config.toml'],
    reload: { enabled: true, interval_secs: 2 },
    janitor: { enabled: true, interval_secs: 30 },
    session_cache: {
      max_sessions: 32,
      ttl_secs: 600,
      max_bytes: 1024,
      entry_count: 2,
      total_bytes: 256,
      hits: 4,
      misses: 1,
      inserts: 2,
      evictions: 0,
    },
    automation: {
      enabled: true,
      job_count: 0,
      recent_jobs: [],
    },
    operator: {
      mcp: {
        server_count: 1,
        tool_count: 3,
        servers: [{ name: 'filesystem', tool_count: 3 }],
      },
      lsp: {
        server_count: 1,
        diagnostics_count: 2,
        files_with_diagnostics: 1,
        servers: [
          {
            name: 'tsserver',
            command: 'typescript-language-server --stdio',
            file_extensions: ['ts', 'tsx'],
            root_markers: ['package.json'],
          },
        ],
      },
      agents: {
        default_agent: 'build',
        total_count: 8,
        primary_count: 7,
        subagent_count: 6,
        hidden_count: 0,
        agents: [],
      },
      skills: {
        skill_count: 2,
        command_count: 1,
        skills: [
          { name: 'review', description: 'Review code', aliases: ['rv'], source_path: '.agena/skills/review.md' },
          { name: 'summarize', description: 'Summarize logs', aliases: [], source_path: '.agena/skills/summarize.md' },
        ],
        commands: [
          { name: 'deploy', description: 'Deploy app', aliases: ['ship'], source_path: '.agena/commands/deploy.md' },
        ],
      },
    },
  }
}

function sessionExecution(): SessionExecutionResource {
  return {
    session: {
      id: 9,
      workspace_id: 1,
      title: 'demo',
      version: 1,
      created_at: '2026-05-10T00:00:00Z',
      updated_at: '2026-05-10T00:00:00Z',
      message_count: 0,
      child_session_count: 0,
    },
    blocked: false,
    run_state: 'idle',
    latest_event_seq: 1,
    execution: {
      agent_profile: 'planner',
      active_skill_name: 'review',
      allowed_tools: ['Read'],
      model_provider_id: 'anthropic',
      model_id: 'claude-opus-4-7',
      effective_workspace_root: '/repo',
      task_id: 'task-1',
    },
    pending_permission_requests: [],
    pending_user_input_requests: [],
  }
}

function desktopConfig(): DesktopConfig {
  return {
    autostart_on_boot: true,
    backend: {
      host: '127.0.0.1',
      port: 3210,
      cors_origins: [],
      cors_allow_all: false,
      backend_log_level: 'info',
      ui_password: null,
      ui_cookie_samesite: 'lax',
      agena_config_path: '/repo/.agena/config.toml',
      workspace_root: '/repo',
      database_path: '/repo/.agena/agena.db',
      database_url: null,
    },
  }
}

function desktopStatus(): DesktopBackendStatus {
  return {
    running: false,
    url: '  http://127.0.0.1:3210  ',
    last_error: 'boom',
    last_error_info: {
      code: 'EADDRINUSE',
      summary: 'Port in use',
      detail: '',
      hint: '',
      exitCode: 2,
      signal: null,
    },
  }
}

function desktopRuntimeInfo(): DesktopRuntimeInfo {
  return {
    installerVersion: '1.0.0',
    installerTarget: 'linux-x64',
    installerChannel: 'main',
    installerType: 'deb',
    installerManager: 'apt',
  }
}

function desktopUpdate(): DesktopUpdateProgress {
  return {
    running: true,
    kind: 'service',
    phase: 'download',
    message: 'downloading',
    downloadedBytes: 25,
    totalBytes: 50,
    error: null,
  }
}

describe('useRuntimeDerivedState', () => {
  test('computes route metadata, desktop facts, workflow facts, and plugin manifest', () => {
    const derived = useRuntimeDerivedState({
      desktopConfig: ref<DesktopConfig | null>(desktopConfig()),
      desktopRuntimeState: ref<DesktopRuntimeInfo | null>(desktopRuntimeInfo()),
      desktopStatus: ref<DesktopBackendStatus | null>(desktopStatus()),
      desktopUpdate: ref<DesktopUpdateProgress | null>(desktopUpdate()),
      lspQuery: ref(''),
      marketplaceInstalled: ref<MarketplaceInstalledPluginResource[]>([]),
      marketplacePlugins: ref<MarketplacePluginResource[]>([]),
      marketplaceQuery: ref(''),
      mcpQuery: ref(''),
      permissionModeFilter: ref<'all' | 'allow' | 'ask' | 'deny'>('all'),
      permissionRules: ref<PermissionRuleResource[]>([]),
      permissionScopeFilter: ref<'all' | 'session' | 'workspace' | 'global'>('all'),
      permissionStatusFilter: ref<'all' | 'active' | 'revoked'>('active'),
      permissionSubjectFilter: ref<'all' | 'tool' | 'path_access'>('all'),
      routePath: ref('/settings/desktop'),
      runtime: ref<RuntimeStatus | null>(runtime()),
      runtimeSkillQuery: ref(''),
      globalEvents: ref<GlobalEventRecord[]>([
        {
          id: 'event-1',
          seq_global: 101,
          session_id: 9,
          workspace_id: 1,
          created_at: '2026-05-10T00:00:01Z',
          kind: 'turn_started',
          payload: { summary: 'Turn started' },
        },
      ]),
      selectedPlugin: ref<PluginInspect | null>({
        status: {
          plugin_id: 'demo/plugin',
          kind: 'wasm',
          state: 'ready',
          restart_count: 0,
        },
        manifest: { name: 'demo-plugin' },
      }),
      sessionExecution: ref<SessionExecutionResource | null>(sessionExecution()),
      sessionTimeline: ref<TimelineEventRecord[]>([
        {
          seq_global: 1,
          kind: 'run_started',
          payload: { summary: 'Run started' },
          session_id: 9,
          created_at: '2026-05-10T00:00:00Z',
        },
      ]),
      tabs: [
        { id: 'overview', label: 'Overview' },
        { id: 'workflow', label: 'Workflow' },
      ],
    })

    expect(derived.routeSection.value).toBe('settings')
    expect(derived.pageTitle.value).toBe('Settings')
    expect(derived.pageDescription.value).toBe(
      'Manage providers, credentials, permission rules, and desktop configuration.',
    )
    expect(derived.visibleTabs.value).toEqual([])
    expect(derived.desktopBackendUrl.value).toBe('http://127.0.0.1:3210')
    expect(derived.desktopBackendErrorFacts.value).toEqual([
      { label: 'Code', value: 'EADDRINUSE', mono: true },
      { label: 'Summary', value: 'Port in use' },
      { label: 'Detail', value: 'n/a' },
      { label: 'Hint', value: 'n/a' },
      { label: 'Exit Code', value: '2', mono: true },
      { label: 'Signal', value: 'n/a', mono: true },
    ])
    expect(derived.desktopUpdateProgressPercent.value).toBe('50%')
    expect(derived.operatorCards.value.length).toBe(7)
    expect(derived.runtimeSnapshotFacts.value.length > 0).toBe(true)
    expect(derived.sessionCacheFacts.value.length > 0).toBe(true)
    expect(derived.executionFacts.value.length > 0).toBe(true)
    expect(derived.globalEventSummaries.value.length).toBe(1)
    expect(derived.timelineSummaries.value.length).toBe(1)
    expect(derived.selectedPluginManifest.value).toEqual({ name: 'demo-plugin' })
  })

  test('filters skills, mcp, lsp, permissions, marketplace, and runtime tabs by query and section', () => {
    const permissionRules = ref<PermissionRuleResource[]>([
      {
        id: 1,
        action_key: 'tool:bash',
        subject_kind: 'tool',
        tool_name: 'bash',
        qualifier: 'git status *',
        path_access_kind: null,
        workspace_root: null,
        target_path: null,
        mode: 'allow',
        scope: 'workspace',
        session_id: null,
        workspace_id: null,
        source: 'manual',
        reason: null,
        operator: null,
        revoked_at: null,
        revoked_reason: null,
        revoked_by: null,
        created_at: '2026-05-10T00:00:00Z',
        updated_at: '2026-05-10T00:00:00Z',
      },
      {
        id: 2,
        action_key: 'path:read',
        subject_kind: 'path_access',
        tool_name: null,
        qualifier: null,
        path_access_kind: 'read',
        workspace_root: '/repo',
        target_path: 'src',
        mode: 'deny',
        scope: 'session',
        session_id: 9,
        workspace_id: null,
        source: 'manual',
        reason: null,
        operator: null,
        revoked_at: '2026-05-10T00:00:00Z',
        revoked_reason: null,
        revoked_by: null,
        created_at: '2026-05-10T00:00:00Z',
        updated_at: '2026-05-10T00:00:00Z',
      },
    ])
    const marketplacePlugins = ref<MarketplacePluginResource[]>([
      {
        plugin_id: 'demo/plugin',
        name: 'Demo Plugin',
        description: 'Installed example plugin',
        homepage: null,
        version_count: 2,
        latest_version: '1.1.0',
        latest_kind: 'wasm',
        latest_platform: 'any',
      },
      {
        plugin_id: 'logs/plugin',
        name: 'Log Helper',
        description: 'Log search helper',
        homepage: null,
        version_count: 1,
        latest_version: '0.1.0',
        latest_kind: 'native',
        latest_platform: 'linux-x64',
      },
    ])
    const marketplaceInstalled = ref<MarketplaceInstalledPluginResource[]>([
      {
        plugin_id: 'demo/plugin',
        version: '1.0.0',
        kind: 'wasm',
        platform: 'any',
        binary_path: '/plugins/demo/plugin.wasm',
        config_path: '/plugins/demo/plugin.json',
        installed_at: '2026-05-10T00:00:00Z',
        registry_id: 'default',
        registry_url: 'https://registry.example.test',
        archive_extracted: false,
      },
    ])
    const derived = useRuntimeDerivedState({
      desktopConfig: ref<DesktopConfig | null>(null),
      desktopRuntimeState: ref<DesktopRuntimeInfo | null>(null),
      desktopStatus: ref<DesktopBackendStatus | null>(null),
      desktopUpdate: ref<DesktopUpdateProgress | null>(null),
      lspQuery: ref('tsserver'),
      marketplaceInstalled,
      marketplacePlugins,
      marketplaceQuery: ref('installed'),
      mcpQuery: ref('filesystem 3'),
      permissionModeFilter: ref<'all' | 'allow' | 'ask' | 'deny'>('allow'),
      permissionRules,
      permissionScopeFilter: ref<'all' | 'session' | 'workspace' | 'global'>('workspace'),
      permissionStatusFilter: ref<'all' | 'active' | 'revoked'>('active'),
      permissionSubjectFilter: ref<'all' | 'tool' | 'path_access'>('tool'),
      routePath: ref('/runtime/workflow'),
      runtime: ref<RuntimeStatus | null>(runtime()),
      runtimeSkillQuery: ref('review'),
      globalEvents: ref<GlobalEventRecord[]>([]),
      selectedPlugin: ref<PluginInspect | null>(null),
      sessionExecution: ref<SessionExecutionResource | null>(null),
      sessionTimeline: ref<TimelineEventRecord[]>([]),
      tabs: [
        { id: 'overview', label: 'Overview' },
        { id: 'workflow', label: 'Workflow' },
      ],
    })

    expect(derived.routeSection.value).toBe('runtime')
    expect(derived.pageTitle.value).toBe('Runtime')
    expect(derived.visibleTabs.value.map((item) => item.id)).toEqual(['overview', 'workflow'])
    expect(derived.skillCommands.value.map((item) => item.name)).toEqual(['deploy'])
    expect(derived.discoveredSkills.value.map((item) => item.name)).toEqual(['review', 'summarize'])
    expect(derived.filteredSkillCommands.value).toEqual([])
    expect(derived.filteredDiscoveredSkills.value.map((item) => item.name)).toEqual(['review'])
    expect(derived.filteredMcpServers.value.map((item) => item.name)).toEqual(['filesystem'])
    expect(derived.filteredLspServers.value.map((item) => item.name)).toEqual(['tsserver'])
    expect(derived.filteredPermissionRules.value.map((item) => item.id)).toEqual([1])
    expect(derived.filteredMarketplacePlugins.value.map((item) => item.plugin_id)).toEqual(['demo/plugin'])
    expect(Array.from(derived.installedMarketplacePluginIds.value)).toEqual(['demo/plugin'])
    expect(derived.selectedPluginManifest.value).toBe(null)
  })
})
