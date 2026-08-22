import type { SettingsTab } from './sidebar/settingsSidebarNavigation'
import type { SettingsSubpageDefinition } from './workbench/settingsSectionNavigation'
import { settingsText as st } from '../../i18n/settingsText'

type SettingsSubpageSource = Omit<SettingsSubpageDefinition, 'label' | 'description'> & {
  label: () => string
  description: () => string
}

const SETTINGS_SUBPAGE_SOURCES: Record<SettingsTab, readonly SettingsSubpageSource[]> = {
  'models-providers': [
    {
      id: 'provider-studio',
      label: () => st('Provider Studio'),
      description: () => st('Create providers, configure authentication, adapters, and model routes.'),
      keywords: ['provider', 'authentication', 'oauth', 'api key', 'adapter', 'model'],
    },
    {
      id: 'model-catalog',
      label: () => st('Model Catalog'),
      description: () => st('Search the resolved model catalog and inspect capabilities, limits, modes, and pricing.'),
      keywords: ['catalog', 'metadata', 'pricing', 'capabilities', 'context window'],
    },
    {
      id: 'defaults',
      label: () => st('Model defaults'),
      description: () => st('Choose the one runtime-wide default model and its optional execution modes.'),
      keywords: ['default', 'model', 'thinking', 'speed', 'verbosity'],
    },
    {
      id: 'inventory',
      label: () => st('Configured inventory'),
      description: () => st('Review every configured provider, adapter, endpoint, and model.'),
      keywords: ['inventory', 'configured', 'provider list', 'adapter list'],
    },
  ],
  permissions: [
    {
      id: 'policy-studio',
      label: () => st('Permission Studio'),
      description: () => st('Edit global, workspace, current-session, and effective permission policy layers.'),
      keywords: ['filesystem', 'network', 'tools', 'allow', 'auto', 'ask', 'deny'],
    },
    {
      id: 'persistent-rules',
      label: () => st('Persistent rules'),
      description: () =>
        st('Inspect and revoke durable approval rules captured from interactive permission decisions.'),
      keywords: ['rules', 'approval', 'revoke', 'history'],
    },
  ],
  'plugins-tools': [
    {
      id: 'marketplace',
      label: () => st('Plugin Marketplace'),
      description: () => st('Discover, verify, install, upgrade, and remove GitHub-hosted Agena plugins.'),
      keywords: ['marketplace', 'github', 'install', 'upgrade', 'release', 'plugins'],
    },
    {
      id: 'plugin-workbench',
      label: () => st('Plugin Workbench'),
      description: () =>
        st('Configure plugins, run tools and operations, and inspect capabilities, logs, and diagnostics.'),
      keywords: ['plugins', 'schema', 'config', 'tools', 'operations', 'logs'],
    },
    {
      id: 'mcp-server',
      label: () => st('MCP Server'),
      description: () =>
        st('Manage the connected server’s MCP listener, OAuth policy, public identity, and tool exposure.'),
      keywords: ['mcp', 'oauth', 'chatgpt', 'public url', 'tools'],
    },
    {
      id: 'harnesses',
      label: () => st('Tool harnesses'),
      description: () => st('Create named browser, shell, and editor harness configurations.'),
      keywords: ['browser', 'shell', 'editor', 'environment', 'commands'],
    },
  ],
  'runtime-session': [
    {
      id: 'client-versions',
      label: () => st('Provider client versions'),
      description: () =>
        st('Pin or refresh the compatibility client versions presented by Codex, Claude, and Gemini adapters.'),
      keywords: ['client', 'version', 'codex', 'claude', 'gemini', 'npm'],
    },
    {
      id: 'compaction',
      label: () => st('Session compaction'),
      description: () => st('Control automatic compaction and the token reserve used when deciding when to compact.'),
      keywords: ['session', 'compaction', 'context', 'tokens', 'reserve'],
    },
  ],
  interface: [
    {
      id: 'tui',
      label: () => st('TUI preferences'),
      description: () =>
        st(
          'Server-backed language, color, graphics, plugin theme, and transcript expansion defaults shared with the TUI.',
        ),
      keywords: ['tui', 'locale', 'color', 'graphics', 'theme', 'transcript'],
    },
    {
      id: 'web-appearance',
      label: () => st('Web appearance'),
      description: () => st('Browser-only theme, fonts, density, spacing, and geometry preferences.'),
      keywords: ['web', 'appearance', 'font', 'padding', 'radius', 'language'],
    },
    {
      id: 'conversation',
      label: () => st('Conversation display'),
      description: () =>
        st('Web transcript timestamps, reasoning visibility, activity expansion, and exact tool overrides.'),
      keywords: ['chat', 'conversation', 'reasoning', 'timestamps', 'activity', 'tools'],
    },
  ],
  diagnostics: [
    {
      id: 'runtime',
      label: () => st('Runtime & tracing'),
      description: () =>
        st('Configure tracing, inspect the runtime snapshot, validate settings, and reload the runtime.'),
      keywords: ['tracing', 'logs', 'runtime', 'reload', 'validate'],
    },
    {
      id: 'advanced-settings',
      label: () => st('Advanced settings'),
      description: () =>
        st('Edit any explicit Global or Workspace JSON path with dry-run validation and source comparison.'),
      keywords: ['advanced', 'configuration', 'json path', 'global', 'workspace', 'override'],
    },
    {
      id: 'activities',
      label: () => st('Activity history'),
      description: () => st('Inspect durable operational activity records and their current states.'),
      keywords: ['activities', 'tasks', 'operations', 'history'],
    },
    {
      id: 'memories',
      label: () => st('Memories'),
      description: () => st('Inspect memory records and indexing state.'),
      keywords: ['memory', 'index', 'documents'],
    },
    {
      id: 'usage',
      label: () => st('Usage'),
      description: () => st('Review recorded usage and cost information.'),
      keywords: ['usage', 'tokens', 'cost'],
    },
  ],
}

export const SETTINGS_DEFAULT_SUBPAGE: Record<SettingsTab, string> = {
  'models-providers': 'provider-studio',
  permissions: 'policy-studio',
  'plugins-tools': 'plugin-workbench',
  'runtime-session': 'client-versions',
  interface: 'tui',
  diagnostics: 'runtime',
}

export function buildSettingsSubpages(section: SettingsTab): SettingsSubpageDefinition[] {
  return SETTINGS_SUBPAGE_SOURCES[section].map((page) => ({
    ...page,
    label: page.label(),
    description: page.description(),
    keywords: page.keywords ? [...page.keywords] : undefined,
  }))
}
