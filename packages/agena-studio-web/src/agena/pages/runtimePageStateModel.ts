import type {
  MarketplaceInstalledPluginResource,
  MarketplacePluginResource,
  RuntimeSkill,
  SessionResource,
  WorkspaceResource,
} from '@/agena/lib/agenaApi'
import type { RouteLocationNormalizedLoaded } from 'vue-router'

export type RuntimeRouteSection = 'runtime' | 'plugins' | 'settings'
export type RuntimeTab = 'overview' | 'workflow' | 'mcp' | 'lsp' | 'skills' | 'operator'
export type SettingsTab = 'providers' | 'agents' | 'plugins' | 'permissions' | 'desktop'
export type PluginsTab = 'installed' | 'marketplace'
export type SectionTabOption<TTab extends string = string> = { id: TTab; label: string }

export const runtimeTabs: SectionTabOption<RuntimeTab>[] = [
  { id: 'overview', label: 'Overview' },
  { id: 'workflow', label: 'Workflow' },
  { id: 'mcp', label: 'MCP' },
  { id: 'lsp', label: 'LSP' },
  { id: 'skills', label: 'Skills' },
  { id: 'operator', label: 'Operator' },
]

export const settingsTabs: SectionTabOption<SettingsTab>[] = [
  { id: 'providers', label: 'Providers' },
  { id: 'agents', label: 'Agents' },
  { id: 'plugins', label: 'Plugins' },
  { id: 'permissions', label: 'Guardrails' },
  { id: 'desktop', label: 'Desktop' },
]

export const pluginsTabs: SectionTabOption<PluginsTab>[] = [
  { id: 'installed', label: 'Installed' },
  { id: 'marketplace', label: 'Marketplace' },
]

export const sectionTitles: Record<RuntimeRouteSection, string> = {
  runtime: 'Runtime',
  plugins: 'Plugins',
  settings: 'Settings',
}

export const sectionDescriptions: Record<RuntimeRouteSection, string> = {
  runtime: 'Inspect runtime state, workflows, MCP, LSP, skills, and operator snapshots.',
  plugins: 'Inspect installed plugins, marketplace readiness, manifests, and retained logs.',
  settings: 'Configure Agena providers, agents, plugins, runtime guardrails, and desktop services.',
}

export const sectionPagePaths: Record<RuntimeRouteSection, string> = {
  runtime: './RuntimePage.vue',
  plugins: './PluginsPage.vue',
  settings: './SettingsPage.vue',
}

export const sectionPageLoaders = {
  runtime: () => import('./RuntimePage.vue'),
  plugins: () => import('./PluginsPage.vue'),
  settings: () => import('./SettingsPage.vue'),
} satisfies Record<RuntimeRouteSection, () => Promise<unknown>>

export const sectionBasePaths: Record<RuntimeRouteSection, `/${RuntimeRouteSection}`> = {
  runtime: '/runtime',
  plugins: '/plugins',
  settings: '/settings',
}

export const sectionNavItems: Array<{ section: RuntimeRouteSection; label: string }> = [
  { section: 'runtime', label: 'Runtime' },
  { section: 'plugins', label: 'Plugins' },
  { section: 'settings', label: 'Settings' },
]

export type SectionTabNavigationItem = {
  id: string
  title: string
  description: string
  section: RuntimeRouteSection
  tab: RuntimeTab | SettingsTab | PluginsTab
  slash: string
  aliases: string[]
  shortcutSlash?: string
}

