import { defineStore } from 'pinia'
import { computed, ref } from 'vue'

import { apiJson } from '../lib/api'

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

  async function refresh() {
    loading.value = true
    error.value = null
    try {
      // GET /api/v1/health is public even when UI auth is enabled.
      data.value = await apiJson<Health>('/api/v1/health', { signal: timeoutSignal(HEALTH_REQUEST_TIMEOUT_MS) })
    } catch (err) {
      error.value = err instanceof Error ? err.message : String(err)
      data.value = null
    } finally {
      loading.value = false
    }
  }

  return { data, loading, error, serverConnected, refresh }
})
