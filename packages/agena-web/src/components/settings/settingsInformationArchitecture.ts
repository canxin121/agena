/**
 * User-facing navigation follows tasks, not backend module boundaries.
 * Keep the nine legacy settings sections intact so bookmarks, persisted
 * subpages, scope selectors and API writes remain compatible.
 */
import { settingsText as st } from '../../i18n/settingsText'
import { buildSettingsSubpages } from './settingsNavigationCatalog'
import type {
  SettingsCategoryId,
  SettingsSidebarIconKey,
  SettingsSidebarTab,
  SettingsTab,
} from './sidebar/settingsSidebarNavigation'

type SettingsTask = {
  id: string
  section: SettingsTab
  view: string
  relatedViews?: readonly string[]
  label: () => string
  keywords?: readonly string[]
}

type SettingsCategory = {
  id: SettingsCategoryId
  label: () => string
  icon: SettingsSidebarIconKey
  keywords?: readonly string[]
  tasks: readonly SettingsTask[]
}

const SETTINGS_CATEGORIES: readonly SettingsCategory[] = [
  {
    id: 'ai-session',
    label: () => st('AI & sessions'),
    icon: 'models-providers',
    keywords: ['AI', 'model', 'prompt', 'session', '模型', '会话'],
    tasks: [
      {
        id: 'session-behavior',
        section: 'runtime-session',
        view: 'session-behavior',
        label: () => st('Session settings'),
      },
      {
        id: 'provider-studio',
        section: 'models-providers',
        view: 'provider-studio',
        label: () => st('Provider Studio'),
        keywords: ['服务商', '认证', '适配器'],
      },
      {
        id: 'agent-profiles',
        section: 'runtime-session',
        view: 'agent-profiles',
        label: () => st('Agent profiles'),
        keywords: ['profile', 'agent', 'permissions', 'system prompt', '智能体', '配置档案'],
      },
      {
        id: 'model-catalog',
        section: 'models-providers',
        view: 'model-catalog',
        label: () => st('Model Catalog'),
        keywords: ['模型目录', '定价', '价格', '能力'],
      },
    ],
  },
  {
    id: 'extensions',
    label: () => st('Extensions & connections'),
    icon: 'plugins-tools',
    keywords: ['plugin', 'skill', 'MCP', '扩展', '连接'],
    tasks: [
      {
        id: 'plugins',
        section: 'plugins-tools',
        view: 'plugin-workbench',
        relatedViews: ['plugin-workbench', 'marketplace'],
        label: () => st('Plugins'),
        keywords: ['plugin marketplace', 'install', 'commands', 'hooks', 'hook', '插件', '市场', '钩子'],
      },
      {
        id: 'skills',
        section: 'skills',
        view: 'library',
        relatedViews: ['library', 'install'],
        label: () => st('Skills'),
        keywords: ['SKILL.md', 'install skills', '技能', '安装'],
      },
      {
        id: 'mcp-client',
        section: 'mcp-connections',
        view: 'connections',
        label: () => st('MCP connections'),
        keywords: ['external MCP', 'client', 'connect', '外部', '接入'],
      },
      {
        id: 'mcp-host',
        section: 'mcp-server',
        view: 'connection',
        label: () => st('MCP service'),
        keywords: ['MCP server', 'OAuth', 'ChatGPT', '对外', '服务'],
      },
    ],
  },
  {
    id: 'security',
    label: () => st('Permissions & security'),
    icon: 'permissions',
    keywords: ['security', 'allow', 'deny', '权限', '安全'],
    tasks: [
      {
        id: 'profiles',
        section: 'permissions',
        view: 'profiles',
        label: () => st('Permission profiles'),
        keywords: ['profile', 'policy', 'plugin access', '权限配置', '权限组', '插件访问'],
      },
      {
        id: 'assignments',
        section: 'permissions',
        view: 'assignments',
        label: () => st('Permission assignments'),
        keywords: ['group', 'user', 'conversation', 'telegram', '成员', '私聊', '群聊', '权限绑定'],
      },
      {
        id: 'access',
        section: 'permissions',
        view: 'access',
        label: () => st('Access permissions'),
        keywords: [
          'read',
          'write',
          'path',
          'network',
          'internet',
          'private network',
          'host',
          'tools',
          'plugin access',
          'allow',
          'ask',
          'deny',
          'rules',
          '文件',
          '插件访问',
          '路径',
          '网络',
          '公网',
          '工具',
          '审批',
        ],
      },
    ],
  },
  {
    id: 'appearance',
    label: () => st('Interface & experience'),
    icon: 'interface',
    keywords: ['appearance', 'theme', 'chat', '界面', '体验'],
    tasks: [
      {
        id: 'web',
        section: 'interface',
        view: 'web-appearance',
        label: () => st('Web preferences'),
        keywords: ['font', 'theme', 'size', 'language', 'conversation', 'push', '字体', '主题', '语言', '对话', '通知'],
      },
      {
        id: 'tui',
        section: 'interface',
        view: 'tui',
        label: () => st('TUI preferences'),
        keywords: ['terminal', 'graphics', 'color', '终端'],
      },
    ],
  },
  {
    id: 'operations',
    label: () => st('Operations & maintenance'),
    icon: 'diagnostics',
    keywords: ['runtime', 'compatibility', 'advanced', '运行', '维护'],
    tasks: [
      { id: 'tracing', section: 'diagnostics', view: 'runtime', label: () => st('Runtime & tracing') },
      {
        id: 'clients',
        section: 'runtime-session',
        view: 'client-versions',
        label: () => st('Client compatibility'),
        keywords: ['Codex', 'Claude', 'Gemini', 'version', '适配器', '版本'],
      },
      {
        id: 'advanced',
        section: 'diagnostics',
        view: 'advanced-settings',
        label: () => st('Advanced settings'),
        keywords: ['JSON', 'global', 'workspace', '高级配置'],
      },
    ],
  },
  {
    id: 'records',
    label: () => st('Data & records'),
    icon: 'runtime-session',
    keywords: ['history', 'usage', 'memory', 'records', '数据', '记录'],
    tasks: [
      { id: 'activity', section: 'diagnostics', view: 'activities', label: () => st('Activity history') },
      {
        id: 'usage',
        section: 'diagnostics',
        view: 'usage',
        label: () => st('Usage & costs'),
        keywords: ['token', 'cost', 'billing', '用量', '费用'],
      },
      {
        id: 'memory',
        section: 'diagnostics',
        view: 'memories',
        label: () => st('Memories & indexing'),
        keywords: ['index', 'documents', '记忆', '索引'],
      },
    ],
  },
]

