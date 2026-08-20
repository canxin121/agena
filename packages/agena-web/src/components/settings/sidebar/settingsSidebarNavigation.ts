/**
 * The web settings workbench follows the same top-level sections as the TUI
 * Settings Studio.  The legacy names remain accepted by the route parser so
 * links from older clients do not become dead links.
 */

export const SETTINGS_TAB_IDS = [
  'models-providers',
  'permissions',
  'plugins-tools',
  'runtime-session',
  'interface',
  'diagnostics',
] as const

export type SettingsTab = (typeof SETTINGS_TAB_IDS)[number]
export type LegacySettingsTab =
  | 'general'
  | 'providers'
  | 'permissions'
  | 'plugins'
  | 'activities'
  | 'memories'
  | 'usage'
export type SettingsRouteTab = SettingsTab | LegacySettingsTab
export type SettingsSidebarGroupId = 'core' | 'application' | 'system'
export type SettingsSidebarIconKey =
  | 'models-providers'
  | 'permissions'
  | 'plugins-tools'
  | 'runtime-session'
  | 'interface'
  | 'diagnostics'

export type SettingsSidebarNavigationNode = {
  id: string
  label: string
  description?: string
  keywords?: string[]
  view?: string
  children?: SettingsSidebarNavigationNode[]
}

export type SettingsSidebarTab = {
  id: SettingsTab
  label: string
  icon: SettingsSidebarIconKey
  group: SettingsSidebarGroupId
  keywords?: string[]
  children?: SettingsSidebarNavigationNode[]
}

export type SettingsSidebarDestination = {
  section: SettingsTab
  view: string
}

export type SettingsSidebarTabRow = {
  kind: 'item'
  key: string
  section: SettingsTab
  view?: string
  label: string
  icon?: SettingsSidebarIconKey
  depth: number
  active: boolean
  branchActive: boolean
  hasChildren: boolean
  expanded: boolean
}

export type SettingsSidebarRenderGroup = {
  id: SettingsSidebarGroupId
  labelKey: string
  items: SettingsSidebarTabRow[]
}

const SETTINGS_TAB_CONFIG: Array<{
  id: SettingsTab
  labelKey: string
  icon: SettingsSidebarIconKey
  group: SettingsSidebarGroupId
  keywords?: string[]
}> = [
  {
    id: 'models-providers',
    labelKey: 'settings.tabs.modelsProviders',
    icon: 'models-providers',
    group: 'core',
    keywords: ['model', 'models', 'provider', 'providers', 'adapter', 'llm', 'catalog', 'auth'],
  },
  {
    id: 'permissions',
    labelKey: 'settings.tabs.permissions',
    icon: 'permissions',
    group: 'core',
    keywords: ['permission', 'rules', 'allow', 'auto', 'ask', 'deny', 'filesystem', 'network', 'tool'],
  },
  {
    id: 'plugins-tools',
    labelKey: 'settings.tabs.pluginsTools',
    icon: 'plugins-tools',
    group: 'core',
    keywords: ['plugin', 'plugins', 'tools', 'commands', 'harness', 'browser', 'shell', 'editor'],
  },
  {
    id: 'runtime-session',
    labelKey: 'settings.tabs.runtimeSession',
    icon: 'runtime-session',
    group: 'application',
    keywords: ['runtime', 'session', 'compaction', 'context', 'client', 'version'],
  },
  {
    id: 'interface',
    labelKey: 'settings.tabs.interface',
    icon: 'interface',
    group: 'application',
    keywords: ['appearance', 'interface', 'language', 'theme', 'graphics', 'transcript', 'activity'],
  },
  {
    id: 'diagnostics',
    labelKey: 'settings.tabs.diagnostics',
    icon: 'diagnostics',
    group: 'system',
    keywords: ['diagnostics', 'debug', 'tracing', 'config', 'runtime status', 'logs', 'usage', 'memory', 'activity'],
  },
]

const GROUP_LABEL_KEYS: Record<SettingsSidebarGroupId, string> = {
  core: 'settings.groups.core',
  application: 'settings.groups.application',
  system: 'settings.groups.system',
}

