import { defineStore } from 'pinia'
import { ref } from 'vue'

import { postAppBroadcast } from '@/lib/appBroadcast'
import type { JsonValue as JsonLike } from '@/types/json'

// Agena has no /api/config/opencode endpoint — opencode's JSON config file
// editing was removed with the opencode settings panels. This store is kept as
// a minimal local stub so the restored chat chrome (model selection, plugin
// host) still compiles and renders; refresh/save resolve locally without a
// server round-trip.

export type OpencodeConfigScope = 'user' | 'project' | 'custom'

export type OpencodeConfigPaths = {
  user?: string
  project?: string
  custom?: string
}

export type OpencodeConfigResponse = {
  scope: OpencodeConfigScope
  path?: string
  exists: boolean
  config: Record<string, JsonLike>
  paths: OpencodeConfigPaths
}

type RefreshOptions = {
  scope?: OpencodeConfigScope
  directory?: string | null
}

export const useOpencodeConfigStore = defineStore('opencodeConfig', () => {
  const data = ref<Record<string, JsonLike> | null>(null)
  const paths = ref<OpencodeConfigPaths | null>(null)
  const activePath = ref<string | null>(null)
  const scope = ref<OpencodeConfigScope>('user')
  const exists = ref<boolean | null>(null)
  const loading = ref(false)
  const error = ref<string | null>(null)

  async function refresh(opts?: RefreshOptions) {
    loading.value = true
    error.value = null
    try {
      const resolvedScope = (opts?.scope || scope.value || 'user').trim()
      // Local stub: no config file exists server-side for agena.
      data.value = {}
      paths.value = null
      activePath.value = null
      scope.value = resolvedScope === 'user' ? 'user' : 'project'
      exists.value = false
    } finally {
      loading.value = false
    }
  }

  async function save(config: Record<string, JsonLike>) {
    // Local stub: record the request in memory for the session and broadcast.
    error.value = null
    data.value = { ...(data.value || {}), ...(config || {}) }
    postAppBroadcast('opencodeConfig.updated', { updatedAt: Date.now(), scope: scope.value })
  }

  return {
    data,
    paths,
    activePath,
    scope,
    exists,
    loading,
    error,
    refresh,
    save,
  }
})
