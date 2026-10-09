import { i18n } from '@/i18n'
import { isDocumentVisible } from './backgroundReads'
import { conditionalJson } from './conditionalJson'
import { createRevalidator } from './revalidation'
import { captureResourceObservation, checkResourceVersions, subscribeResource } from './resourceSync'

export type RuntimeBackgroundTask = {
  id: string
  kind: string
  status: string
  title?: string
  message?: string | null
  failure?: { fallback?: string; title?: string; detail?: string; user?: { fallback?: string } } | null
}

/** Runtime maintenance tasks already publish unified activity descriptors.
 * Reuse that stream and the shared revision heartbeat instead of polling all
 * tasks. One initial read closes the gap between starting and subscribing. */
export function waitForRuntimeTask<T extends RuntimeBackgroundTask>(
  initial: T,
  options: { signal?: AbortSignal; timeoutMs?: number } = {},
): Promise<T> {
  options.signal?.throwIfAborted()
  if (initial.status !== 'running') return Promise.resolve(initial)
  const resource = `activity:${initial.id}`
  const scope = captureResourceObservation(resource).scope
  return new Promise<T>((resolve, reject) => {
    let settled = false
    let hasRead = false
    let activeRead: AbortController | undefined
    let timer: ReturnType<typeof setTimeout> | undefined
    let release: (() => void) | undefined

    function cleanup() {
      settled = true
      queue.dispose()
      release?.()
      activeRead?.abort()
      clearTimeout(timer)
      document.removeEventListener('visibilitychange', visibility)
      options.signal?.removeEventListener('abort', cancel)
    }
    function complete(task: RuntimeBackgroundTask) {
      if (settled || task.id !== initial.id || task.status === 'running') return
      cleanup()
      // The activity's kind is "runtime"; retain the operation's exact kind.
      resolve({ ...initial, ...task, kind: initial.kind })
    }
    function fail(reason: unknown) {
      if (settled) return
      cleanup()
      reject(reason)
    }
    function cancel() {
      fail(options.signal?.reason ?? new DOMException('Aborted', 'AbortError'))
    }
    function checkScope() {
      if (scope === captureResourceObservation(resource).scope) return true
      fail(new Error(i18n.global.t('errors.task.backendChanged')))
      return false
    }
    const queue = createRevalidator(
      async () => {
        if (settled || !checkScope()) return
        const controller = new AbortController()
        activeRead = controller
        try {
          if (hasRead) await checkResourceVersions([resource], controller.signal)
          const task = await conditionalJson<RuntimeBackgroundTask>(
            resource,
            `/api/v1/activities/${encodeURIComponent(initial.id)}`,
            { signal: controller.signal },
          )
          if (!settled && checkScope()) {
            hasRead = true
            complete(task)
          }
        } catch (error) {
          if (settled || (controller.signal.aborted && !isDocumentVisible())) return
          throw error
        } finally {
          if (activeRead === controller) activeRead = undefined
        }
      },
      { intervalMs: 1000, retryMs: 5000, enabled: () => !settled && isDocumentVisible() },
    )

    function visibility() {
      if (isDocumentVisible()) {
        queue.invalidate(0)
        queue.resume()
      } else {
        queue.pause()
        activeRead?.abort()
      }
    }
    release = subscribeResource(resource, (event) => {
      if (settled || !checkScope()) return
      const payload = event?.properties?.payload
      const activity = payload && typeof payload === 'object' && !Array.isArray(payload) ? payload.activity : null
      if (
        event?.type === 'runtime_signal' &&
        event.properties?.kind === 'activity' &&
        activity &&
        typeof activity === 'object' &&
        !Array.isArray(activity) &&
        activity.id === initial.id &&
        typeof activity.status === 'string'
      ) {
        complete(activity as RuntimeBackgroundTask)
      } else queue.invalidate(0)
    })
    document.addEventListener('visibilitychange', visibility)
    options.signal?.addEventListener('abort', cancel, { once: true })
    timer = setTimeout(() => {
      const timeout = new DOMException(
        'The runtime task is still running. Check Runtime background tasks for progress.',
        'TimeoutError',
      )
      if (!isDocumentVisible() || !checkScope()) {
        fail(timeout)
        return
      }
      // A short operation can finish during an SSE gap before the shared
      // heartbeat is due. Validate once at its deadline before reporting a
      // timeout, without reinstating a per-operation polling loop.
      queue.dispose()
      activeRead?.abort()
      const controller = new AbortController()
      activeRead = controller
      void conditionalJson<RuntimeBackgroundTask>(
        resource,
        `/api/v1/activities/${encodeURIComponent(initial.id)}`,
        { signal: AbortSignal.any([controller.signal, AbortSignal.timeout(1000)]) },
        true,
      )
        .then((task) => {
          if (!settled && checkScope()) complete(task)
        })
        .catch(() => {})
        .finally(() => fail(timeout))
    }, options.timeoutMs ?? 30_000)
    queue.invalidate(0)
  })
}