export const sectionTabNavigationItems: SectionTabNavigationItem[] = [
  {
    id: 'nav.runtime.overview',
    title: 'Open Runtime Overview',
    description: 'Inspect runtime snapshot, providers, models, and session cache facts.',
    section: 'runtime',
    tab: 'overview',
    slash: '/runtime-overview',
    aliases: ['runtime overview', 'providers', 'models'],
  },
  {
    id: 'nav.runtime.workflow',
    title: 'Open Runtime Workflow',
    description: 'Inspect workspaces, sessions, execution state, and timeline summaries.',
    section: 'runtime',
    tab: 'workflow',
    slash: '/runtime-workflow',
    aliases: ['workflow inspector', 'execution timeline', 'sessions'],
    shortcutSlash: '/workflow',
  },
  {
    id: 'nav.runtime.mcp',
    title: 'Open Runtime MCP',
    description: 'Inspect MCP servers, config roots, and workspace shortcuts.',
    section: 'runtime',
    tab: 'mcp',
    slash: '/runtime-mcp',
    aliases: ['mcp servers', 'tools', 'server config'],
    shortcutSlash: '/mcp',
  },
  {
    id: 'nav.runtime.lsp',
    title: 'Open Runtime LSP',
    description: 'Inspect LSP servers, roots, and workspace shortcuts.',
    section: 'runtime',
    tab: 'lsp',
    slash: '/runtime-lsp',
    aliases: ['language server', 'diagnostics', 'lsp servers'],
    shortcutSlash: '/lsp',
  },
  {
    id: 'nav.runtime.skills',
    title: 'Open Runtime Skills',
    description: 'Inspect discovered skills, commands, and runtime entry sources.',
    section: 'runtime',
    tab: 'skills',
    slash: '/runtime-skills',
    aliases: ['skills', 'commands', 'slash commands'],
    shortcutSlash: '/skills',
  },
  {
    id: 'nav.runtime.operator',
    title: 'Open Runtime Operator',
    description: 'Inspect the raw operator runtime payload and runtime capabilities.',
    section: 'runtime',
    tab: 'operator',
    slash: '/runtime-operator',
    aliases: ['operator', 'raw runtime', 'runtime payload'],
  },
  {
    id: 'nav.plugins.installed',
    title: 'Open Installed Plugins',
    description: 'Inspect installed plugins, selected plugin detail, manifests, and retained logs.',
    section: 'plugins',
    tab: 'installed',
    slash: '/plugins-installed',
    aliases: ['installed plugins', 'plugin logs', 'plugin manifest'],
  },
  {
    id: 'nav.plugins.marketplace',
    title: 'Open Plugin Marketplace',
    description: 'Browse marketplace plugins, registry settings, and install actions.',
    section: 'plugins',
    tab: 'marketplace',
    slash: '/plugins-marketplace',
    aliases: ['marketplace', 'plugin registry', 'install plugin'],
    shortcutSlash: '/marketplace',
  },
  {
    id: 'nav.settings.providers',
    title: 'Open Provider Settings',
    description: 'Manage provider credentials, API keys, and auth provider state.',
    section: 'settings',
    tab: 'providers',
    slash: '/settings-providers',
    aliases: ['providers', 'credentials', 'api keys'],
    shortcutSlash: '/providers',
  },
  {
    id: 'nav.settings.agents',
    title: 'Open Agent Settings',
    description: 'Inspect runtime agent profiles and default provider/model settings.',
    section: 'settings',
    tab: 'agents',
    slash: '/settings-agents',
    aliases: ['agents', 'agent profiles', 'default agent'],
    shortcutSlash: '/agents',
  },
  {
    id: 'nav.settings.plugins',
    title: 'Open Plugin Settings',
    description: 'Manage grouped per-plugin and per-tool prompt policies plus web/TUI display modes.',
    section: 'settings',
    tab: 'plugins',
    slash: '/settings-plugins',
    aliases: ['plugins', 'tool descriptions', 'brief mode', 'display mode'],
    shortcutSlash: '/plugins-settings',
  },
  {
    id: 'nav.settings.permissions',
    title: 'Open Permission Settings',
    description: 'Manage permission rules, filters, drafts, and revoke actions.',
    section: 'settings',
    tab: 'permissions',
    slash: '/settings-permissions',
    aliases: ['permissions', 'permission rules', 'allow deny'],
    shortcutSlash: '/permissions',
  },
  {
    id: 'nav.settings.desktop',
    title: 'Open Desktop Settings',
    description: 'Manage desktop backend status, config, updates, and restart actions.',
    section: 'settings',
    tab: 'desktop',
    slash: '/settings-desktop',
    aliases: ['desktop', 'desktop backend', 'desktop config'],
    shortcutSlash: '/desktop',
  },
]

export function isRuntimeTab(value: string): value is RuntimeTab {
  return runtimeTabs.some((tab) => tab.id === value)
}

export function isSettingsTab(value: string): value is SettingsTab {
  return settingsTabs.some((tab) => tab.id === value)
}

export function isPluginsTab(value: string): value is PluginsTab {
  return pluginsTabs.some((tab) => tab.id === value)
}

