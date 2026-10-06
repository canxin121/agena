import { defineStore } from 'pinia'
import { computed, ref } from 'vue'

import { apiJson, apiUrl } from '../lib/api'

const HEALTH_REQUEST_TIMEOUT_MS = 5000

function timeoutSignal(ms: number): AbortSignal | undefined {
  try {
    if (typeof AbortSignal !== 'undefined' && typeof AbortSignal.timeout === 'function') {
      return AbortSignal.timeout(ms)
    }
  } catch {
    // ignore
  }
  return undefined
}

export type Health = {
  status: string
  generation?: number
  loaded_at?: string
  database_connected?: boolean
  server?: { id?: string; pid?: number; started_at?: string; protocol_version?: number }
}

export const useHealthStore = defineStore('health', () => {
  const data = ref<Health | null>(null)
  const loading = ref(false)
  const error = ref<string | null>(null)

  const serverConnected = computed(() => Boolean(data.value))

  let flight: Promise<void> | undefined
  let nextAllowedAt = 0
  let failures = 0
  let backendUrl = ''
  function refresh(): Promise<void> {
    const url = apiUrl('/api/v1/health')
    if (backendUrl !== url) { backendUrl = url; flight = undefined; nextAllowedAt = 0; failures = 0 }
    if (flight) return flight
    if (Date.now() < nextAllowedAt) return Promise.resolve()
    const current = performRefresh(url).finally(() => { if (flight === current) flight = undefined })
    flight = current
    return current
  }
  async function performRefresh(url: string) {
    loading.value = true
    error.value = null
    try {
      // GET /api/v1/health is public even when UI auth is enabled.
      const next = await apiJson<Health>(url, { signal: timeoutSignal(HEALTH_REQUEST_TIMEOUT_MS) })
      if (url !== backendUrl) return
      data.value = next
      failures = 0
      nextAllowedAt = Date.now() + 1000
    } catch (err) {
      if (url !== backendUrl) return
      error.value = err instanceof Error ? err.message : String(err)
      data.value = null
      failures = Math.min(6, failures + 1)
      nextAllowedAt = Date.now() + Math.min(60_000, 5000 * 2 ** (failures - 1))
    } finally {
      if (url === backendUrl) loading.value = false
    }
  }

  return { data, loading, error, serverConnected, refresh }
})
