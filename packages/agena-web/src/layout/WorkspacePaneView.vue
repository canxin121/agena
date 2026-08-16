<script setup lang="ts">
import { computed, provide, ref, shallowReactive, watch } from 'vue'
import {
  loadRouteLocation,
  RouterView,
  routeLocationKey,
  routerKey,
  useRouter,
  type LocationQuery,
  type RouteLocationNormalizedLoaded,
  type RouteLocationRaw,
  type Router,
} from 'vue-router'

import { mainTabFromPath, mainTabPath } from '@/app/navigation/mainTabs'
import { workspacePaneContextKey } from '@/app/workspace/workspacePaneContext'
import { useUiStore } from '@/stores/ui'

const props = defineProps<{
  windowId: string
}>()

const ui = useUiStore()
const globalRouter = useRouter()

const windowId = computed(() => String(props.windowId || '').trim())
const windowTab = computed(() => ui.getWorkspaceWindowById(windowId.value))
const isFocused = computed(() => windowId.value !== '' && windowId.value === ui.focusedWorkspaceWindowId)

function normalizeQuery(raw: LocationQuery | Record<string, unknown> | null | undefined): Record<string, string> {
  const out: Record<string, string> = {}
  for (const [rawKey, rawValue] of Object.entries(raw || {})) {
    const key = String(rawKey || '').trim()
    if (!key || key === 'windowId' || key === 'windowid' || key === 'ocEmbed') continue
    const value = Array.isArray(rawValue)
      ? String(rawValue.find((item) => String(item || '').trim()) || '').trim()
      : String(rawValue || '').trim()
    if (value) out[key] = value
  }
  return out
}

const resolvedRoute = computed(() => {
  const target = windowTab.value
  const path = String(target?.routePath || '').trim() || mainTabPath(target?.mainTab || 'chat')
  const query = {
    ...(target?.routeQuery || {}),
    ...(windowId.value ? { windowId: windowId.value } : {}),
  }
  return globalRouter.resolve({
    path,
    ...(Object.keys(query).length ? { query } : {}),
    ...(target?.routeHash ? { hash: target.routeHash } : {}),
  })
})

const scopedRoute = shallowReactive({ ...resolvedRoute.value }) as RouteLocationNormalizedLoaded
const routeReady = ref(false)
let routeLoadSequence = 0

watch(
  resolvedRoute,
  async (next) => {
    const sequence = ++routeLoadSequence

    try {
      const loaded = await loadRouteLocation(next)
      if (sequence !== routeLoadSequence) return

      Object.assign(scopedRoute, loaded)
      routeReady.value = true
    } catch (error) {
      if (sequence !== routeLoadSequence) return

      routeReady.value = false
      console.error('[workspace] failed to load pane route', {
        path: next.fullPath,
        error,
      })
    }
  },
  { immediate: true },
)

const scopedRouteRef = computed<RouteLocationNormalizedLoaded>(() => scopedRoute)

async function navigate(to: RouteLocationRaw, replace = false): Promise<unknown> {
  const targetId = windowId.value
  if (!targetId || !ui.getWorkspaceWindowById(targetId)) return undefined

  const resolved = globalRouter.resolve(to, scopedRoute)
  const mainTab = mainTabFromPath(resolved.path)
  const query = normalizeQuery(resolved.query)

  ui.setWorkspaceWindowMainTab(targetId, mainTab)
  ui.setWorkspaceWindowRoutePath(targetId, resolved.path)
  ui.setWorkspaceWindowRouteQuery(targetId, query)
  ui.setWorkspaceWindowRouteHash(targetId, resolved.hash)

  if (!isFocused.value) return undefined

  const shellLocation = {
    path: resolved.path,
    query: {
      ...query,
      windowId: targetId,
    },
    hash: resolved.hash,
  }
  return replace ? globalRouter.replace(shellLocation) : globalRouter.push(shellLocation)
}

const scopedRouter = new Proxy(globalRouter, {
  get(target, property) {
    if (property === 'currentRoute') return scopedRouteRef
    if (property === 'push') return (to: RouteLocationRaw) => navigate(to, false)
    if (property === 'replace') return (to: RouteLocationRaw) => navigate(to, true)
    if (property === 'resolve') {
      return (to: RouteLocationRaw) => globalRouter.resolve(to, scopedRoute)
    }

    const value = Reflect.get(target, property, target)
    return typeof value === 'function' ? value.bind(target) : value
  },
}) as Router

provide(routeLocationKey, scopedRoute)
provide(routerKey, scopedRouter)
provide(workspacePaneContextKey, {
  windowId,
  isFocused,
  route: scopedRouteRef,
  navigate,
})
</script>

<template>
  <section class="flex h-full min-h-0 flex-col bg-background" :data-workspace-pane-window="windowId">
    <div class="min-h-0 flex-1 overflow-hidden">
      <RouterView v-if="routeReady" :route="scopedRoute" />
      <div v-else class="h-full min-h-0 animate-pulse bg-muted/20" aria-hidden="true" />
    </div>
  </section>
</template>
