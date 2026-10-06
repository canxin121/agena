/** A bounded invalidation queue. An invalidation during a read always leaves
 * one trailing read; a stream of events cannot postpone the first read.
 * Ordinary readers share the current request without creating more work. */
export function createRevalidator(
  read: () => Promise<void>,
  options: { intervalMs?: number; retryMs?: number; pollIntervalMs?: number; enabled?: () => boolean } = {},
) {
  let pending = false
  let disposed = false
  let timer: ReturnType<typeof setTimeout> | undefined
  let scheduledAt = Infinity
  let flight: Promise<void> | undefined
  let nextAllowedAt = -Infinity
  let failures = 0
  const interval = options.intervalMs ?? 250

  function schedule(delay = interval) {
    if (disposed || flight || !pending || options.enabled?.() === false) return
    const wait = Math.max(0, delay, nextAllowedAt - Date.now())
    const due = Date.now() + wait
    if (timer && scheduledAt <= due) return
    if (timer) clearTimeout(timer)
    scheduledAt = due
    timer = setTimeout(() => {
      timer = undefined
      scheduledAt = Infinity
      if (options.enabled?.() === false) return
      void refresh().catch(() => {})
    }, wait)
  }

  function refresh(): Promise<void> {
    if (disposed) return Promise.resolve()
    if (flight) return flight
    if (options.enabled?.() === false) {
      pending = true
      return Promise.resolve()
    }
    if (Date.now() < nextAllowedAt) {
      pending = true
      schedule(0)
      return Promise.resolve()
    }
    if (timer) clearTimeout(timer)
    timer = undefined
    pending = false
    scheduledAt = Infinity
    flight = Promise.resolve()
      .then(() => {
        if (!disposed) return read()
      })
      .then(
        () => {
          failures = 0
        },
        (error) => {
          if (options.retryMs !== undefined || options.pollIntervalMs !== undefined) pending = true
          // Navigation and visibility cancellation are lifecycle changes,
          // not failed network attempts. Keep pending work for a later resume.
          if (!(error instanceof DOMException && error.name === 'AbortError')) failures = Math.min(failures + 1, 8)
          throw error
        },
      )
      .finally(() => {
        flight = undefined
        const delay = failures
          ? Math.min(60_000, (options.retryMs ?? options.pollIntervalMs ?? interval) * 2 ** (failures - 1))
          : (options.pollIntervalMs ?? interval)
        // Cool down after completion: a slow server must get a breathing gap.
        // New events and visibility resumes cannot shorten a failure backoff.
        nextAllowedAt = Date.now() + (failures ? delay : interval)
        const trailing = pending
        if (options.pollIntervalMs !== undefined) pending = true
        schedule(!failures && trailing ? interval : delay)
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
    pause() {
      if (timer) clearTimeout(timer)
      timer = undefined
      scheduledAt = Infinity
    },
    dispose() {
      disposed = true
      if (timer) clearTimeout(timer)
      timer = undefined
      scheduledAt = Infinity
    },
  }
}
