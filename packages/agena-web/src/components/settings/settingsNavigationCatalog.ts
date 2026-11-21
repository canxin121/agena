import type { SettingsSidebarNavigationNode, SettingsTab } from './sidebar/settingsSidebarNavigation'
import type { SettingsSubpageDefinition } from './workbench/settingsSectionNavigation'
import { settingsText as st } from '../../i18n/settingsText'
import { PERMISSION_ACCESS_SECTIONS, PERMISSION_ACCESS_VIEW } from './permissions/permissionNavigation'

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
      // The catalog is a master/detail browser: the list and the inspection pane
      // both scroll independently, so the page itself must not scroll.
      fillsViewport: true,
    },
  ],
  permissions: [
    {
      id: 'profiles',
      label: () => st('Permission profiles'),
      description: () => st('Create and edit reusable permission profiles.'),
      keywords: ['profiles', 'policy', 'plugin access', '权限配置', '权限组', '插件访问'],
    },
    {
      id: 'assignments',
      label: () => st('Permission assignments'),
      description: () => st('Assign permission profiles to conversations and their members.'),
      keywords: ['groups', 'users', 'conversations', 'telegram', '成员', '私聊', '群聊', '权限绑定'],
    },
    {
      id: PERMISSION_ACCESS_VIEW,
      aliases: [...PERMISSION_ACCESS_SECTIONS, 'overview'],
      label: () => st('Access permissions'),
      description: () => st('Configure file, network, and tool access together for each permission scope.'),
      keywords: [
        'filesystem',
        'file',
        'path',
        'read',
        'write',
        'network',
        'outbound',
        'inbound',
        'destination',
        'cidr',
        'protocol',
        'ports',
        'hostLoopback',
        'tools',
        'plugin access',
        'commands',
        'allow',
        'auto',
        'ask',
        'deny',
        'inherit',
        'rules',
        '文件',
        '网络',
        '工具',
        '插件访问',
        '审批',
        'sandbox',
        'mxc',
        'resource guard',
        'resource preflight',
        'overview',
        '沙盒',
        '资源预检',
        '总览',
      ],
    },
  ],
  'plugins-tools': [
    {
      id: 'plugin-workbench',
      label: () => st('Installed plugins'),
      description: () => st('Select a plugin to manage its settings, tools, commands, and activity.'),
      keywords: [
        'plugins',
        'installed',
        'schema',
        'config',
        'settings',
        'tools',
        'commands',
        'hooks',
        'hook',
        '钩子',
        'operations',
        'logs',
        'diagnostics',
      ],
      fillsViewport: true,
    },
    {
      id: 'marketplace',
      label: () => st('Plugin Marketplace'),
      description: () => st('Discover, verify, install, upgrade, and remove GitHub-hosted Agena plugins.'),
      keywords: ['marketplace', 'github', 'install', 'upgrade', 'release', 'plugins'],
    },
  ],
  skills: [
    {
      id: 'library',
      label: () => st('Skill library'),
      description: () => st('Manage reusable instructions, supporting files, and skill availability.'),
      keywords: ['skills', 'SKILL.md', 'instructions', 'files', 'create', 'edit', 'delete'],
      fillsViewport: true,
    },
    {
      id: 'install',
      label: () => st('Install skills'),
      description: () => st('Install skills from GitHub, archives, or local folders into Global or Workspace.'),
      keywords: ['skills', 'github', 'archive', 'zip', 'tar', 'folder', 'upload', 'install'],
    },
  ],
  'mcp-connections': [
    {
      id: 'connections',
      label: () => st('MCP connections'),
      description: () =>
        st('Connect external MCP services and manage their configuration, authentication, tools, and resources.'),
      keywords: ['mcp', 'client', 'external', 'servers', 'stdio', 'http', 'oauth', 'tools', 'resources', 'prompts'],
      fillsViewport: true,
    },
  ],
  'mcp-server': [
    {
      id: 'connection',
      label: () => st('MCP Server'),
      description: () =>
        st('Manage the connected server’s MCP listener, OAuth policy, public identity, and tool exposure.'),
      keywords: ['mcp', 'oauth', 'chatgpt', 'server url', 'address', 'connection', 'tools'],
    },
  ],
  'runtime-session': [
    {
      id: 'agent-profiles',
      aliases: ['prompt-layers', 'prompt-library'],
      label: () => st('Agent profiles'),
      description: () => st('Reuse permissions, system instructions, and model preferences across sessions.'),
      keywords: [
        'agent',
        'profile',
        'subagent',
        'permissions',
        'system prompt',
        'instructions',
        '提示词',
        '指令',
        'model',
        'configuration',
        '智能体',
        '配置档案',
        '子智能体',
      ],
    },
    {
      id: 'client-versions',
      label: () => st('Provider client versions'),
      description: () =>
        st('Pin or refresh the compatibility client versions presented by Codex, Claude, and Gemini adapters.'),
      keywords: ['client', 'version', 'codex', 'claude', 'gemini', 'npm'],
    },
    {
      id: 'session-behavior',
      aliases: ['compaction'],
      label: () => st('Session settings'),
      description: () => st('Configure default profiles and models, automatic titles, and context compaction.'),
      keywords: [
        'default profile',
        'default_profile',
        'default_subagent_profile',
        'default model',
        'approval model',
        'auxiliary model',
        'model configuration',
        'response preferences',
        'thinking',
        'speed',
        'verbosity',
        'automatic naming',
        'session',
        'session behavior',
        'compaction',
        'context',
        'tokens',
        'reserve',
        '会话行为',
        '默认档案',
        '默认智能体',
        '默认 Profile',
        '默认Profile',
        '子 Agent',
        '默认模型',
        '审批模型',
        '辅助模型',
        '模型设置',
        '自动命名',
        '上下文',
        '压缩',
      ],
    },
  ],
  interface: [
    {
      id: 'tui',
      label: () => st('TUI preferences'),
      description: () => st('Server-backed language, color, graphics, and plugin theme settings for TUI clients.'),
      keywords: ['tui', 'locale', 'color', 'graphics', 'theme', 'transcript'],
    },
    {
      id: 'web-appearance',
      aliases: ['conversation', 'notifications'],
      label: () => st('Web preferences'),
      description: () => st('Configure Web appearance, conversation display, and browser notifications.'),
      keywords: [
        'theme',
        'font',
        'language',
        'density',
        'chat',
        'conversation',
        'reasoning',
        'timestamps',
        'push',
        'notifications',
        '对话',
        '通知',
      ],
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
  permissions: PERMISSION_ACCESS_VIEW,
  'plugins-tools': 'plugin-workbench',
  skills: 'library',
  'mcp-connections': 'connections',
  'mcp-server': 'connection',
  'runtime-session': 'session-behavior',
  interface: 'web-appearance',
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

/** The settings sidebar has two levels; scope selection lives in the page workbench. */
export function buildSettingsSidebarNodes(section: SettingsTab): SettingsSidebarNavigationNode[] {
  return buildSettingsSubpages(section).map((page) => {
    return {
      id: page.id,
      label: page.label,
      description: page.description,
      keywords: page.keywords,
      view: page.id,
    }
  })
}
