export const SETTINGS_TAB_IDS = ['general', 'providers', 'permissions', 'activities', 'memories', 'usage'] as const

export type SettingsTab = (typeof SETTINGS_TAB_IDS)[number]
export type SettingsSidebarGroupId = 'primary' | 'secondary'
export type SettingsSidebarIconKey =
  | 'general'
  | 'providers'
  | 'permissions'
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
