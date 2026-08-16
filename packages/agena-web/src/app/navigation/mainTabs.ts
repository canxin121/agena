export type MainTabId = 'chat' | 'files' | 'preview' | 'terminal' | 'git' | 'settings'
export type NavigationMainTabId = Exclude<MainTabId, 'settings'>

export type MainTabDef<T extends MainTabId = MainTabId> = {
  id: T
  path: string
  labelKey: string
}

export const WORKSPACE_MAIN_TABS: MainTabDef[] = [
  { id: 'chat', path: '/chat', labelKey: 'nav.chat' },
  { id: 'files', path: '/files', labelKey: 'nav.files' },
  { id: 'preview', path: '/preview', labelKey: 'nav.preview' },
  { id: 'terminal', path: '/terminal', labelKey: 'nav.terminal' },
  { id: 'git', path: '/git', labelKey: 'nav.git' },
  { id: 'settings', path: '/settings', labelKey: 'nav.settings' },
]

export const MAIN_TABS: MainTabDef<NavigationMainTabId>[] = WORKSPACE_MAIN_TABS.filter(
  (tab): tab is MainTabDef<NavigationMainTabId> => tab.id !== 'settings',
)

const MAIN_TAB_IDS = new Set<MainTabId>(WORKSPACE_MAIN_TABS.map((tab) => tab.id))

const MAIN_TAB_PATHS = new Map<MainTabId, string>(WORKSPACE_MAIN_TABS.map((tab) => [tab.id, tab.path]))

export function mainTabPath(tab: MainTabId): string {
  return MAIN_TAB_PATHS.get(tab) || '/chat'
}

export function isWorkspaceMainTabPath(path: string): boolean {
  const p = String(path || '').toLowerCase()
  return WORKSPACE_MAIN_TABS.some((tab) => p.startsWith(tab.path))
}

export function isMainTabId(value: string): value is MainTabId {
  return MAIN_TAB_IDS.has(value as MainTabId)
}

export function mainTabFromPath(path: string): MainTabId {
  const p = String(path || '').toLowerCase()
  for (const tab of WORKSPACE_MAIN_TABS) {
    if (p.startsWith(tab.path)) return tab.id
  }
  return 'chat'
}

export function normalizeMainTabPath(tab: MainTabId, path?: string | null): string {
  const normalized = String(path || '').trim()
  if (normalized === '/') return mainTabPath(tab)
  if (!normalized.startsWith('/')) return mainTabPath(tab)
  return mainTabFromPath(normalized) === tab ? normalized : mainTabPath(tab)
}
