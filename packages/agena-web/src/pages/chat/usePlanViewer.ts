import { onScopeDispose, ref, watch } from 'vue'
import type { JsonValue } from '@/types/json'
import type { PlanTool, PlanToolInput } from './planViewerRequest'
import { planDocument, readPlanSnapshot, type PlanSnapshot } from './planSnapshot'

type PlanResponse = Record<string, JsonValue>
type InvokePlan = (
  sessionId: string | null,
  tool: PlanTool,
  input: PlanToolInput,
  signal: AbortSignal,
) => Promise<PlanResponse>

/** Each opening/session owns its requests; completions cannot change another plan. */
export function usePlanViewer(scope: () => readonly [boolean, string | null], invoke: InvokePlan) {
  const loading = ref(false)
  const toggling = ref(false)
  const markdown = ref('')
  const error = ref('')
  const autorun = ref<boolean | null>(null)
  const snapshot = ref<PlanSnapshot | null>(null)
  let generation = 0
  let read: AbortController | undefined
  let write: AbortController | undefined

  async function refresh() {
    if (!scope()[0] || loading.value || toggling.value) return
    const owner = generation
    const sessionId = scope()[1]
    const controller = new AbortController()
    read = controller
    loading.value = true
    error.value = ''
    const timeout = setTimeout(() => controller.abort(), 30_000)
    try {
      const response = await invoke(sessionId, 'get', { view: 'full' }, controller.signal)
      if (owner !== generation || read !== controller || controller.signal.aborted) return
      snapshot.value = readPlanSnapshot(response)
      markdown.value = snapshot.value ? planDocument(response) : ''
      autorun.value = snapshot.value?.autorun ?? null
    } catch (reason) {
      if (owner === generation && read === controller && !controller.signal.aborted)
        error.value = reason instanceof Error ? reason.message : String(reason)
    } finally {
      clearTimeout(timeout)
      if (owner === generation && read === controller) {
        loading.value = false
        read = undefined
      }
    }
  }

  async function toggleAutorun() {
    if (!scope()[0] || autorun.value === null || toggling.value || loading.value) return
    const owner = generation
    const sessionId = scope()[1]
    const controller = new AbortController()
    write = controller
    toggling.value = true
    error.value = ''
    const timeout = setTimeout(() => controller.abort(), 30_000)
    try {
      await invoke(sessionId, 'phase', { autorun: !autorun.value }, controller.signal)
      if (owner !== generation || controller.signal.aborted) return
      toggling.value = false
      await refresh()
    } catch (reason) {
      if (owner === generation) error.value = reason instanceof Error ? reason.message : String(reason)
    } finally {
      clearTimeout(timeout)
      if (owner === generation) {
        toggling.value = false
        write = undefined
      }
    }
  }

  function invalidate() {
    generation += 1
    read?.abort()
    write?.abort()
    read = write = undefined
    loading.value = toggling.value = false
    markdown.value = error.value = ''
    autorun.value = null
    snapshot.value = null
  }

  function pauseRead() {
    read?.abort()
    read = undefined
    loading.value = false
  }

  watch(
    scope,
    ([open]) => {
      invalidate()
      if (open) void refresh()
    },
    { immediate: true, flush: 'sync' },
  )
  onScopeDispose(invalidate)
  return { loading, toggling, markdown, error, autorun, snapshot, refresh, toggleAutorun, pauseRead }
}
