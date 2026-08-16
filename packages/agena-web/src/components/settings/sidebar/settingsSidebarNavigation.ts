export const SETTINGS_TAB_IDS = [
  'general',
  'providers',
  'permissions',
  'plugins',
  'activities',
  'memories',
  'usage',
] as const

export type SettingsTab = (typeof SETTINGS_TAB_IDS)[number]
export type SettingsSidebarGroupId = 'primary' | 'secondary'
export type SettingsSidebarIconKey =
  | 'general'
  | 'providers'
  | 'permissions'
  | 'plugins'
  | 'activities'
  | 'memories'
  | 'usage'

export type SettingsSidebarTab = {
  id: SettingsTab
  label: string
  icon: SettingsSidebarIconKey
  group: SettingsSidebarGroupId
  keywords?: string[]
}

export type SettingsSidebarTabRow = {
  kind: 'tab'
  id: SettingsTab
  label: string
  icon: SettingsSidebarIconKey
  active: boolean
}

export type SettingsSidebarRenderGroup = {
  id: SettingsSidebarGroupId
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
    id: 'general',
    labelKey: 'settings.tabs.general',
    icon: 'general',
    group: 'primary',
    keywords: ['appearance', 'theme', 'fonts', 'language', 'chat', 'ui'],
  },
  {
    id: 'providers',
    labelKey: 'settings.tabs.providers',
    icon: 'providers',
    group: 'primary',
    keywords: ['model', 'provider', 'adapter', 'llm'],
  },
  {
    id: 'permissions',
    labelKey: 'settings.tabs.permissions',
    icon: 'permissions',
    group: 'primary',
    keywords: ['rules', 'allow', 'security', 'action'],
  },
  {
    id: 'plugins',
    labelKey: 'settings.tabs.plugins',
    icon: 'plugins',
    group: 'primary',
    keywords: ['plugin', 'workbench', 'command', 'control', 'view'],
  },
  {
    id: 'activities',
    labelKey: 'settings.tabs.activities',
    icon: 'activities',
    group: 'secondary',
    keywords: ['background', 'tasks', 'jobs', 'status'],
  },
  {
    id: 'memories',
    labelKey: 'settings.tabs.memories',
    icon: 'memories',
    group: 'secondary',
    keywords: ['memory', 'context', 'knowledge', 'persist'],
  },
  {
    id: 'usage',
    labelKey: 'settings.tabs.usage',
    icon: 'usage',
    group: 'secondary',
    keywords: ['billing', 'quota', 'tokens', 'count', 'stats'],
  },
]

export function isSettingsTab(input: string): input is SettingsTab {
  return SETTINGS_TAB_IDS.includes(input as SettingsTab)
}

export function settingsPathForTab(tab: SettingsTab): string {
  return `/settings/${tab}`
}

export function settingsTabFromRouteValue(value: unknown): SettingsTab | null {
  const raw = String(value || '')
    .trim()
    .toLowerCase()
  if (!raw) return null
  if (isSettingsTab(raw)) return raw

  const path = raw.split(/[?#]/, 1)[0] || ''
  const parts = path.split('/').filter(Boolean)
  if (parts[0] !== 'settings') return null
  if (parts[1] === 'opencode') {
    const legacySection = parts[2] || ''
    if (['provider', 'providers', 'model', 'models'].includes(legacySection)) return 'providers'
    if (['permission', 'permissions'].includes(legacySection)) return 'permissions'
    if (['plugin', 'plugins', 'command', 'commands', 'skill', 'skills'].includes(legacySection)) return 'plugins'
    if (['activity', 'activities', 'task', 'tasks'].includes(legacySection)) return 'activities'
    if (['memory', 'memories'].includes(legacySection)) return 'memories'
    if (legacySection === 'usage') return 'usage'
    return 'general'
  }
  if (parts[1] === 'plan') return 'plugins'
  const section = parts[1] || ''
  return isSettingsTab(section) ? section : null
}

export function normalizeRememberedSettingsRoute(value: unknown, fallback: SettingsTab = 'general'): string {
  return settingsPathForTab(settingsTabFromRouteValue(value) || fallback)
}

export function buildSettingsSidebarTabs(
  resolveLabel: (id: SettingsTab, labelKey: string) => string,
): SettingsSidebarTab[] {
  return SETTINGS_TAB_CONFIG.map((item) => ({
    id: item.id,
    label: resolveLabel(item.id, item.labelKey),
    icon: item.icon,
    group: item.group,
    keywords: item.keywords,
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

export function buildSettingsSidebarGroups(args: {
  query: string
  tabs: SettingsSidebarTab[]
  activeTab: SettingsTab
}): SettingsSidebarRenderGroup[] {
  const query = normalizeSettingsSidebarQuery(args.query)

  const groups = new Map<SettingsSidebarGroupId, SettingsSidebarTabRow[]>([
    ['primary', []],
    ['secondary', []],
  ])

  for (const tab of args.tabs) {
    const selfMatches = matchesSidebarQuery(query, [tab.label, tab.id, ...(tab.keywords || [])])
    if (!selfMatches) continue

    groups.get(tab.group)?.push({
      kind: 'tab',
      id: tab.id,
      label: tab.label,
      icon: tab.icon,
      active: args.activeTab === tab.id,
    })
  }

  return Array.from(groups.entries())
    .map(([id, items]) => ({ id, items }))
    .filter((group) => group.items.length > 0)
}
