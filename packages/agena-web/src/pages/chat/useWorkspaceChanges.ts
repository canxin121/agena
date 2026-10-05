import { computed, ref, shallowRef, watch, type Ref } from 'vue'
import { ApiError } from '@/lib/api'
import { gitJson } from '@/lib/gitApi'
import type { GitDiffResponse, GitStatusFile, GitStatusResponse } from '@/types/git'
import { useVisibleResource } from './useVisibleResource'

const PAGE_SIZE = 40
const DIFF_PAGE_BYTES = 256 * 1024
type Snapshot = { kind: 'repository'; status: GitStatusResponse } | { kind: 'not_repository' }

function hasStagedChange(file: GitStatusFile) {
  return Boolean(file.index.trim() && file.index !== '?')
}

export function useWorkspaceChanges(options: { directory: Ref<string>; busy: Ref<boolean>; request?: typeof gitJson }) {
  const request = options.request ?? gitJson
  const expanded = ref(false)
  const page = ref(0)
  const selected = shallowRef<GitStatusFile | null>(null)
  const staged = ref(false)
  const diffLimit = ref(DIFF_PAGE_BYTES)
  // Keep counts while switching between summary and file pages, but never
  // carry a snapshot or selection into a different workspace.
  const snapshot = shallowRef<Snapshot | null>(null)
  watch(
    options.directory,
    () => {
      snapshot.value = null
      selected.value = null
      expanded.value = false
      page.value = 0
    },
    { flush: 'sync' },
  )
  watch(
    page,
    () => {
      selected.value = null
    },
    { flush: 'sync' },
  )
  watch(
    [() => selected.value?.path, staged],
    () => {
      diffLimit.value = DIFF_PAGE_BYTES
    },
    { flush: 'sync' },
  )

  const status = useVisibleResource<Snapshot>({
    key: computed(() =>
      options.directory.value ? JSON.stringify([options.directory.value, expanded.value, page.value]) : '',
    ),
    interval: () => (options.busy.value || expanded.value ? 5000 : 30_000),
    async load(key, signal) {
      const [directory, open, index] = JSON.parse(key) as [string, boolean, number]
      try {
        return {
          kind: 'repository',
          status: await request<GitStatusResponse>(
            'status',
            directory,
            { summary: !open, offset: index * PAGE_SIZE, limit: PAGE_SIZE, includeDiffStats: false },
            { signal },
          ),
        }
      } catch (error) {
        if (error instanceof ApiError && error.code === 'not_git_repo') return { kind: 'not_repository' }
        throw error
      }
    },
  })
  const current = computed(() => (status.data.value?.kind === 'repository' ? status.data.value.status : null))
  const files = computed(() => (status.error.value ? [] : (current.value?.files ?? [])))
  const total = computed(() => (snapshot.value?.kind === 'repository' ? snapshot.value.status.totalFiles : null))
  const hasChanges = computed(() => (total.value ?? 0) > 0)
  const notRepository = computed(() => snapshot.value?.kind === 'not_repository')
  const statusPending = computed(
    () => Boolean(options.directory.value) && (status.loading.value || (!status.data.value && !status.error.value)),
  )
  const hasStaged = computed(() => Boolean(selected.value && hasStagedChange(selected.value)))
  const hasWorking = computed(() => Boolean(selected.value?.workingDir.trim()))

  watch(
    status.data,
    (value) => {
      if (!value) return
      snapshot.value = value
      if (value.kind === 'not_repository') {
        selected.value = null
        expanded.value = false
        page.value = 0
        return
      }
      if (value.status.totalFiles === 0) {
        selected.value = null
        expanded.value = false
        page.value = 0
        return
      }
      if (!expanded.value) return
      const data = value.status
      if (page.value && !data.files.length) {
        page.value = Math.max(0, Math.ceil(data.totalFiles / PAGE_SIZE) - 1)
        return
      }
      if (selected.value) {
        selected.value = data.files.find((file) => file.path === selected.value?.path) ?? null
        if (selected.value) {
          if (!hasStaged.value) staged.value = false
          else if (!hasWorking.value) staged.value = true
        }
      }
    },
    { flush: 'sync' },
  )

  const diffTarget = computed(() =>
    expanded.value && selected.value && !status.error.value && !notRepository.value
      ? [
          options.directory.value,
          selected.value.path,
          staged.value,
          (staged.value ? selected.value.indexOldPath : selected.value.workingOldPath) || null,
        ]
      : null,
  )
  const diffIdentity = computed(() => (diffTarget.value ? JSON.stringify(diffTarget.value) : ''))
  const diffKey = computed(() => (diffTarget.value ? JSON.stringify([...diffTarget.value, diffLimit.value]) : ''))
  // Keep the current preview and renderer expansion while fetching more of
  // the same diff. Changing the file, side or workspace clears it immediately.
  const visibleDiff = shallowRef<GitDiffResponse | null>(null)
  watch(
    diffIdentity,
    () => {
      visibleDiff.value = null
    },
    { flush: 'sync' },
  )
  const diff = useVisibleResource<GitDiffResponse>({
    key: diffKey,
    interval: () => (options.busy.value ? 5000 : 30_000),
    load(key, signal) {
      const [directory, path, index, oldPath, maxBytes] = JSON.parse(key) as [
        string,
        string,
        boolean,
        string | null,
        number,
      ]
      return request('diff', directory, { path, staged: index, contextLines: 3, maxBytes, oldPath }, { signal })
    },
  })
  const diffPending = computed(
    () => Boolean(diffKey.value) && (diff.loading.value || (!diff.data.value && !diff.error.value)),
  )
  watch(
    diff.data,
    (value) => {
      if (value) visibleDiff.value = value
    },
    { flush: 'sync' },
  )

  function select(file: GitStatusFile) {
    selected.value = selected.value?.path === file.path ? null : file
    staged.value = hasStagedChange(file) && !file.workingDir.trim()
  }
  function refresh() {
    void status.refresh()
    void diff.refresh()
  }
  function moreDiff() {
    // The budget belongs to the request identity. Changing it cancels an old
    // read, so a click during throttling or an in-flight request is not lost.
    diffLimit.value += DIFF_PAGE_BYTES
  }

  return {
    expanded,
    page,
    selected,
    staged,
    status,
    current,
    files,
    total,
    hasChanges,
    notRepository,
    statusPending,
    hasStaged,
    hasWorking,
    diff,
    diffKey,
    diffIdentity,
    visibleDiff,
    diffPending,
    select,
    refresh,
    moreDiff,
  }
}