const LEGACY_ROUTE_ALIASES: Record<string, LegacySettingsTab> = {
  general: 'general',
  appearance: 'general',
  provider: 'providers',
  providers: 'providers',
  model: 'providers',
  models: 'providers',
  permission: 'permissions',
  permissions: 'permissions',
  plugin: 'plugins',
  plugins: 'plugins',
  command: 'plugins',
  commands: 'plugins',
  skill: 'plugins',
  skills: 'plugins',
  activity: 'activities',
  activities: 'activities',
  task: 'activities',
  tasks: 'activities',
  memory: 'memories',
  memories: 'memories',
  usage: 'usage',
}

const LEGACY_TO_CANONICAL: Record<LegacySettingsTab, SettingsTab> = {
  general: 'interface',
  providers: 'models-providers',
  permissions: 'permissions',
  plugins: 'plugins-tools',
  activities: 'diagnostics',
  memories: 'diagnostics',
  usage: 'diagnostics',
}

export function isSettingsTab(input: string): input is SettingsTab {
  return SETTINGS_TAB_IDS.includes(input as SettingsTab)
}

export function isLegacySettingsTab(input: string): input is LegacySettingsTab {
  return Object.prototype.hasOwnProperty.call(LEGACY_TO_CANONICAL, input)
}

export function canonicalSettingsTab(input: SettingsRouteTab | null | undefined): SettingsTab {
  if (input && isSettingsTab(input)) return input
  if (input && isLegacySettingsTab(input)) return LEGACY_TO_CANONICAL[input]
  return 'interface'
}

/** Return the canonical URL used by the new workbench. */
export function settingsPathForTab(tab: SettingsRouteTab): string {
  if (isLegacySettingsTab(tab)) {
    // Keep the most common old URL stable for bookmarks. The SettingsPage
    // still renders the canonical Interface section for it.
    if (tab === 'general') return '/settings/general'
    return settingsPathForTab(LEGACY_TO_CANONICAL[tab])
  }
  return `/settings/${tab}`
}

/**
 * Parse both canonical section ids and legacy route values. Returning the
 * legacy alias is intentional: existing contract tests and old callers can
 * still identify what they linked to; callers rendering the workbench should
 * pass the result through canonicalSettingsTab().
 */
