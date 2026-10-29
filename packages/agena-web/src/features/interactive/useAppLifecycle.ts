import { onScopeDispose, ref } from 'vue'

/** Stop may supersede an in-flight Run; its late completion cannot own the UI. */
export function useAppLifecycle() {
  const phase = ref<'' | 'run' | 'stop'>('')
  let current: AbortController | undefined

  function cancel() {
    current?.abort()
    current = undefined
    phase.value = ''
  }

  function begin(next: 'run' | 'stop') {
    if (phase.value && !(next === 'stop' && phase.value === 'run')) return undefined
    current?.abort()
    const controller = new AbortController()
    current = controller
    phase.value = next
    const isCurrent = () => current === controller && !controller.signal.aborted
    return {
      signal: controller.signal,
      isCurrent,
      finish: () => {
        if (current !== controller) return
        current = undefined
        phase.value = ''
      },
    }
  }

  onScopeDispose(cancel)
  return { phase, begin, cancel }
}
