import { mainTabPath, type MainTabId } from '@/app/navigation/mainTabs'
import { router } from '@/router'
import { useUiStore, type WorkspaceWindowTab } from '@/stores/ui'
import { useDirectoryStore } from '@/stores/directory'
import { resourceDirectory } from '@/app/workspace/resourceDirectory'

const navigationInFlightByPath = new Map<string, Promise<unknown>>()

type OpenWorkspaceLocationOptions = {
  path?: string
  query?: unknown
  hash?: string
  title?: string
  matchKeys?: string[]
  reuseExisting?: boolean
  replace?: boolean
}

export function useWorkspaceNavigation() {
  const ui = useUiStore()
  const directoryStore = useDirectoryStore()

  function routeForWindow(windowTab: WorkspaceWindowTab) {
    return {
      path: windowTab.routePath || mainTabPath(windowTab.mainTab),
      query: {
        ...(windowTab.routeQuery || {}),
        windowId: windowTab.id,
      },
      hash: windowTab.routeHash || '',
    }
  }

  async function navigateToEmptyWorkspace(replace = true) {
    ui.closeAllWorkspaceWindows()
    const location = { path: '/' }
    const resolved = router.resolve(location)
    if (router.currentRoute.value.fullPath === resolved.fullPath) return

    const key = resolved.fullPath
    const existing = navigationInFlightByPath.get(key)
    if (existing) {
      await existing
      return
    }

    const request = (replace ? router.replace(location) : router.push(location)).catch(() => {})
    navigationInFlightByPath.set(key, request)
    try {
      await request
    } finally {
      if (navigationInFlightByPath.get(key) === request) navigationInFlightByPath.delete(key)
    }
  }

  async function navigateToWorkspaceWindow(windowId: string, replace = false) {
    const target = ui.getWorkspaceWindowById(windowId)
    if (!target) return
    ui.selectWorkspaceWindow(target.id)
    const directory = resourceDirectory(target.routeQuery, directoryStore.getDirectoryForWindow(target.id) || '')
    if (directory) directoryStore.setDirectoryForWindow(target.id, directory)
    const location = routeForWindow(target)
    const resolved = router.resolve(location)
    if (router.currentRoute.value.fullPath === resolved.fullPath) return

    // A tab click and MainLayout's active-window watcher can both request the
    // same navigation in the same tick.  Share that request instead of
    // issuing two router transitions (which also causes every scoped pane to
    // reevaluate its route).
    const key = resolved.fullPath
    const existing = navigationInFlightByPath.get(key)
    if (existing) {
      await existing
      return
    }

    const request = (replace ? router.replace(location) : router.push(location)).catch(() => {})
    navigationInFlightByPath.set(key, request)
    try {
      await request
    } finally {
      if (navigationInFlightByPath.get(key) === request) navigationInFlightByPath.delete(key)
    }
  }

  async function openWorkspaceLocation(tab: MainTabId, opts?: OpenWorkspaceLocationOptions): Promise<string> {
    const query = { ...((opts?.query || {}) as Record<string, unknown>) }
    if (tab === 'files' || tab === 'git' || tab === 'terminal') {
      const directory = resourceDirectory(query, directoryStore.currentDirectory || '')
      if (directory) query.directory = directory
    }
    const windowId = ui.openWorkspaceWindow(tab, {
      activate: true,
      path: opts?.path || mainTabPath(tab),
      query,
      hash: opts?.hash,
      title: opts?.title,
      matchKeys: query.directory && opts?.matchKeys ? [...new Set(['directory', ...opts.matchKeys])] : opts?.matchKeys,
      reuseExisting: opts?.reuseExisting,
    })
    if (query.directory) directoryStore.setDirectoryForWindow(windowId, String(query.directory))
    await navigateToWorkspaceWindow(windowId, opts?.replace === true)
    return windowId
  }

  async function openMainTab(tab: MainTabId, opts?: { path?: string; replace?: boolean }): Promise<string> {
    // Tool entry points use the selected folder, even when another folder's
    // Files/Git tab is already open. Resource tabs keep their original owner.
    if (tab === 'files' || tab === 'git' || tab === 'terminal') {
      return openWorkspaceLocation(tab, {
        path: opts?.path,
        query: { directory: directoryStore.currentDirectory || '' },
        replace: opts?.replace,
      })
    }
    const activeGroup = ui.activeWorkspaceGroup
    const groupWindows = activeGroup
      ? activeGroup.tabIds
          .map((windowId) => ui.getWorkspaceWindowById(windowId))
          .filter((item): item is WorkspaceWindowTab => Boolean(item))
      : []
    const existing =
      groupWindows.find((item) => item.mainTab === tab) || ui.workspaceWindows.find((item) => item.mainTab === tab)

    if (existing) {
      await navigateToWorkspaceWindow(existing.id, opts?.replace === true)
      return existing.id
    }

    // Chat without a concrete session is the stable empty-workspace surface,
    // not a durable workspace window. This also gives the shell a recovery
    // path from context-less non-chat tabs such as Files with no project.
    if (tab === 'chat') {
      await navigateToEmptyWorkspace(true)
      return ''
    }

    return openWorkspaceLocation(tab, {
      path: opts?.path || mainTabPath(tab),
      replace: opts?.replace,
    })
  }

  return {
    navigateToWorkspaceWindow,
    navigateToEmptyWorkspace,
    openWorkspaceLocation,
    openMainTab,
    routeForWindow,
  }
}
