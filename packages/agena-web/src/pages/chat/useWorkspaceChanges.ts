import { computed, ref, shallowRef, watch, type Ref } from 'vue'
import { apiJson } from '@/lib/api'
import { conditionalJsonObserved } from '@/lib/conditionalJson'
import { captureResourceObservation } from '@/lib/resourceSync'
import type { SessionFileChange, SessionFileChanges } from '@/types/sessionFileChanges'
import { useVisibleResource } from './useVisibleResource'

const PAGE_SIZE = 40
const DIFF_PAGE_BYTES = 256 * 1024

// The name remains for the dock's callers; its source is durable session facts,
// independent of workspace/Git and of how much transcript the client loaded.
export function useWorkspaceChanges(options: { sessionId: Ref<string>; busy: Ref<boolean>; request?: typeof apiJson }) {
  const request = async (
    resource: string,
    url: string,
    signal: AbortSignal,
    force: boolean,
    observe: (observation: ReturnType<typeof captureResourceObservation>) => void,
  ) => {
    if (options.request) return options.request<SessionFileChanges>(url, { signal })
    const { value, observation } = await conditionalJsonObserved<SessionFileChanges>(resource, url, { signal }, force)
    observe(observation)
    return value
  }
  const expanded = ref(false)
  const page = ref(0)
  const selected = shallowRef<SessionFileChange | null>(null)
  const diffLimit = ref(DIFF_PAGE_BYTES)
  const snapshot = shallowRef<SessionFileChanges | null>(null)
  watch(
    options.sessionId,
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
    () => selected.value?.path,
    () => {
      diffLimit.value = DIFF_PAGE_BYTES
    },
    { flush: 'sync' },
  )

  const status = useVisibleResource<SessionFileChanges>({
    key: computed(() =>
      options.sessionId.value ? JSON.stringify([options.sessionId.value, expanded.value, page.value]) : '',
    ),
    resource: (key) => `session:${(JSON.parse(key) as string[])[0]}:files`,
    interval: () => null,
    load(key, signal, _previous, force, observe) {
      const [id, open, index] = JSON.parse(key) as [string, boolean, number]
      return request(
        `session:${id}:files`,
        `/api/v1/sessions/${encodeURIComponent(id)}/file-changes?summary=${!open}&offset=${index * PAGE_SIZE}&limit=${PAGE_SIZE}`,
        signal,
        force,
        observe,
      )
    },
  })
  const current = status.data
  const files = computed(() => (status.error.value ? [] : (current.value?.files ?? [])))
  const total = computed(() => snapshot.value?.total_files ?? null)
  const recordingIncomplete = computed(() => snapshot.value?.recording_incomplete ?? false)
  const hasChanges = computed(() => (total.value ?? 0) > 0 || recordingIncomplete.value)
  const statusPending = computed(
    () => Boolean(options.sessionId.value) && (status.loading.value || (!current.value && !status.error.value)),
  )
  watch(
    status.data,
    (value) => {
      if (!value) return
      snapshot.value = value
      if (!value.total_files && !value.recording_incomplete) {
        selected.value = null
        expanded.value = false
        page.value = 0
        return
      }
      if (!expanded.value) return
      if (page.value && !value.files.length) {
        page.value = Math.max(0, Math.ceil(value.total_files / PAGE_SIZE) - 1)
        return
      }
      if (selected.value) selected.value = value.files.find((file) => file.path === selected.value?.path) ?? null
    },
    { flush: 'sync' },
  )

  const diffIdentity = computed(() =>
    expanded.value && selected.value && !status.error.value
      ? JSON.stringify([options.sessionId.value, selected.value.path])
      : '',
  )
  const diffKey = computed(() =>
    diffIdentity.value
      ? JSON.stringify([...JSON.parse(diffIdentity.value), diffLimit.value, selected.value?.revision ?? ''])
      : '',
  )
  const visibleDiff = shallowRef<SessionFileChanges | null>(null)
  watch(
    diffIdentity,
    () => {
      visibleDiff.value = null
    },
    { flush: 'sync' },
  )
  let retainedDiff: { key: string; scope: number; value: SessionFileChanges } | null = null
  const diff = useVisibleResource<SessionFileChanges>({
    key: diffKey,
    // The list carries a per-file fingerprint. Only a changed selected row
    // should reload its diff; legacy responses still use the broad clock.
    resource: (key) => {
      const [id, , , revision] = JSON.parse(key) as [string, string, number, string]
      return revision ? undefined : `session:${id}:files`
    },
    interval: () => null,
    async load(key, signal, _previous, force, observe) {
      const [id, path, maxBytes, revision] = JSON.parse(key) as [string, string, number, string]
      const scope = captureResourceObservation(`session:${id}:files`).scope
      if (revision && retainedDiff?.key === key && retainedDiff.scope === scope && !force) return retainedDiff.value
      const query = new URLSearchParams({ path, max_bytes: String(maxBytes) })
      const value = await request(
        `session:${id}:files`,
        `/api/v1/sessions/${encodeURIComponent(id)}/file-changes?${query}`,
        signal,
        force,
        observe,
      )
      signal.throwIfAborted()
      // One selected diff is retained, bounded by the existing 2 MiB limit.
      if (revision && scope === captureResourceObservation(`session:${id}:files`).scope)
        retainedDiff = { key, scope, value }
      return value
    },
  })
  watch(
    diff.data,
    (value) => {
      if (value) visibleDiff.value = value
    },
    { flush: 'sync' },
  )
  const diffPending = computed(
    () => Boolean(diffKey.value) && (diff.loading.value || (!diff.data.value && !diff.error.value)),
  )
  function select(file: SessionFileChange) {
    selected.value = selected.value?.path === file.path ? null : file
  }
  function refresh() {
    void status.refresh()
    void diff.refresh()
  }
  function moreDiff() {
    diffLimit.value = Math.min(2 * 1024 * 1024, diffLimit.value + DIFF_PAGE_BYTES)
  }
  return {
    expanded,
    page,
    selected,
    status,
    current,
    files,
    total,
    hasChanges,
    recordingIncomplete,
    statusPending,
    diff,
    diffKey,
    diffIdentity,
    visibleDiff,
    diffPending,
    select,
    refresh,
    moreDiff,
    moreDiffAvailable: computed(() => diffLimit.value < 2 * 1024 * 1024),
  }
}
