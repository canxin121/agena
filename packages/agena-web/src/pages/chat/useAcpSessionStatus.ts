import { computed, onScopeDispose, ref, watch, type ComputedRef } from 'vue'

import {
  manageNativeAgentSession,
  useExternalAgentCatalog,
  type NativeSessionStatus,
} from '@/features/sessions/api/nativeAgentSessions'
import { isAcpSessionId } from '@/lib/acpSession'
import { rememberAcpViewBinding } from '@/lib/acpViewRecovery'
import type { ExternalAgent } from '@/types/chat'
import {
  acpCanForkSession,
  acpForkPointIds,
  acpIsReadOnlyHistory,
  acpRewindPointIds,
  acpSupportsAdditionalDirectories,
  acpSupportsSystemPrompt,
  normalizeAdditionalDirectories,
} from './acpSessionControls'

export function useAcpSessionStatus(options: {
  sessionId: ComputedRef<string | null>
  externalAgent: ComputedRef<ExternalAgent | undefined>
  workspaceId?: ComputedRef<number | null | undefined>
  title?: ComputedRef<string | undefined>
  isBusy: ComputedRef<boolean>
}) {
  const catalog = useExternalAgentCatalog()
  const status = ref<NativeSessionStatus | null>(null)
  const loading = ref(false)
  const directoriesSaving = ref(false)
  const error = ref('')

  let controller: AbortController | null = null
  let generation = 0
  let disposed = false

  const activeSessionId = computed(() => {
    const sid = options.sessionId.value
    if (!sid || !options.externalAgent.value || !isAcpSessionId(sid)) return null
    return sid
  })

  const agentUnavailable = computed(() => {
    const agentId = options.externalAgent.value
    if (!agentId) return false
    return catalog.value.find((entry) => entry.id === agentId)?.available === false
  })

  async function refresh(): Promise<NativeSessionStatus | null> {
    const sid = activeSessionId.value
    if (!sid || agentUnavailable.value || disposed) {
      status.value = null
      error.value = ''
      loading.value = false
      return null
    }
    const revision = ++generation
    controller?.abort()
    const request = new AbortController()
    controller = request
    loading.value = true
    error.value = ''
    try {
      const result = await manageNativeAgentSession(sid, { action: 'status' }, request.signal)
      if (disposed || revision !== generation || activeSessionId.value !== sid) return null
      if (result.kind === 'status') {
        status.value = result
        if (result.session?.native_id && options.externalAgent.value) {
          rememberAcpViewBinding({
            sessionId: sid,
            workspaceId: options.workspaceId?.value,
            agent: options.externalAgent.value,
            nativeId: result.session.native_id,
            title: result.session.title || options.title?.value,
          })
        }
        return result
      }
      return null
    } catch (reason) {
      if (!disposed && revision === generation && activeSessionId.value === sid && !request.signal.aborted) {
        error.value = reason instanceof Error ? reason.message : String(reason)
      }
      return null
    } finally {
      if (revision === generation) loading.value = false
    }
  }

  async function updateDirectories(directories: readonly string[]): Promise<string[]> {
    const sid = activeSessionId.value
    if (!sid) return []
    const normalized = normalizeAdditionalDirectories(directories)
    directoriesSaving.value = true
    try {
      const result = await manageNativeAgentSession(sid, {
        action: 'set_directories',
        directories: normalized,
      })
      if (result.kind === 'directories') {
        if (activeSessionId.value === sid && status.value) {
          status.value = {
            ...status.value,
            additional_directories: result.directories,
          }
        }
        return result.directories
      }
      return status.value?.additional_directories ?? []
    } finally {
      directoriesSaving.value = false
    }
  }

  watch(
    [activeSessionId, agentUnavailable],
    ([sid, unavailable], [prevSid]) => {
      if (sid !== prevSid) {
        status.value = null
        error.value = ''
      }
      if (!sid || unavailable) {
        generation++
        controller?.abort()
        controller = null
        loading.value = false
        return
      }
      void refresh()
    },
    { immediate: true },
  )

  // Refresh after a turn finishes so newly established native links, fork points,
  // and rewind points are surfaced right away.
  watch(
    () => options.isBusy.value,
    (busy, prevBusy) => {
      if (prevBusy && !busy && activeSessionId.value && !agentUnavailable.value) {
        void refresh()
      }
    },
  )

  onScopeDispose(() => {
    disposed = true
    generation++
    controller?.abort()
    controller = null
  })

  const forkPointIds = computed(() => acpForkPointIds(status.value))
  const rewindPointIds = computed(() => acpRewindPointIds(status.value))
  const isReadOnly = computed(() => acpIsReadOnlyHistory(status.value))
  const canFork = computed(() => acpCanForkSession(status.value))
  const supportsDirectories = computed(() => acpSupportsAdditionalDirectories(status.value))
  const supportsSystemPrompt = computed(() => acpSupportsSystemPrompt(status.value))
  const additionalDirectories = computed(() => status.value?.additional_directories ?? [])

  return {
    status,
    loading,
    directoriesSaving,
    error,
    forkPointIds,
    rewindPointIds,
    isReadOnly,
    canFork,
    supportsDirectories,
    supportsSystemPrompt,
    additionalDirectories,
    refresh,
    updateDirectories,
  }
}
