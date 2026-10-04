import { onScopeDispose, ref, watch } from 'vue'
import type { BtwAnswer } from './btwRequest'

type Ask = (
  sessionId: string,
  question: string,
  signal: AbortSignal,
  update: (answer: BtwAnswer) => void,
) => Promise<void>

export function useBtw(scope: () => readonly [boolean, string | null], ask: Ask) {
  const loading = ref(false)
  const markdown = ref('')
  const error = ref('')
  let generation = 0
  let request: AbortController | undefined

  function stop() {
    generation += 1
    request?.abort()
    request = undefined
    loading.value = false
  }

  async function submit(question: string) {
    const [open, sessionId] = scope()
    if (!open || !sessionId || !question.trim() || loading.value) return
    stop()
    const owner = generation
    const controller = new AbortController()
    request = controller
    loading.value = true
    markdown.value = error.value = ''
    try {
      await ask(sessionId, question.trim(), controller.signal, (answer) => {
        if (owner !== generation) return
        if (answer.text || !answer.error) markdown.value = answer.text
        error.value = answer.error || ''
      })
    } catch (reason) {
      if (owner === generation) error.value = reason instanceof Error ? reason.message : String(reason)
    } finally {
      if (owner === generation) {
        loading.value = false
        request = undefined
      }
    }
  }

  watch(
    scope,
    () => {
      stop()
      markdown.value = error.value = ''
    },
    { flush: 'sync' },
  )
  onScopeDispose(stop)
  return { loading, markdown, error, submit, stop }
}