export function settingsTabFromRouteValue(value: unknown): SettingsRouteTab | null {
  const raw = String(value || '')
    .trim()
    .toLowerCase()
  if (!raw) return null
  if (isSettingsTab(raw)) return raw
  if (isLegacySettingsTab(raw)) return raw
  if (LEGACY_ROUTE_ALIASES[raw]) return LEGACY_ROUTE_ALIASES[raw]

  const path = raw.split(/[?#]/, 1)[0] || ''
  const parts = path.split('/').filter(Boolean)
  if (parts[0] !== 'settings') return null

  if (parts[1] === 'opencode') {
    const legacySection = parts[2] || ''
    return LEGACY_ROUTE_ALIASES[legacySection] || 'general'
  }
  if (parts[1] === 'plan') return 'plugins'

  const section = parts[1] || ''
  if (isSettingsTab(section)) return section
  return LEGACY_ROUTE_ALIASES[section] || null
}

export function normalizeRememberedSettingsRoute(value: unknown, fallback: SettingsTab = 'interface'): string {
  const parsed = settingsTabFromRouteValue(value)
  // Keep the legacy General URL stable for old bookmarks and the existing
  // shell navigation contract. SettingsPage canonicalizes it to Interface
  // when deciding which panel to render.
  if (parsed === 'general') return settingsPathForTab('general')
  return settingsPathForTab(parsed ? canonicalSettingsTab(parsed) : fallback)
}

function navigationNodeWithDefaultView(node: SettingsSidebarNavigationNode): SettingsSidebarNavigationNode {
  const children = (node.children || []).map(navigationNodeWithDefaultView)
  return {
    ...node,
    view: node.view || (children.length === 0 ? node.id : undefined),
    children: children.length > 0 ? children : undefined,
  }
}

export function buildSettingsSidebarTabs(
  resolveLabel: (id: SettingsTab, labelKey: string) => string,
  resolveChildren: (id: SettingsTab) => readonly SettingsSidebarNavigationNode[] = () => [],
): SettingsSidebarTab[] {
  return SETTINGS_TAB_CONFIG.map((item) => ({
    id: item.id,
    label: resolveLabel(item.id, item.labelKey),
    icon: item.icon,
    group: item.group,
    keywords: item.keywords,
    children: resolveChildren(item.id).map(navigationNodeWithDefaultView),
  }))
}

export function normalizeSettingsSidebarQuery(raw: string): string {
  return String(raw || '')
    .trim()
    .toLowerCase()
}

function matchesSidebarQuery(query: string, parts: Array<string | undefined>): boolean {
  if (!query) return true
  return parts.some((part) => normalizeSettingsSidebarQuery(part || '').includes(query))
}

function filterNavigationNode(
  node: SettingsSidebarNavigationNode,
  query: string,
): SettingsSidebarNavigationNode | null {
  if (!query) return node

  const selfMatches = matchesSidebarQuery(query, [node.label, node.id, node.description, ...(node.keywords || [])])
  const matchingChildren = (node.children || [])
    .map((child) => filterNavigationNode(child, query))
    .filter((child): child is SettingsSidebarNavigationNode => Boolean(child))

  if (!selfMatches && matchingChildren.length === 0) return null
  return {
    ...node,
    children: selfMatches ? node.children : matchingChildren,
  }
}

function nodeContainsActiveView(node: SettingsSidebarNavigationNode, activeView: string): boolean {
  if (node.view && normalizeSettingsSidebarQuery(node.view) === activeView) return true
  return (node.children || []).some((child) => nodeContainsActiveView(child, activeView))
}

function flattenNavigationNode(args: {
  node: SettingsSidebarNavigationNode
  section: SettingsTab
  activeTab: SettingsTab
  activeView: string
  expandedNodeKeys: ReadonlySet<string>
  forceExpanded: boolean
  key: string
  depth: number
  icon?: SettingsSidebarIconKey
}): SettingsSidebarTabRow[] {
  const children = args.node.children || []
  const hasChildren = children.length > 0
  const branchActive =
    args.activeTab === args.section &&
    (args.node.view
      ? normalizeSettingsSidebarQuery(args.node.view) === args.activeView
      : nodeContainsActiveView(args.node, args.activeView))
  const active = Boolean(args.node.view) && branchActive
  const expanded = hasChildren && (args.forceExpanded || args.expandedNodeKeys.has(args.key))
  const row: SettingsSidebarTabRow = {
    kind: 'item',
    key: args.key,
    section: args.section,
    view: args.node.view,
    label: args.node.label,
    icon: args.icon,
    depth: args.depth,
    active,
    branchActive,
    hasChildren,
    expanded,
  }

  if (!expanded) return [row]
  return [
    row,
    ...children.flatMap((child) =>
      flattenNavigationNode({
        ...args,
        node: child,
        key: `${args.key}/${child.id}`,
        depth: args.depth + 1,
        icon: undefined,
      }),
    ),
  ]
}

export function buildSettingsSidebarGroups(args: {
  query: string
  tabs: SettingsSidebarTab[]
  activeTab: SettingsTab
  activeView?: string
  expandedNodeKeys?: ReadonlySet<string>
}): SettingsSidebarRenderGroup[] {
  const query = normalizeSettingsSidebarQuery(args.query)
  const activeView = normalizeSettingsSidebarQuery(args.activeView || '')
  const expandedNodeKeys = args.expandedNodeKeys || new Set<string>()
  const groups = new Map<SettingsSidebarGroupId, SettingsSidebarTabRow[]>([
    ['core', []],
    ['application', []],
    ['system', []],
  ])

  for (const tab of args.tabs) {
    const filteredRoot = filterNavigationNode(
      {
        id: tab.id,
        label: tab.label,
        keywords: tab.keywords,
        children: tab.children,
      },
      query,
    )
    if (!filteredRoot) continue

    groups.get(tab.group)?.push(
      ...flattenNavigationNode({
        node: filteredRoot,
        section: tab.id,
        activeTab: args.activeTab,
        activeView,
        expandedNodeKeys,
        forceExpanded: Boolean(query),
        key: tab.id,
        depth: 0,
        icon: tab.icon,
      }),
    )
  }

  return Array.from(groups.entries())
    .map(([id, items]) => ({ id, labelKey: GROUP_LABEL_KEYS[id], items }))
    .filter((group) => group.items.length > 0)
}
