import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

function extractFunctionSource(source: string, signature: string): string {
  const start = source.indexOf(signature)
  assert.ok(start >= 0, `missing function signature: ${signature}`)

  const blockStart = source.indexOf('{', start)
  assert.ok(blockStart >= 0, `missing block start for: ${signature}`)

  let depth = 0
  for (let idx = blockStart; idx < source.length; idx += 1) {
    const ch = source[idx]
    if (ch === '{') depth += 1
    else if (ch === '}') depth -= 1
    if (depth === 0) {
      return source.slice(start, idx + 1)
    }
  }

  throw new Error(`unterminated function block: ${signature}`)
}

test('workspace route sync opens durable tabs and restores complete locations', () => {
  const uiStoreSource = readFileSync(resolve(import.meta.dir, '../src/stores/ui.ts'), 'utf8')
  const mainLayoutSource = readFileSync(resolve(import.meta.dir, '../src/layout/MainLayout.vue'), 'utf8')
  const sidebarSource = readFileSync(resolve(import.meta.dir, '../src/layout/AppDesktopSidebar.vue'), 'utf8')

  // Persisted state is loaded without fabricating data before the shell knows
  // which route should seed a first tab.
  assert.ok(!uiStoreSource.includes('return [createWorkspaceWindowTab(defaultMainTab)]'))
  assert.ok(mainLayoutSource.includes('if (!ui.workspaceWindows.length)'))
  assert.ok(mainLayoutSource.includes("if (route.path !== '/' && (tab !== 'chat' || sessionId))"))
  assert.ok(mainLayoutSource.includes('<WorkspacePrimaryPaneView :window-id="activePrimaryWindowId" />'))
  assert.ok(uiStoreSource.includes("if (mainTab === 'chat' && !readMatchQueryValue(routeQuery, 'sessionId')) return null"))
  assert.ok(uiStoreSource.includes("if (tab === 'chat' && !readMatchQueryValue(query, 'sessionId')) return ''"))

  const setRouteQueryFn = extractFunctionSource(
    uiStoreSource,
    'function setActiveWorkspaceWindowRouteQuery(rawQuery: unknown)',
  )
  const resolveRouteWindowFn = extractFunctionSource(
    uiStoreSource,
    'function resolveWorkspaceWindowIdFromRouteQuery(rawQuery: unknown)',
  )
  const setActiveMainTabFn = extractFunctionSource(uiStoreSource, 'function setActiveMainTab(tab: MainTab)')

  // Route-query sync should be a no-op when there is no active workspace window.
  assert.ok(setRouteQueryFn.includes('if (!targetId) return'))
  assert.ok(setRouteQueryFn.includes('setWorkspaceWindowRouteQuery(targetId, rawQuery)'))
  assert.ok(!setRouteQueryFn.includes('createWorkspaceWindow('))

  // Main-tab mutation remains a focused-window operation.
  assert.ok(setActiveMainTabFn.includes('activeMainTabFallback.value = tab'))
  assert.ok(setActiveMainTabFn.includes('if (!targetId) return'))
  assert.ok(!setActiveMainTabFn.includes('createWorkspaceWindow('))

  // Desktop navigation creates/reuses a durable tab, while explicit window
  // routes update the complete persisted location.
  const desktopRouteGuardIdx = uiStoreSource.indexOf('if (!isCompactLayout.value)')
  assert.ok(desktopRouteGuardIdx >= 0)
  assert.ok(uiStoreSource.includes('openWorkspaceWindow(tab,'))
  assert.ok(uiStoreSource.includes('setWorkspaceWindowRoutePath(routeWindowId, routePath)'))
  assert.ok(uiStoreSource.includes('setWorkspaceWindowRouteQuery(routeWindowId, normalizedQuery)'))
  assert.ok(uiStoreSource.includes('setWorkspaceWindowRouteHash(routeWindowId, routeHash)'))
  assert.ok(uiStoreSource.includes('routePath: normalizeMainTabPath(mainTab, routePath)'))
  assert.ok(uiStoreSource.includes('routeHash:'))

  // Rail clicks must enter the workspace navigation path instead of changing
  // only the shell URL.
  assert.ok(sidebarSource.includes('workspaceNavigation.openMainTab(tabId'))
  assert.ok(!sidebarSource.includes('await router.push(routeForTab(tabId))'))

  // Route-derived window-title updates should require explicit window scope on desktop.
  assert.ok(resolveRouteWindowFn.includes('if (isCompactLayout.value)'))
  assert.ok(resolveRouteWindowFn.includes('return getResolvedWorkspaceWindowId()'))
  assert.ok(resolveRouteWindowFn.includes("return ''"))
})
