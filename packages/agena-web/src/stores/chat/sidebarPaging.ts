import * as api from './api'

// Keep a directory page with many expanded trees from saturating the connection.
export function createRequestLimiter(concurrency: number) {
  let active = 0
  const waiting: Array<() => void> = []
  return async <T>(run: () => Promise<T>, signal?: AbortSignal): Promise<T> => {
    if (active >= concurrency) await new Promise<void>((resolve) => waiting.push(resolve))
    else active++
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

const limitRequests = createRequestLimiter(6)
type Options = NonNullable<Parameters<typeof api.listSessions>[0]>

export async function loadSidebarSessionPage(
  options: Options,
  requestedPage: number,
  pageSize: number,
  list = api.listSessions,
) {
  let page = Math.max(0, Math.floor(requestedPage))
  const size = Math.max(1, Math.min(200, Math.floor(pageSize)))
  const fetchPage = () =>
    limitRequests(
      () => list({ ...options, limit: size, offset: page * size, excludeSubagents: true, includeTotal: true }),
      options.signal,
    )
  let result = await fetchPage()
  const total = result.total ?? page * size + result.sessions.length + (result.hasMore ? 1 : 0)
  const pageCount = Math.max(1, Math.ceil(total / size))
  if (page >= pageCount) {
    page = pageCount - 1
    result = await fetchPage()
  }
  return { sessions: result.sessions, total, page, pageCount }
}
