import { describe, expect, test } from 'bun:test'

import { renderVueSsr } from './test/renderVueSsr'

const sharedPanels = {
  mcp: {
    runtime: {
      operator: {
        mcp: { server_count: 1, tool_count: 2, servers: [{ name: 'alpha', tool_count: 2 }] },
        lsp: {
          server_count: 1,
          diagnostics_count: 3,
          files_with_diagnostics: 1,
          servers: [
            {
              name: 'ts',
              command: 'typescript-language-server',
              file_extensions: ['ts'],
              root_markers: ['tsconfig.json'],
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
        skills: { skill_count: 4, command_count: 2 },
      },
      config_found: true,
      session_runtime_available: true,
      watch_paths: ['src'],
    },
    filteredMcpServers: [{ name: 'alpha', tool_count: 2 }],
    filteredLspServers: [
      { name: 'ts', command: 'typescript-language-server', file_extensions: ['ts'], root_markers: ['tsconfig.json'] },
    ],
    mcpQuery: '',
    lspQuery: '',
    openRuntimeConfigRoot: () => {},
    openWorkspaceShortcut: () => {},
    openWorkspacePath: () => {},
  },
  lsp: {
    runtime: {
      operator: {
        mcp: { server_count: 1, tool_count: 2, servers: [{ name: 'alpha', tool_count: 2 }] },
        lsp: {
          server_count: 1,
          diagnostics_count: 3,
          files_with_diagnostics: 1,
          servers: [
            {
              name: 'ts',
              command: 'typescript-language-server',
              file_extensions: ['ts'],
              root_markers: ['tsconfig.json'],
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
        skills: { skill_count: 4, command_count: 2 },
      },
      config_found: true,
      session_runtime_available: true,
      watch_paths: ['src'],
    },
    filteredMcpServers: [{ name: 'alpha', tool_count: 2 }],
    filteredLspServers: [
      { name: 'ts', command: 'typescript-language-server', file_extensions: ['ts'], root_markers: ['tsconfig.json'] },
    ],
    mcpQuery: '',
    lspQuery: '',
    openRuntimeConfigRoot: () => {},
    openWorkspaceShortcut: () => {},
    openWorkspacePath: () => {},
  },
  operator: {
    runtime: {
      config_found: true,
      session_runtime_available: true,
      watch_paths: ['src', '.agena'],
      operator: {
        mcp: { server_count: 1, tool_count: 2 },
        lsp: { server_count: 1, diagnostics_count: 3 },
        agents: {
          default_agent: 'build',
          total_count: 8,
          primary_count: 7,
          subagent_count: 6,
          hidden_count: 0,
          agents: [],
        },
        skills: { skill_count: 4, command_count: 2 },
      },
    },
  },
  overview: {
    catalogEntries: [
      {
        model_id: 'gpt-5',
        default_model: 'gpt-5',
        has_local_override: false,
        display_name: 'GPT-5',
        description: 'flagship model',
      },
    ],
    operatorCards: [{ label: 'Providers', value: '2' }],
    runtimeSnapshotFacts: [{ label: 'Workspace Root', value: '/repo', mono: true }],
    runtime: {
      reload: { enabled: true, interval_secs: 10 },
      janitor: { enabled: false, interval_secs: 60 },
      watch_paths: ['src'],
      automation: { recent_jobs: [], enabled: true, job_count: 0 },
      model_catalog: {
        last_successful_source: 'generated',
        last_refresh_at: '2026-05-15T00:00:00Z',
        entry_count: 0,
        official_entry_count: 0,
        custom_entry_count: 0,
      },
    },
    providers: [
      {
        provider_id: 'openai',
        default_model: 'openai/gpt-5',
        adapters: [{ adapter_id: 'openai', enabled: true, configured_model_count: 1 }],
      },
    ],
    providerModels: { openai: [{ provider_id: 'openai', id: 'openai/gpt-5', display_name: 'GPT-5' }] },
    sessionCacheFacts: [{ label: 'Entries', value: '4' }],
    formatProviderModel: (model: { display_name?: string; id: string }) => model.display_name || model.id,
  },
  skills: {
    runtimeSkillQuery: 'review',
    catalogSections: [
      {
        id: 'skills',
        title: 'Skills',
        description: 'Discovered runtime skills',
        badgeLabel: 'skill',
        openShortcutId: 'skills',
        openShortcutLabel: 'Open Skills Dir',
        totalCount: 1,
        filteredCount: 1,
        entries: [
          {
            name: 'review',
            description: 'Review code',
            aliases: ['rv'],
            source_path: '.agena/skills/review.md',
          },
        ],
        emptyLabel: 'No skills',
      },
    ],
    openWorkspaceShortcut: () => {},
    openRuntimeConfigRoot: () => {},
    openPluginLogsWorkspacePath: () => {},
    openRuntimeEntryInChat: () => {},
    openRuntimeEntrySource: () => {},
  },
  workflow: {
    selectedWorkspaceId: 1,
    selectedSessionId: 12,
    workspaces: [{ id: 1, path: '/repo' }],
    sessions: [{ id: 12, title: 'Session 12' }],
    executionFacts: [{ label: 'Run State', value: 'idle' }],
    workflowLoading: false,
    sessionExecution: { pending_permission_requests: [] },
    timelineSummaries: [
      { key: '1', kind: 'assistant', summary: 'Completed', sessionId: 'session 12', timestamp: 'now' },
    ],
    globalEventSummaries: [{ key: '2', kind: 'runtime', summary: 'Reloaded', sessionId: 'global', timestamp: 'now' }],
    openSelectedSessionInChat: () => {},
    selectWorkspace: () => {},
    selectSession: () => {},
    approvePermission: () => {},
  },
}

describe('RuntimeSectionPanelRenderer', () => {
  test('renders the overview tab content', async () => {
    const html = await renderVueSsr('/src/agena/pages/RuntimeSectionPanelRenderer.vue', {
      activeTab: 'overview',
      formatProviderModel: sharedPanels.overview.formatProviderModel,
      load: async () => {},
      panels: sharedPanels,
    })

    expect(html.includes('Runtime Snapshot')).toBe(true)
    expect(html.includes('Provider Defaults')).toBe(true)
    expect(html.includes('Model Catalog')).toBe(true)
    expect(html.includes('Refresh Catalog')).toBe(true)
    expect(html.includes('Workspace Root')).toBe(true)
  })

  test('renders the workflow tab content', async () => {
    const html = await renderVueSsr('/src/agena/pages/RuntimeSectionPanelRenderer.vue', {
      activeTab: 'workflow',
      formatProviderModel: sharedPanels.overview.formatProviderModel,
      load: async () => {},
      panels: sharedPanels,
    })

    expect(html.includes('Workflow Inspector')).toBe(true)
    expect(html.includes('Recent Timeline')).toBe(true)
    expect(html.includes('Global Event History')).toBe(true)
  })

  test('renders inspector tabs for mcp and lsp', async () => {
    const mcpHtml = await renderVueSsr('/src/agena/pages/RuntimeSectionPanelRenderer.vue', {
      activeTab: 'mcp',
      formatProviderModel: sharedPanels.overview.formatProviderModel,
      load: async () => {},
      panels: sharedPanels,
    })
    const lspHtml = await renderVueSsr('/src/agena/pages/RuntimeSectionPanelRenderer.vue', {
      activeTab: 'lsp',
      formatProviderModel: sharedPanels.overview.formatProviderModel,
      load: async () => {},
      panels: sharedPanels,
    })

    expect(mcpHtml.includes('MCP Servers')).toBe(true)
    expect(mcpHtml.includes('Open Config Root')).toBe(true)
    expect(lspHtml.includes('LSP Fleet')).toBe(true)
    expect(lspHtml.includes('Open Source Root')).toBe(true)
  })

  test('renders skills and operator fallback tabs', async () => {
    const skillsHtml = await renderVueSsr('/src/agena/pages/RuntimeSectionPanelRenderer.vue', {
      activeTab: 'skills',
      formatProviderModel: sharedPanels.overview.formatProviderModel,
      load: async () => {},
      panels: sharedPanels,
    })
    const operatorHtml = await renderVueSsr('/src/agena/pages/RuntimeSectionPanelRenderer.vue', {
      activeTab: 'operator',
      formatProviderModel: sharedPanels.overview.formatProviderModel,
      load: async () => {},
      panels: sharedPanels,
    })

    expect(skillsHtml.includes('Search Skills &amp; Commands')).toBe(true)
    expect(skillsHtml.includes('Use in Chat')).toBe(true)
    expect(operatorHtml.includes('Agents + Skills')).toBe(true)
    expect(operatorHtml.includes('Default Agent')).toBe(true)
    expect(operatorHtml.includes('Config Found')).toBe(true)
  })
})
