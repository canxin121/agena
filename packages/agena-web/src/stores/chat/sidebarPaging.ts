import * as api from './api'
import { limitBackgroundReads } from '@/lib/backgroundReads'

export { createRequestLimiter } from '@/lib/backgroundReads'

type Options = NonNullable<Parameters<typeof api.listSessions>[0]>

export async function loadSidebarSessionPage(
  options: Options,
  requestedPage: number,
  pageSize: number,
  list = api.listSessions,
  force = false,
) {
  let page = Math.max(0, Math.floor(requestedPage))
  const size = Math.max(1, Math.min(200, Math.floor(pageSize)))
  // Root/bucket lists omit delegated sessions, but a parent's children must
  // include them. Applying the root filter to this query hides every task.
  const parentId = Number(options.parentId)
  const excludeSubagents = options.excludeSubagents ?? !(Number.isSafeInteger(parentId) && parentId > 0)
  const fetchPage = () => {
    const run = () =>
      list({
        ...options,
        limit: size,
        offset: page * size,
        excludeSubagents,
        includeTotal: true,
        ...(force ? { force: true, forceFresh: true } : {}),
      })
    return list === api.listSessions ? run() : limitBackgroundReads(run, options.signal)
  }
  let result = await fetchPage()
  const total = result.total ?? page * size + result.sessions.length + (result.hasMore ? 1 : 0)
  const pageCount = Math.max(1, Math.ceil(total / size))
  if (page >= pageCount) {
    page = pageCount - 1
    result = await fetchPage()
  }
  return { sessions: result.sessions, total, page, pageCount, observation: result.observation }
}
