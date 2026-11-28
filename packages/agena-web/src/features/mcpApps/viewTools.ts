import { ApiError } from '@/lib/api'
import {
  callMcpAppServer,
  closeMcpAppView,
  openMcpAppView,
  pollMcpAppView,
  replyToMcpAppView,
  updateMcpAppView,
  type McpAppViewCall,
  type McpAppViewInputMethod,
  type McpAppServerRequest,
  type McpAppViewPrompt,
  type McpAppViewReply,
  type McpAppViewSource,
  type McpAppViewTaskMethod,
  type McpAppViewTool,
} from './api'
import { mcpAppViewRequest, type McpAppGuestResponse, type RpcId } from './bridge'
import { record } from './resources'

const TOOL_LIMIT = 50
const encoder = new TextEncoder()

function viewTool(value: unknown): McpAppViewTool {
  if (!record(value) || typeof value.name !== 'string' || !record(value.inputSchema)) {
    throw new Error('Invalid MCP App tool definition.')
  }
  if (value.outputSchema !== undefined && !record(value.outputSchema)) {
    throw new Error('Invalid MCP App tool output schema.')
  }
  const taskSupport = record(value.execution) ? value.execution.taskSupport : undefined
  return {
    name: value.name,
    inputSchema: value.inputSchema,
    outputSchema: value.outputSchema,
    title: typeof value.title === 'string' ? value.title : undefined,
    description: typeof value.description === 'string' ? value.description : undefined,
    execution:
      taskSupport === 'optional' || taskSupport === 'required' || taskSupport === 'forbidden'
        ? { taskSupport }
        : undefined,
  }
}

function retryable(error: unknown) {
  if (error instanceof ApiError) return error.status === 429 || error.status >= 500
  return error instanceof TypeError || (error instanceof DOMException && error.name === 'TimeoutError')
}

function pause(ms: number, signal: AbortSignal) {
  return new Promise<void>((resolve) => {
    const finish = () => {
      clearTimeout(timer)
      signal.removeEventListener('abort', finish)
      resolve()
    }
    const timer = setTimeout(finish, ms)
    if (signal.aborted) finish()
    else signal.addEventListener('abort', finish, { once: true })
  })
}

