/** A bounded invalidation queue. An invalidation during a read always leaves
 * one trailing read; a stream of events cannot postpone the first read.
 * Ordinary readers share the current request without creating more work. */
export function createRevalidator(
  read: () => Promise<void>,
  options: { intervalMs?: number; retryMs?: number; enabled?: () => boolean } = {},
) {
  let pending = false
  let disposed = false
  let timer: ReturnType<typeof setTimeout> | undefined
  let flight: Promise<void> | undefined
  let startedAt = -Infinity
  let failures = 0
  const interval = options.intervalMs ?? 250

  function schedule(delay = interval) {
    if (disposed || timer || flight || !pending || options.enabled?.() === false) return
    timer = setTimeout(
      () => {
        timer = undefined
        if (options.enabled?.() === false) return
        void refresh().catch(() => {})
      },
      Math.max(delay, interval - (Date.now() - startedAt)),
    )
  }

  function refresh(): Promise<void> {
    if (disposed) return Promise.resolve()
    if (flight) return flight
    if (timer) clearTimeout(timer)
    timer = undefined
    pending = false
    startedAt = Date.now()
    flight = Promise.resolve()
      .then(read)
      .then(
        () => {
          failures = 0
        },
        (error) => {
          if (options.retryMs !== undefined) pending = true
          failures = Math.min(failures + 1, 8)
          throw error
        },
      )
      .finally(() => {
        flight = undefined
        schedule(failures ? Math.min(30_000, (options.retryMs ?? interval) * 2 ** (failures - 1)) : interval)
      })
    return flight
  }

  return {
    refresh,
    invalidate(delay = interval) {
      pending = true
      schedule(delay)
    },
    resume() {
      schedule(0)
    },
    dispose() {
      disposed = true
      if (timer) clearTimeout(timer)
      timer = undefined
    },
  }
}
