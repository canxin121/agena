export type MainTabId = 'chat' | 'files' | 'preview' | 'terminal' | 'git' | 'settings' | 'bot' | 'plugins'
export type NavigationMainTabId = Exclude<MainTabId, 'settings' | 'plugins'>
export type PrimaryMainTabId = Exclude<MainTabId, 'plugins'>

export type MainTabDef<T extends MainTabId = MainTabId> = {
  id: T
  path: string
  labelKey: string
}

export const WORKSPACE_MAIN_TABS: MainTabDef[] = [
  { id: 'bot', path: '/bot', labelKey: 'bot.title' },
  { id: 'chat', path: '/chat', labelKey: 'nav.chat' },
  { id: 'files', path: '/files', labelKey: 'nav.files' },
  { id: 'preview', path: '/preview', labelKey: 'nav.preview' },
  { id: 'terminal', path: '/terminal', labelKey: 'nav.terminal' },
  { id: 'git', path: '/git', labelKey: 'nav.git' },
  { id: 'plugins', path: '/plugins', labelKey: 'nav.plugins' },
  { id: 'settings', path: '/settings', labelKey: 'nav.settings' },
]

// Plugin applications remain addressable workspace tabs, but are opened from
// Settings → Plugins and never add a second top-level plugin directory.
export const PRIMARY_MAIN_TABS: MainTabDef<PrimaryMainTabId>[] = WORKSPACE_MAIN_TABS.filter(
  (tab): tab is MainTabDef<PrimaryMainTabId> => tab.id !== 'plugins',
)
export const MAIN_TABS: MainTabDef<NavigationMainTabId>[] = PRIMARY_MAIN_TABS.filter(
  (tab): tab is MainTabDef<NavigationMainTabId> => tab.id !== 'settings',
)

const MAIN_TAB_IDS = new Set<MainTabId>(WORKSPACE_MAIN_TABS.map((tab) => tab.id))

const MAIN_TAB_PATHS = new Map<MainTabId, string>(WORKSPACE_MAIN_TABS.map((tab) => [tab.id, tab.path]))

export function mainTabPath(tab: MainTabId): string {
  return MAIN_TAB_PATHS.get(tab) || '/chat'
}

export function isConversationSettingsPath(path: string): boolean {
  return path === '/assistant' || path === '/integrations'
}

export function isWorkspaceMainTabPath(path: string): boolean {
  const p = String(path || '').toLowerCase()
  return WORKSPACE_MAIN_TABS.some((tab) => p === tab.path || p.startsWith(`${tab.path}/`))
}

export function isMainTabId(value: string): value is MainTabId {
  return MAIN_TAB_IDS.has(value as MainTabId)
}

export function mainTabFromPath(path: string): MainTabId {
  const p = String(path || '').toLowerCase()
  for (const tab of WORKSPACE_MAIN_TABS) {
    if (p === tab.path || p.startsWith(`${tab.path}/`)) return tab.id
  }
  return 'chat'
}

export function normalizeMainTabPath(tab: MainTabId, path?: string | null): string {
  const normalized = String(path || '').trim()
  if (normalized === '/') return mainTabPath(tab)
  if (!normalized.startsWith('/') || !isWorkspaceMainTabPath(normalized)) return mainTabPath(tab)
  return mainTabFromPath(normalized) === tab ? normalized : mainTabPath(tab)
}
