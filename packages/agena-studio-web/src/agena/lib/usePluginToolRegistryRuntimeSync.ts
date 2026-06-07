import { onBeforeUnmount, onMounted, type Ref } from 'vue'

import {
  fetchRuntimeStatus,
  listPluginToolRegistryChanges,
  streamPluginToolRegistryChanges,
  type NotificationStreamHandle,
  type RuntimeStatus,
  type ToolRegistryChangedEvent,
} from './agenaApi'

export type PluginToolRegistryRuntimeSyncInput = {
  runtime: Ref<RuntimeStatus | null>
}

export type PluginToolRegistryRuntimeSyncDeps = {
  fetchRuntimeStatus: typeof fetchRuntimeStatus
  listPluginToolRegistryChanges: typeof listPluginToolRegistryChanges
  streamPluginToolRegistryChanges: typeof streamPluginToolRegistryChanges
}

const defaultDeps: PluginToolRegistryRuntimeSyncDeps = {
  fetchRuntimeStatus,
  listPluginToolRegistryChanges,
  streamPluginToolRegistryChanges,
}

export type PluginToolRegistryRuntimeSyncOptions = {
  registerComponentLifecycle?: boolean
  onError?: (error: Error) => void
  onRuntimeRefreshed?: (runtime: RuntimeStatus) => void
}

export function usePluginToolRegistryRuntimeSync(
  input: PluginToolRegistryRuntimeSyncInput,
  deps: PluginToolRegistryRuntimeSyncDeps = defaultDeps,
  options: PluginToolRegistryRuntimeSyncOptions = {},
) {
  const registerComponentLifecycle = options.registerComponentLifecycle !== false
  let refreshInFlight = false
  let refreshQueued = false
  let refreshTimer: ReturnType<typeof setTimeout> | null = null
  let streamHandle: NotificationStreamHandle | null = null

  function currentToolRegistryGeneration(): number {
    return Math.max(0, Math.trunc(input.runtime.value?.operator.ui?.tool_registry_generation ?? 0))
  }

  function applyRuntime(runtime: RuntimeStatus) {
    input.runtime.value = runtime
    options.onRuntimeRefreshed?.(runtime)
  }

  function stopScheduledRefresh() {
    refreshQueued = false
    if (!refreshTimer) return
    clearTimeout(refreshTimer)
    refreshTimer = null
  }

  async function refreshRuntime() {
    if (refreshInFlight) {
      refreshQueued = true
      return
    }

    refreshInFlight = true
    try {
      const runtime = await deps.fetchRuntimeStatus()
      applyRuntime(runtime)
    } catch (error) {
      options.onError?.(error instanceof Error ? error : new Error(String(error)))
    } finally {
      refreshInFlight = false
      if (refreshQueued) {
        refreshQueued = false
        scheduleRefresh(0)
      }
    }
  }

  function scheduleRefresh(delayMs = 120) {
    if (refreshTimer) return
    refreshTimer = setTimeout(() => {
      refreshTimer = null
      void refreshRuntime()
    }, Math.max(0, Math.trunc(delayMs)))
  }

  async function reconcileLagged() {
    try {
      const afterGeneration = currentToolRegistryGeneration()
      const response = await deps.listPluginToolRegistryChanges({
        afterGeneration,
        limit: 100,
      })
      if ((response.events ?? []).length > 0 || response.generation > afterGeneration) {
        scheduleRefresh(0)
      }
    } catch (error) {
      options.onError?.(error instanceof Error ? error : new Error(String(error)))
      scheduleRefresh(0)
    }
  }

  function handleRegistryEvent(event: ToolRegistryChangedEvent) {
    if (event.generation <= currentToolRegistryGeneration()) return
    scheduleRefresh(40)
  }

  function start() {
    if (streamHandle) return
    if (typeof ReadableStream === 'undefined' || typeof TextDecoder === 'undefined') return

    streamHandle = deps.streamPluginToolRegistryChanges({
      sinceSeqGlobal: null,
      onEvent: handleRegistryEvent,
      onLagged: () => {
        void reconcileLagged()
      },
      onError: (error) => {
        options.onError?.(error)
      },
    })
  }

  function stop() {
    streamHandle?.close()
    streamHandle = null
    stopScheduledRefresh()
  }

  if (registerComponentLifecycle) {
    onMounted(start)
    onBeforeUnmount(stop)
  }

  return {
    refreshRuntime,
    start,
    stop,
  }
}