export function exposeMcpAppViewTools(
  source: McpAppViewSource,
  send: (message: Record<string, unknown>) => void,
  sameBackend: () => boolean,
  publishesTools: boolean,
  taskExtension: boolean,
  askUser?: (request: McpAppViewPrompt, signal: AbortSignal) => Promise<string | null>,
) {
  const lifetime = new AbortController()
  const pending = new Map<RpcId, (reply: McpAppGuestResponse) => void>()
  const calls = new Map<string, AbortController>()
  const posting = new Map<AbortController, Promise<void>>()
  const current = () => !lifetime.signal.aborted && sameBackend()
  let sequence = 0
  let view: string | undefined
  let opening: Promise<string> | undefined
  let timer: ReturnType<typeof setTimeout> | undefined
  let refreshing = false
  let refreshPending = false

  function request(
    method: 'tools/list' | 'tools/call' | McpAppViewTaskMethod | McpAppViewInputMethod,
    params: Record<string, unknown>,
    cancellation = lifetime.signal,
  ) {
    const timeout = method === 'tools/list' ? [AbortSignal.timeout(5000)] : []
    // The runtime holds the absolute deadline for each published call and task.
    const signal = AbortSignal.any([lifetime.signal, cancellation, ...timeout])
    return new Promise<McpAppGuestResponse>((resolve, reject) => {
      if (!current() || signal.aborted) {
        reject(new Error('The MCP App view is unavailable.'))
        return
      }
      const id = `agena-view-${++sequence}`
      const abort = () => {
        if (pending.delete(id) && current()) {
          send({ method: 'notifications/cancelled', params: { requestId: id } })
        }
        reject(signal.reason)
      }
      pending.set(id, (reply) => {
        pending.delete(id)
        signal.removeEventListener('abort', abort)
        resolve(reply)
      })
      signal.addEventListener('abort', abort, { once: true })
      send({ id, method, params })
    })
  }

  async function catalog() {
    const entries = new Map<string, McpAppViewTool>()
    const seenCursors = new Set<string>()
    let cursor: string | undefined
    for (let page = 0; page < TOOL_LIMIT; page++) {
      const answer = await request('tools/list', cursor === undefined ? {} : { cursor })
      if ('error' in answer || !Array.isArray(answer.result.tools)) {
        throw new Error('The MCP App did not return a tool catalog.')
      }
      for (const item of answer.result.tools as unknown[]) {
        const definition = viewTool(item)
        if (entries.size >= TOOL_LIMIT || entries.has(definition.name)) {
          throw new Error('The MCP App tool catalog is too large or contains duplicate names.')
        }
        entries.set(definition.name, definition)
      }
      const next = answer.result.nextCursor
      if (next === undefined) {
        const tools = [...entries.values()]
        if (encoder.encode(JSON.stringify({ tools })).byteLength > 128 * 1024) {
          throw new Error('The MCP App tool catalog exceeds its size limit.')
        }
        return tools
      }
      if (typeof next !== 'string' || !next || next.length > 2048 || seenCursors.has(next)) {
        throw new Error('The MCP App returned an invalid tool cursor.')
      }
      seenCursors.add(next)
      cursor = next
    }
    throw new Error('The MCP App tool catalog exceeds its page limit.')
  }

  function close() {
    if (timer !== undefined) clearTimeout(timer)
    const openView = view
    view = undefined
    const received = [...posting]
    for (const call of calls.values()) {
      if (!posting.has(call)) call.abort()
    }
    lifetime.abort()
    calls.clear()
    if (!openView) return
    void (async () => {
      // A guest can return a server task and immediately request teardown.
      // Let replies already received from the document reach the host first.
      const drain = new AbortController()
      await Promise.race([Promise.allSettled(received.map(([, reply]) => reply)), pause(2000, drain.signal)])
      drain.abort()
      for (const [controller] of received) controller.abort()
      if (sameBackend()) await closeMcpAppView(source, openView).catch(() => {})
    })()
  }

  async function deliver(view: string, call: McpAppViewCall, controller: AbortController) {
    const signal = controller.signal
    let reply: McpAppViewReply
    try {
      if ('prompt' in call) {
        reply = { status: 'input', answer: askUser ? await askUser(call, signal) : null }
      } else {
        const { method, params } = mcpAppViewRequest(call)
        const answer = await request(method, params, signal)
        reply =
          'result' in answer
            ? { status: 'result', result: answer.result }
            : { status: 'error', message: answer.error.message }
      }
    } catch {
      if (!current() || signal.aborted) return
      reply = { status: 'error', message: 'The MCP App did not answer the tool request.' }
    }
    if (!current() || signal.aborted) return
    const postingReply = submitReply(view, call.id, reply, signal)
    posting.set(controller, postingReply)
    try {
      await postingReply
    } finally {
      posting.delete(controller)
    }
  }

  async function submitReply(view: string, id: string, reply: McpAppViewReply, signal: AbortSignal) {
    for (const delay of [0, 250, 1000]) {
      if (delay) await pause(delay, signal)
      if (!sameBackend() || signal.aborted) return
      try {
        await replyToMcpAppView(source, view, id, reply, AbortSignal.any([signal, AbortSignal.timeout(7000)]))
        return
      } catch (error) {
        if (!retryable(error)) return
      }
    }
  }

  function ensureView(tools: McpAppViewTool[] = []): Promise<string> {
    if (!current()) return Promise.reject(new Error('The MCP App view is unavailable.'))
    if (view) return Promise.resolve(view)
    if (!opening) {
      const signal = AbortSignal.any([lifetime.signal, AbortSignal.timeout(24_000)])
      opening = openMcpAppView(source, tools, taskExtension, signal).then(({ view: opened }) => {
        if (!current()) {
          if (sameBackend()) void closeMcpAppView(source, opened).catch(() => {})
          throw new Error('The MCP App view is unavailable.')
        }
        view = opened
        void poll(opened)
        return opened
      }).finally(() => { opening = undefined })
    }
    return opening
  }

  async function poll(openView: string) {
    let after: number | undefined
    while (current() && view === openView) {
      try {
        const snapshot = await pollMcpAppView(
          source,
          openView,
          after,
          AbortSignal.any([lifetime.signal, AbortSignal.timeout(24_000)]),
        )
        if (!current() || view !== openView) return
        after = snapshot.revision
        const active = new Set(snapshot.calls.map((call) => call.id))
        for (const [id, controller] of calls) {
          if (!active.has(id)) {
            calls.delete(id)
            controller.abort()
          }
        }
        for (const call of snapshot.calls) {
          if (calls.has(call.id)) continue
          const controller = new AbortController()
          calls.set(call.id, controller)
          void deliver(openView, call, controller)
        }
      } catch (error) {
        if (!current()) return
        if (!retryable(error)) {
          close()
          return
        }
        await pause(1000, lifetime.signal)
      }
    }
  }

  function refresh(delay = 120) {
    refreshPending = true
    if (!current() || refreshing || timer !== undefined) return
    timer = setTimeout(async () => {
      timer = undefined
      if (!current()) return
      refreshing = true
      refreshPending = false
      let nextDelay = 120
      try {
        const tools = await catalog()
        if (!current()) return
        const signal = AbortSignal.any([lifetime.signal, AbortSignal.timeout(24_000)])
        if (view || opening) {
          await updateMcpAppView(source, await ensureView(), tools, signal)
        } else {
          await ensureView(tools)
        }
      } catch (error) {
        if (retryable(error) && current()) {
          refreshPending = true
          nextDelay = 1000
        } else if (view && current()) {
          try {
            await updateMcpAppView(source, view, [], AbortSignal.any([lifetime.signal, AbortSignal.timeout(7000)]))
          } catch {
            close()
          }
        }
      } finally {
        refreshing = false
        if (refreshPending) refresh(nextDelay)
      }
    }, delay)
  }

  if (publishesTools) refresh(0)
  return {
    refresh,
    receive(reply: McpAppGuestResponse) {
      pending.get(reply.id)?.(reply)
    },
    async requestServer(request: McpAppServerRequest, signal: AbortSignal) {
      const opened = await ensureView()
      return callMcpAppServer(source, opened, request, signal)
    },
    close,
  }
}
