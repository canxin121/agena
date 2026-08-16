import { mainTabPath, type MainTabId } from '@/app/navigation/mainTabs'
import { router } from '@/router'
import { useUiStore, type WorkspaceWindowTab } from '@/stores/ui'

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

  async function navigateToWorkspaceWindow(windowId: string, replace = false) {
    const target = ui.getWorkspaceWindowById(windowId)
    if (!target) return
    ui.selectWorkspaceWindow(target.id)
    const location = routeForWindow(target)
    if (replace) {
      await router.replace(location).catch(() => {})
      return
    }
    await router.push(location).catch(() => {})
  }

  async function openWorkspaceLocation(tab: MainTabId, opts?: OpenWorkspaceLocationOptions): Promise<string> {
    const windowId = ui.openWorkspaceWindow(tab, {
      activate: true,
      path: opts?.path || mainTabPath(tab),
      query: opts?.query,
      hash: opts?.hash,
      title: opts?.title,
      matchKeys: opts?.matchKeys,
      reuseExisting: opts?.reuseExisting,
    })
    await navigateToWorkspaceWindow(windowId, opts?.replace === true)
    return windowId
  }

  async function openMainTab(tab: MainTabId, opts?: { path?: string; replace?: boolean }): Promise<string> {
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

    return openWorkspaceLocation(tab, {
      path: opts?.path || mainTabPath(tab),
      replace: opts?.replace,
    })
  }

  return {
    navigateToWorkspaceWindow,
    openWorkspaceLocation,
    openMainTab,
    routeForWindow,
  }
}
