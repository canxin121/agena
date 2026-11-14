import { defineStore } from 'pinia'
import { ref, shallowRef } from 'vue'
import { apiJson, apiUrl, ApiError } from '@/lib/api'
import { parsePluginPages } from './model'
import { PluginStyleRegistry } from './styles'
import type { PluginPageCatalogItem } from './sdk'
import { parsePluginStatusList, type PluginRuntimeStatus } from './hub'

/** A single catalog refresh loop is shared by all plugin panes. */
export const usePluginWebCatalog = defineStore('plugin-web-catalog', () => {
  const pages = shallowRef<PluginPageCatalogItem[]>([])
  const plugins = shallowRef<PluginRuntimeStatus[]>([])
  const loading = ref(false)
  const loaded = ref(false)
  const error = ref('')
  let users = 0
  let current: { controller: AbortController; promise: Promise<void> } | undefined
  let timer: ReturnType<typeof setTimeout> | undefined
  let styles: PluginStyleRegistry | undefined

  function cancelTimer() {
    if (timer !== undefined) clearTimeout(timer)
    timer = undefined
  }
  function schedule() {
    cancelTimer()
    if (users && document.visibilityState !== 'hidden') timer = setTimeout(() => void refresh(), 5000)
  }
  function isCurrent(controller: AbortController): boolean {
    return current?.controller === controller
  }
  function refresh(): Promise<void> {
    if (current) return current.promise
    cancelTimer()
    const controller = new AbortController()
    loading.value = true
    const promise = (async () => {
      try {
        // An installed plugin need not declare surface.web. Read the runtime
        // inventory separately, allowing either endpoint to be temporarily
        // unavailable without hiding the other surface.
        const [surface, runtime] = await Promise.allSettled([
          apiJson<{ catalog?: { web_pages?: unknown } }>('/api/v1/plugins/surface', {
            signal: controller.signal,
          }),
          apiJson<unknown>('/api/v1/plugins', { signal: controller.signal }),
        ])
        if (controller.signal.aborted) return
        let successful = false
        const failures: string[] = []
        if (surface.status === 'fulfilled') {
          try {
            pages.value = parsePluginPages(surface.value.catalog?.web_pages, new URL(apiUrl('/')).origin)
            successful = true
          } catch (cause) {
            failures.push(cause instanceof Error ? cause.message : String(cause))
          }
        } else {
          if (surface.reason instanceof ApiError && surface.reason.status === 401) pages.value = []
          failures.push(surface.reason instanceof Error ? surface.reason.message : String(surface.reason))
        }
        if (runtime.status === 'fulfilled') {
          try {
            plugins.value = parsePluginStatusList(runtime.value)
            successful = true
          } catch (cause) {
            failures.push(cause instanceof Error ? cause.message : String(cause))
          }
        } else {
          if (runtime.reason instanceof ApiError && runtime.reason.status === 401) plugins.value = []
          failures.push(runtime.reason instanceof Error ? runtime.reason.message : String(runtime.reason))
        }
        if (!successful) throw new Error(failures.join('; '))
        loaded.value = true
        error.value = failures.join('; ')
      } catch (cause) {
        if (controller.signal.aborted) return
        if (cause instanceof ApiError && cause.status === 401) {
          pages.value = []
          plugins.value = []
        }
        error.value = cause instanceof Error ? cause.message : String(cause)
      } finally {
        if (isCurrent(controller)) {
          current = undefined
          loading.value = false
          schedule()
        }
      }
    })()
    current = { controller, promise }
    return promise
  }
  function visibility() {
    if (document.visibilityState === 'hidden') cancelTimer()
    else void refresh()
  }
  function retain(): () => void {
    users++
    if (users === 1) {
      document.addEventListener('visibilitychange', visibility)
      void refresh()
    }
    let released = false
    return () => {
      if (released) return
      released = true
      if (--users !== 0) return
      cancelTimer()
      document.removeEventListener('visibilitychange', visibility)
      current?.controller.abort()
      current = undefined
      loading.value = false
    }
  }
  function acquireStyles(urls: string[], signal: AbortSignal) {
    styles ??= new PluginStyleRegistry(document)
    return styles.acquire(urls, signal)
  }
  return { pages, plugins, loading, loaded, error, refresh, retain, acquireStyles }
})
