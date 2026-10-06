/** Bound display reads across panes, including work queued before navigation. */
export function createRequestLimiter(concurrency: number) {
  let active = 0
  const waiting: Array<() => void> = []

  return async <T>(run: () => Promise<T>, signal?: AbortSignal): Promise<T> => {
    signal?.throwIfAborted()
    if (active >= concurrency) {
      await new Promise<void>((resolve, reject) => {
        const ready = () => {
          signal?.removeEventListener('abort', abort)
          resolve()
        }
        const abort = () => {
          const index = waiting.indexOf(ready)
          if (index < 0) return
          waiting.splice(index, 1)
          reject(signal?.reason)
        }
        waiting.push(ready)
        signal?.addEventListener('abort', abort, { once: true })
      })
    } else active++
    try {
      signal?.throwIfAborted()
      return await run()
    } finally {
      const next = waiting.shift()
      if (next) next()
      else active--
    }
  }
}

export const limitBackgroundReads = createRequestLimiter(4)

/** Consumers can cancel independently; cancel the shared read only when its
 * last consumer leaves. This also removes abandoned work from the limiter. */
export function createSharedRead<T>(read: (signal: AbortSignal) => Promise<T>) {
  const controller = new AbortController()
  let consumers = 0
  let settled = false
  const promise = Promise.resolve()
    .then(() => read(controller.signal))
    .then(
      (value) => {
        settled = true
        return value
      },
      (error) => {
        settled = true
        throw error
      },
    )
  return {
    promise,
    isAborted: () => controller.signal.aborted,
    signal: controller.signal,
    abort: () => controller.abort(),
    join(signal?: AbortSignal | null): Promise<T> {
      signal?.throwIfAborted()
      consumers++
      let released = false
      const release = () => {
        if (released) return
        released = true
        signal?.removeEventListener('abort', abort)
        consumers--
        if (!consumers && !settled) controller.abort()
      }
      const abort = () => {
        rejectAbort?.(signal?.reason ?? new DOMException('Aborted', 'AbortError'))
        release()
      }
      let rejectAbort: ((reason: unknown) => void) | undefined
      return new Promise<T>((resolve, reject) => {
        rejectAbort = reject
        signal?.addEventListener('abort', abort, { once: true })
        promise.then(resolve, reject)
      }).finally(release)
    },
  }
}

export function isDocumentVisible() {
  return typeof document === 'undefined' || (!document.hidden && document.visibilityState !== 'hidden')
}
