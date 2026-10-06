import * as api from './api'
import { limitBackgroundReads } from '@/lib/backgroundReads'

export { createRequestLimiter } from '@/lib/backgroundReads'

type Options = NonNullable<Parameters<typeof api.listSessions>[0]>

export async function loadSidebarSessionPage(
  options: Options,
  requestedPage: number,
  pageSize: number,
  list = api.listSessions,
) {
  let page = Math.max(0, Math.floor(requestedPage))
  const size = Math.max(1, Math.min(200, Math.floor(pageSize)))
  const fetchPage = () => {
    const run = () => list({ ...options, limit: size, offset: page * size, excludeSubagents: true, includeTotal: true })
    return list === api.listSessions ? run() : limitBackgroundReads(run, options.signal)
  }
  let result = await fetchPage()
  const total = result.total ?? page * size + result.sessions.length + (result.hasMore ? 1 : 0)
  const pageCount = Math.max(1, Math.ceil(total / size))
  if (page >= pageCount) {
    page = pageCount - 1
    result = await fetchPage()
  }
  return { sessions: result.sessions, total, page, pageCount }
}