/** One task row per intent, including search terms from every contained legacy page. */
export function buildSettingsNavigationCategories(): SettingsSidebarTab[] {
  const pages = new Map(
    SETTINGS_CATEGORIES.flatMap((category) => category.tasks.map((task) => task.section)).map((section) => [
      section,
      buildSettingsSubpages(section),
    ]),
  )
  return SETTINGS_CATEGORIES.map((category) => ({
    id: category.id,
    label: category.label(),
    icon: category.icon,
    group: 'all',
    keywords: [...(category.keywords || [])],
    children: category.tasks.map((task) => {
      const relatedViews = [...(task.relatedViews || [task.view])]
      const related = (pages.get(task.section) || []).filter((page) => relatedViews.includes(page.id))
      return {
        id: task.id,
        section: task.section,
        view: task.view,
        relatedViews,
        searchTargets: related.map((page) => ({
          view: page.id,
          label: page.label,
          keywords: [page.description, ...(page.keywords || []), ...(page.aliases || [])],
        })),
        label: task.label(),
        description: related.map((page) => page.description).join(' '),
        keywords: [
          ...(task.keywords || []),
          ...related.flatMap((page) => [
            page.label,
            page.description,
            ...(page.aliases || []),
            ...(page.keywords || []),
          ]),
        ],
      }
    }),
  }))
}