export function pickWorkspaceId(currentWorkspaceId: number | null, items: WorkspaceResource[]): number | null {
  if (currentWorkspaceId && items.some((workspace) => workspace.id === currentWorkspaceId)) {
    return currentWorkspaceId
  }
  return items[0]?.id ?? null
}

export function pickSessionId(currentSessionId: number | null, items: SessionResource[]): number | null {
  if (currentSessionId && items.some((session) => session.id === currentSessionId)) {
    return currentSessionId
  }
  return items[0]?.id ?? null
}

export function normalizeOptionalText(value: string): string | null {
  const normalized = String(value || '').trim()
  return normalized || null
}

export function normalizePort(value: string, fallback: number): number {
  const parsed = Number(value)
  if (!Number.isFinite(parsed) || parsed < 0) return fallback
  return Math.floor(parsed)
}

export function resolveRuntimeRouteSection(path: string, section?: RuntimeRouteSection): RuntimeRouteSection {
  if (section) return section
  for (const [candidate, basePath] of Object.entries(sectionBasePaths) as Array<[RuntimeRouteSection, string]>) {
    if (path.startsWith(basePath)) return candidate
  }
  return 'runtime'
}

export function defaultTabForSection(section: RuntimeRouteSection): RuntimeTab | SettingsTab | PluginsTab {
  if (section === 'settings') return 'providers'
  if (section === 'plugins') return 'installed'
  return 'overview'
}

function readLegacySectionTab(
  section: RuntimeRouteSection,
  query: RouteLocationNormalizedLoaded['query'],
): string | null {
  const legacyKey = section === 'settings' ? 'settingsTab' : section === 'plugins' ? 'pluginsTab' : 'tab'
  const value = query[legacyKey]
  if (Array.isArray(value)) {
    return normalizeOptionalText(value[0] || '')
  }
  return normalizeOptionalText(String(value || ''))
}

export function sanitizeRuntimeSectionQuery(query: RouteLocationNormalizedLoaded['query']) {
  const nextQuery = { ...query }
  delete nextQuery.tab
  delete nextQuery.settingsTab
  delete nextQuery.pluginsTab
  return nextQuery
}

export function resolveRuntimeTabFromPath(path: string): string {
  const segments = path.split('/').filter(Boolean)
  const section = resolveRuntimeRouteSection(path)
  return segments[1] || defaultTabForSection(section)
}

export function resolveRuntimeTabFromRoute(
  path: string,
  query: RouteLocationNormalizedLoaded['query'],
  section?: RuntimeRouteSection,
): string {
  const routeSection = resolveRuntimeRouteSection(path, section)
  const legacyTab = readLegacySectionTab(routeSection, query)
  if (legacyTab) {
    return legacyTab
  }
  return resolveRuntimeTabFromPath(path)
}

export function buildRuntimeSectionPath(section: RuntimeRouteSection, tab: string): string {
  const normalizedTab = String(tab || '').trim() || defaultTabForSection(section)
  return `${sectionBasePaths[section]}/${normalizedTab}`
}

export function filterMarketplacePluginsByQuery(
  plugins: MarketplacePluginResource[],
  installed: MarketplaceInstalledPluginResource[],
  query: string,
): MarketplacePluginResource[] {
  const normalizedQuery = query.trim().toLowerCase()
  if (!normalizedQuery) return plugins
  const installedIds = new Set(installed.map((item) => item.plugin_id))
  return plugins.filter((plugin) => {
    const haystack = [
      plugin.plugin_id,
      plugin.name,
      plugin.description,
      plugin.latest_kind || '',
      plugin.latest_platform || '',
      plugin.latest_version || '',
      installedIds.has(plugin.plugin_id) ? 'installed' : '',
    ]
      .join(' ')
      .toLowerCase()
    return haystack.includes(normalizedQuery)
  })
}

export function queryMatchesText(parts: Array<string | null | undefined>, query: string): boolean {
  const normalizedQuery = query.trim().toLowerCase()
  if (!normalizedQuery) return true
  return parts
    .map((part) =>
      String(part || '')
        .trim()
        .toLowerCase(),
    )
    .join(' ')
    .includes(normalizedQuery)
}

export function filterRuntimeSkillsByQuery(entries: RuntimeSkill[], query: string): RuntimeSkill[] {
  return entries.filter((entry) =>
    queryMatchesText([entry.name, entry.description, ...(entry.aliases || []), entry.source_path || ''], query),
  )
}
