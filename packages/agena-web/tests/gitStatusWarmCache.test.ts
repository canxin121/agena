import { describe, expect, test } from 'bun:test'
import { ref } from 'vue'

import { useGitStatusPaged } from '../src/composables/git/useGitStatusPaged'
import { recentGitStatusForDiff, rememberGitStatusForDiff } from '../src/pages/git/gitStatusWarmCache'
import type { GitStatusFile, GitStatusResponse, GitWatchStatusPayload } from '../src/types/git'

const file = (path: string): GitStatusFile => ({ path, index: ' ', workingDir: 'M' })
const status: GitStatusResponse = {
  current: 'master',
  tracking: null,
  ahead: 0,
  behind: 0,
  files: [],
  totalFiles: 3,
  stagedCount: 0,
  unstagedCount: 3,
  untrackedCount: 0,
  mergeCount: 0,
  offset: 0,
  limit: 2,
  hasMore: true,
  scope: 'summary',
}
const baseline: GitWatchStatusPayload = {
  current: 'master',
  tracking: null,
  ahead: 0,
  behind: 0,
  stagedCount: 0,
  unstagedCount: 3,
  untrackedCount: 0,
  mergeCount: 0,
  isClean: false,
  worktreeSignature: 'same-revision',
  scopeSignatures: { unstaged: 'same-pages' },
}

describe('Git diff sidebar warm handoff', () => {
  test('reuses only recent snapshots of the same repository', () => {
    const pages = useGitStatusPaged({
      gitReady: ref(true),
      status: ref(status),
      pageSize: 2,
      loadStatusPage: async () => status,
    })
    const snapshot = { status, pages: pages.snapshotPages(), watchBaseline: baseline }
    rememberGitStatusForDiff('/repo/one', snapshot, 100_000)

    expect(recentGitStatusForDiff('/repo/one', 100_100)).toEqual(snapshot)
    expect(recentGitStatusForDiff('/repo/two', 100_100)).toBeNull()
    expect(recentGitStatusForDiff('/repo/one', 115_001)).toBeNull()
  })

  test('restores loaded rows, pagination and snapshot cursors without another first-page read', async () => {
    const original = useGitStatusPaged({
      gitReady: ref(true),
      status: ref(status),
      pageSize: 2,
      loadStatusPage: async () => {
        throw new Error('source should not fetch')
      },
    })
    original.applyFirstPages(
      {
        unstaged: {
          files: [file('a.ts'), file('b.ts')],
          totalFiles: 3,
          offset: 0,
          limit: 2,
          hasMore: true,
        },
      },
      'snapshot-id',
    )

    const reads: Array<{ offset: number; snapshotId?: string }> = []
    const opened = useGitStatusPaged({
      gitReady: ref(true),
      status: ref(status),
      pageSize: 2,
      loadStatusPage: async ({ offset, snapshotId }) => {
        reads.push({ offset, snapshotId })
        return { ...status, files: [file('c.ts')], offset, snapshotId, hasMore: false }
      },
    })
    const snapshot = original.snapshotPages()
    opened.restorePages(snapshot)

    expect(opened.changesList.value.map((row) => row.path)).toEqual(['a.ts', 'b.ts'])
    expect(opened.hasMoreUnstaged.value).toBe(true)
    expect(reads).toHaveLength(0)

    // The newly opened tab can continue loading from the original cursor.
    await opened.loadMore('/repo/one', 'unstaged')
    expect(reads).toEqual([{ offset: 2, snapshotId: 'snapshot-id' }])
    expect(opened.changesList.value.map((row) => row.path)).toEqual(['a.ts', 'b.ts', 'c.ts'])
    expect(opened.hasMoreUnstaged.value).toBe(false)

    // New tabs must not mutate the source tab's rows through shared objects.
    opened.changesList.value[0]!.path = 'edited.ts'
    expect(original.changesList.value[0]!.path).toBe('a.ts')
  })
})
