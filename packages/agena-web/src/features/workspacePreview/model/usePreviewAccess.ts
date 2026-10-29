import { computed, onScopeDispose, ref, shallowRef, watch, type Ref } from 'vue'
import type { WorkspacePreviewSession } from '../api/workspacePreviewApi'
import { requestWorkspacePreviewAccess } from '../api/previewAccessApi'
import { buildIsolatedPreviewFrameSrc, previewRunIdentity, type WorkspacePreviewAccess } from './previewAccess'

const RENEW_INTERVAL_MS = 10 * 60_000
const RETRY_INTERVAL_MS = 30_000

export function usePreviewAccess(options: {
  session: Readonly<Ref<WorkspacePreviewSession | null>>
  visible: Readonly<Ref<boolean>>
  refreshToken: Readonly<Ref<number>>
  request?: typeof requestWorkspacePreviewAccess
}) {
  const access = shallowRef<WorkspacePreviewAccess | null>(null)
  const accessError = ref('')
  const accessLoading = ref(false)
  const documentVisible = ref(typeof document === 'undefined' || document.visibilityState !== 'hidden')
  const active = computed(() => options.visible.value && documentVisible.value)
  const runIdentity = computed(() => previewRunIdentity(options.session.value))
  const previewSrc = computed(() =>
    access.value ? buildIsolatedPreviewFrameSrc(access.value, options.refreshToken.value) : '',
  )
  let timer: ReturnType<typeof setTimeout> | undefined
  let controller: AbortController | undefined
  let revision = 0
  let currentIdentity = ''
  let disposed = false

  function cancelPending() {
    revision++
    controller?.abort()
    controller = undefined
    clearTimeout(timer)
    timer = undefined
    accessLoading.value = false
  }

  function scheduleRenewal(delay: number) {
    clearTimeout(timer)
    if (active.value && !disposed) timer = setTimeout(() => void refreshAccess(), Math.max(1_000, delay))
  }

  async function refreshAccess() {
    cancelPending()
    const identity = runIdentity.value
    if (identity !== currentIdentity || (access.value && access.value.expiresAt <= Date.now())) access.value = null
    currentIdentity = identity
    accessError.value = ''
    if (!active.value || !identity || disposed) return

    const requestRevision = revision
    const requestController = new AbortController()
    controller = requestController
    accessLoading.value = true
    try {
      const result = await (options.request ?? requestWorkspacePreviewAccess)(
        options.session.value!.id,
        AbortSignal.any([requestController.signal, AbortSignal.timeout(15_000)]),
      )
      if (requestRevision !== revision || requestController.signal.aborted || !active.value || disposed) return
      access.value = result
      // A renewal preserves the same URL, so the iframe and in-app route stay mounted.
      scheduleRenewal(Math.min(RENEW_INTERVAL_MS, result.expiresAt - Date.now() - 60_000))
    } catch (error) {
      if (requestRevision !== revision || requestController.signal.aborted || disposed) return
      accessError.value = error instanceof Error ? error.message : String(error)
      if (access.value && access.value.expiresAt <= Date.now()) access.value = null
      scheduleRenewal(
        Math.min(RETRY_INTERVAL_MS, access.value ? access.value.expiresAt - Date.now() : RETRY_INTERVAL_MS),
      )
    } finally {
      if (requestRevision === revision) {
        controller = undefined
        accessLoading.value = false
      }
    }
  }

  watch([runIdentity, active, options.refreshToken], () => void refreshAccess(), { immediate: true, flush: 'sync' })
  const onVisibilityChange = () => {
    documentVisible.value = document.visibilityState !== 'hidden'
  }
  if (typeof document !== 'undefined') document.addEventListener('visibilitychange', onVisibilityChange)
  onScopeDispose(() => {
    disposed = true
    cancelPending()
    if (typeof document !== 'undefined') document.removeEventListener('visibilitychange', onVisibilityChange)
  })

  return { previewSrc, accessError, accessLoading, refreshAccess }
}
