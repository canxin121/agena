import { onScopeDispose, reactive } from 'vue'
import type { BtwAnswer } from './btwRequest'

type Ask = (
  sessionId: string,
  question: string,
  signal: AbortSignal,
  update: (answer: BtwAnswer) => void,
) => Promise<void>

export type BtwExchange = {
  id: number
  question: string
  markdown: string
  error: string
  status: 'running' | 'complete' | 'stopped' | 'failed'
  expanded: boolean
}

export type BtwConversation = {
  draft: string
  expanded: boolean
  focusRequest: number
  exchanges: BtwExchange[]
}

export const BTW_HISTORY_LIMIT = 20
const SESSION_LIMIT = 32

/** Owned by the store, not by a mounted panel or the selected session. */
export function useBtw(ask: Ask) {
  const sessions = reactive(new Map<string, BtwConversation>())
  const requests = new Map<string, { controller: AbortController; exchange: BtwExchange }>()
  let nextId = 0

  function ensure(sessionId: string) {
    let state = sessions.get(sessionId)
    if (!state) {
      // Keep active requests and their history; prune only inactive sessions.
      for (const [id, entry] of sessions) {
        if (sessions.size < SESSION_LIMIT) break
        if (!requests.has(id) && !entry.draft.trim()) sessions.delete(id)
      }
      state = reactive({ draft: '', expanded: true, focusRequest: 0, exchanges: [] })
    }
    sessions.delete(sessionId)
    sessions.set(sessionId, state)
    return state
  }

  function open(sessionId: string, question = '') {
    const state = ensure(sessionId)
    state.expanded = true
    if (question.trim()) {
      state.draft = question
      void submit(sessionId)
    } else {
      state.focusRequest++
    }
  }

  function stop(sessionId: string) {
    const request = requests.get(sessionId)
    if (!request) return
    requests.delete(sessionId)
    request.exchange.status = 'stopped'
    request.controller.abort()
  }

  function clear(sessionId: string) {
    stop(sessionId)
    sessions.delete(sessionId)
  }

  async function submit(sessionId: string) {
    const state = sessions.get(sessionId)
    const question = state?.draft.trim()
    if (!state || !question || requests.has(sessionId)) return
    for (const exchange of state.exchanges) exchange.expanded = false
    const exchange = reactive<BtwExchange>({
      id: ++nextId,
      question,
      markdown: '',
      error: '',
      status: 'running',
      expanded: true,
    })
    state.exchanges.push(exchange)
    state.exchanges.splice(0, Math.max(0, state.exchanges.length - BTW_HISTORY_LIMIT))
    state.draft = ''
    state.expanded = true
    const request = { controller: new AbortController(), exchange }
    requests.set(sessionId, request)
    const isCurrent = () => requests.get(sessionId) === request
    try {
      await ask(sessionId, question, request.controller.signal, (answer) => {
        if (!isCurrent()) return
        if (answer.text || !answer.error) exchange.markdown = answer.text
        exchange.error = answer.error || ''
      })
      if (isCurrent()) exchange.status = exchange.error ? 'failed' : 'complete'
    } catch (reason) {
      if (isCurrent()) {
        exchange.error = reason instanceof Error ? reason.message : String(reason)
        exchange.status = 'failed'
      }
    } finally {
      if (isCurrent()) requests.delete(sessionId)
    }
  }

  onScopeDispose(() => {
    for (const id of requests.keys()) stop(id)
    sessions.clear()
  })
  return { sessions, open, submit, stop, clear }
}
