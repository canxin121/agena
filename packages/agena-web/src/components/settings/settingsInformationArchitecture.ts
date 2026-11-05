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
        id: 'models',
        section: 'models-providers',
        view: 'model-settings',
        relatedViews: ['provider-studio', 'model-catalog', 'model-settings'],
        label: () => st('Model configuration'),
        keywords: ['provider', 'API key', 'oauth', 'default model', '模型', '服务商'],
      },
      {
        id: 'instructions',
        section: 'runtime-session',
        view: 'prompt-library',
        relatedViews: ['prompt-library', 'prompt-layers'],
        label: () => st('Instructions & prompts'),
        keywords: ['system prompt', 'global', 'workspace', 'session', '提示词', '指令'],
      },
      {
        id: 'context',
        section: 'runtime-session',
        view: 'compaction',
        label: () => st('Context management'),
        keywords: ['compaction', 'reserve tokens', '上下文', '压缩'],
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
        keywords: ['plugin marketplace', 'install', 'commands', '插件', '市场'],
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
        id: 'overview',
        section: 'permissions',
        view: 'overview',
        label: () => st('Permission overview'),
        keywords: ['effective', 'summary', 'merged permissions', '总览'],
      },
      {
        id: 'files',
        section: 'permissions',
        view: 'filesystem',
        label: () => st('File access'),
        keywords: ['read', 'write', 'path', '文件', '路径'],
      },
      {
        id: 'network',
        section: 'permissions',
        view: 'network',
        label: () => st('Network access'),
        keywords: ['internet', 'private network', 'host', '网络', '公网'],
      },
      {
        id: 'execution',
        section: 'permissions',
        view: 'tools',
        label: () => st('Tool execution & approvals'),
        keywords: ['allow', 'ask', 'deny', 'rules', '工具', '审批'],
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
        label: () => st('Web appearance & language'),
        keywords: ['font', 'theme', 'size', 'language', '字体', '主题', '语言'],
      },
      {
        id: 'conversation',
        section: 'interface',
        view: 'conversation',
        label: () => st('Conversation display'),
        keywords: ['reasoning', 'timestamps', 'activity', '对话', '推理'],
      },
      {
        id: 'notifications',
        section: 'interface',
        view: 'notifications',
        label: () => st('Browser notifications'),
        keywords: ['push', 'permission', '提醒', '通知'],
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
