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
  pending: boolean
  error: string
}

export const BTW_HISTORY_LIMIT = 20
const SESSION_LIMIT = 32

/** Owned by the store, not by a mounted panel or the selected session. */
export function useBtw(ask: Ask) {
  const sessions = reactive(new Map<string, BtwConversation>())
  const requests = new Map<string, { controller: AbortController; question: string; exchange: BtwExchange | null }>()
  let nextId = 0

  function ensure(sessionId: string) {
    let state = sessions.get(sessionId)
    if (!state) {
      // Keep active requests and their history; prune only inactive sessions.
      for (const [id, entry] of sessions) {
        if (sessions.size < SESSION_LIMIT) break
        if (!requests.has(id) && !entry.draft.trim()) sessions.delete(id)
      }
      state = reactive({ draft: '', expanded: true, focusRequest: 0, exchanges: [], pending: false, error: '' })
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
    const state = sessions.get(sessionId)
    if (state) {
      state.pending = false
      if (!request.exchange) restoreQuestion(state, request.question)
    }
    if (request.exchange) request.exchange.status = 'stopped'
    request.controller.abort()
  }

  function restoreQuestion(state: BtwConversation, question: string) {
    state.draft = state.draft && state.draft !== question ? `${question}\n\n${state.draft}` : question
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
    state.draft = ''
    state.expanded = true
    state.pending = true
    state.error = ''
    const request: { controller: AbortController; question: string; exchange: BtwExchange | null } = {
      controller: new AbortController(),
      question,
      exchange: null,
    }
    requests.set(sessionId, request)
    const isCurrent = () => requests.get(sessionId) === request
    try {
      await ask(sessionId, question, request.controller.signal, (answer) => {
        if (!isCurrent()) return
        if (!request.exchange && (typeof answer.question === 'string' || answer.text)) {
          request.exchange = reactive<BtwExchange>({
            id: ++nextId,
            question: answer.question || '',
            markdown: '',
            error: '',
            status: 'running',
            expanded: true,
          })
          state.exchanges.push(request.exchange)
          state.exchanges.splice(0, Math.max(0, state.exchanges.length - BTW_HISTORY_LIMIT))
        }
        const exchange = request.exchange
        if (exchange) {
          if (typeof answer.question === 'string') exchange.question = answer.question
          if (answer.text || !answer.error) exchange.markdown = answer.text
          exchange.error = answer.error || ''
        } else state.error = answer.error || ''
      })
      if (isCurrent() && request.exchange) request.exchange.status = request.exchange.error ? 'failed' : 'complete'
    } catch (reason) {
      if (isCurrent()) {
        const message = reason instanceof Error ? reason.message : String(reason)
        if (request.exchange) {
          request.exchange.error = message
          request.exchange.status = 'failed'
        } else state.error = message
      }
    } finally {
      if (isCurrent()) {
        state.pending = false
        if (!request.exchange) restoreQuestion(state, question)
        requests.delete(sessionId)
      }
    }
  }

  onScopeDispose(() => {
    for (const id of requests.keys()) stop(id)
    sessions.clear()
  })
  return { sessions, open, submit, stop, clear }
}
