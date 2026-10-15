import { createLatestRequestGuard } from '../../lib/latestRequest'
import type { GitStatusResponse } from '@/types/git'
import type { useGitStatusPaged } from './useGitStatusPaged'

type Scope = 'merge' | 'staged' | 'unstaged' | 'untracked'

export function useGitStatusSummary(opts: {
  directory: { value: string | null }
  status: { value: GitStatusResponse | null }
  generation: () => number
  readSignal: () => AbortSignal
  read: (directory: string, scopes: Scope[], signal?: AbortSignal) => Promise<GitStatusResponse>
  pages: Pick<ReturnType<typeof useGitStatusPaged>, 'beginFirstPages' | 'applySummaryPages'>
  invalidate: (scopes: Scope[]) => void
}) {
  const beginRead = createLatestRequestGuard(() => opts.directory.value, opts.status)

  async function loadSummary(directory: string, scopes: Scope[] = [], signal?: AbortSignal) {
    const generation = opts.generation()
    const isCurrent = beginRead()
    const owner = opts.readSignal()
    const finishFirstPages = opts.pages.beginFirstPages(scopes)
    const isActive = () =>
      opts.directory.value === directory && owner === opts.readSignal() && !owner.aborted && !signal?.aborted
    const retryPages = () => {
      if (!isActive()) return
      // A newer SSE summary invalidates HTTP rows even when no rows have been
      // displayed yet. Preserve the read obligation instead of consuming it.
      if (scopes.length) opts.invalidate(scopes)
    }
    try {
      const response = await opts.read(directory, scopes, signal)
      if (isActive() && isCurrent() && generation === opts.generation()) {
        opts.status.value = response
        await opts.pages.applySummaryPages(directory, response, scopes)
      } else retryPages()
    } catch (error) {
      if (isActive() && isCurrent() && generation === opts.generation()) throw error
      retryPages()
    } finally {
      finishFirstPages()
    }
  }

  return { loadSummary }
}
