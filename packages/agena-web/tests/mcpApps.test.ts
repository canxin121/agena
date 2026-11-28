import { describe, expect, test } from 'bun:test'
import { runInNewContext } from 'node:vm'
import { mcpAppContainerDimensions, mcpAppStyleVariables } from '../src/features/mcpApps/hostContext'
import { MAX_MCP_APP_MEDIA_BYTES, resolveMcpAppMessageContent } from '../src/features/mcpApps/media'
import { exposeMcpAppViewTools } from '../src/features/mcpApps/viewTools'
import {
  MCP_APP_TEARDOWN_ID,
  MCP_TASKS_EXTENSION,
  mcpAppAction,
  mcpAppBrowserPolicy,
  mcpAppGuestMessage,
  mcpAppGuestResponse,
  mcpAppHeight,
  mcpAppLog,
  mcpAppModes,
  mcpAppProgressToken,
  mcpAppProxyDocument,
  mcpAppProxyUrl,
  mcpAppRequest,
  mcpAppResourceDownload,
  mcpAppServerRequest,
  mcpAppTaskExtension,
  mcpAppTeardownResponse,
  mcpAppViewRequest,
} from '../src/features/mcpApps/bridge'

const frame = {} as Window
const request = { jsonrpc: '2.0', id: 1, method: 'tools/call', params: { name: 'refresh', arguments: { page: 2 } } }
const fromApp = (data: unknown, origin = 'null', source: Window = frame) =>
  mcpAppGuestMessage({ data, origin, source }, frame)

const imageBytes = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/lRkAAAAASUVORK5CYII=',
  'base64',
)
const image = { type: 'image', mimeType: 'image/png', data: imageBytes.toString('base64') }
const audio = { type: 'audio', mimeType: 'audio/wav', data: Buffer.from('RIFF0000WAVE').toString('base64') }

describe('MCP Apps bridge', () => {
  test('describes inline height as flexible and overlay dimensions as fixed', () => {
    expect(mcpAppContainerDimensions('inline', 420, 280)).toEqual({ width: 420, maxHeight: 1200 })
    expect(mcpAppContainerDimensions('fullscreen', 720, 540)).toEqual({ width: 720, height: 540 })
    expect(mcpAppContainerDimensions('pip', 320, 172)).toEqual({ width: 320, height: 172 })
    expect(mcpAppContainerDimensions('pip', Infinity, 0)).toEqual({})
  })

  test('projects the active Agena theme to standardized App style variables', () => {
    const theme = new Map([
      ['--background', '0.97 0.02 85'],
      ['--foreground', '0.25 0.02 40'],
      ['--border', '0.85 0.02 70'],
      ['--font-sans', ' IBM Plex Sans, sans-serif '],
      ['--radius', '0.625rem'],
      ['--unknown-token', 'value'],
    ])
    expect(mcpAppStyleVariables({ getPropertyValue: (name) => theme.get(name) ?? '' })).toEqual({
      '--color-background-primary': 'oklch(0.97 0.02 85)',
      '--color-background-inverse': 'oklch(0.25 0.02 40)',
      '--color-text-primary': 'oklch(0.25 0.02 40)',
      '--color-text-inverse': 'oklch(0.97 0.02 85)',
      '--color-border-primary': 'oklch(0.85 0.02 70)',
      '--font-sans': 'IBM Plex Sans, sans-serif',
      '--border-radius-md': '0.625rem',
    })
  })

  test('only accepts bounded RPC from the opaque sandbox window', () => {
    expect(fromApp(request)?.method).toBe('tools/call')
    expect(fromApp(request, 'https://other.test')).toBeNull()
    expect(fromApp(request, 'null', {} as Window)).toBeNull()
    expect(fromApp({ ...request, params: { arguments: { giant: 'x'.repeat(128 * 1024) } } })).toBeNull()
    expect(fromApp({ ...request, id: { unexpected: true } })).toBeNull()
    expect(fromApp({ ...request, params: [] })).toBeNull()
  })

  test('narrows guest messages to same-connection methods and fields', () => {
    expect(mcpAppRequest(fromApp(request)!)).toEqual({ method: request.method, params: request.params })
    expect(
      mcpAppRequest(fromApp({ ...request, params: { ...request.params, server: 'different-server' } })!),
    ).toBeNull()
    expect(mcpAppRequest(fromApp({ ...request, method: 'ui/message' })!)).toBeNull()
    const list = (params: unknown) => mcpAppRequest(fromApp({ ...request, method: 'tools/list', params })!)
    expect(list({ cursor: '50', _meta: {} })).toEqual({ method: 'tools/list', params: { cursor: '50' } })
    expect(list({ cursor: 50 })).toBeNull()
    expect(list({ server: 'different-server' })).toBeNull()
    for (const method of ['resources/list', 'resources/templates/list']) {
      expect(mcpAppRequest(fromApp({ ...request, method, params: { cursor: 'opaque page', _meta: {} } })!)).toEqual({
        method,
        params: { cursor: 'opaque page' },
      })
      expect(mcpAppRequest(fromApp({ ...request, method, params: { server: 'different-server' } })!)).toBeNull()
    }
  })

  test('accepts bounded MCP logging notifications from the App', () => {
    const log = {
      jsonrpc: '2.0',
      method: 'notifications/message',
      params: { level: 'warning', logger: 'chart', data: { value: 3 } },
    }
    const parsed = (data: unknown) => mcpAppLog(fromApp(data)!)
    expect(parsed(log)).toEqual({ level: 'warn', logger: 'chart', data: { value: 3 } })
    expect(parsed({ ...log, id: 1 })).toBeNull()
    expect(parsed({ ...log, params: { ...log.params, level: 'custom' } })).toBeNull()
    expect(parsed({ ...log, params: { ...log.params, data: 'x'.repeat(4096) } })).toBeNull()
  })

  test('negotiates task polling with an App that declares the extension', () => {
    expect(mcpAppTaskExtension({ experimental: { [MCP_TASKS_EXTENSION]: {} } })).toBe(true)
    expect(mcpAppTaskExtension({ extensions: { [MCP_TASKS_EXTENSION]: {} } })).toBe(true)
    expect(mcpAppTaskExtension({ experimental: { [MCP_TASKS_EXTENSION]: true } })).toBe(false)
    expect(mcpAppTaskExtension({ tools: {} })).toBe(false)
  })

  test('forwards host task operations to the published view tool', () => {
    const tool = { id: 'rest-start', name: 'calculate', arguments: { value: 7 } }
    expect(mcpAppViewRequest(tool)).toEqual({
      method: 'tools/call',
      params: { name: 'calculate', arguments: { value: 7 } },
    })
    expect(mcpAppViewRequest({ ...tool, task: {} }).params).toEqual({
      name: 'calculate', arguments: { value: 7 }, task: {},
    })
    const inputResponses = { approval: { action: 'accept', content: { confirmed: true } } }
    expect(mcpAppViewRequest({ ...tool, id: 'rest-retry', requestState: ' state / α ', inputResponses })).toEqual({
      method: 'tools/call',
      params: { name: 'calculate', arguments: { value: 7 }, requestState: ' state / α ', inputResponses },
    })
    expect(mcpAppViewRequest({ ...tool, requestState: '' }).params).toEqual({
      name: 'calculate', arguments: { value: 7 }, requestState: '',
    })
    for (const method of ['tasks/get', 'tasks/result', 'tasks/cancel'] as const) {
      expect(mcpAppViewRequest({ id: 'rest-operation', method, taskId: 'owned-task' })).toEqual({
        method, params: { taskId: 'owned-task' },
      })
      expect(mcpAppRequest(fromApp({ ...request, method, params: { taskId: 'other-task' } })!)).toBeNull()
    }
    const update = { method: 'tasks/update' as const, taskId: 'owned-task', inputResponses }
    expect(mcpAppViewRequest({ id: 'rest-update', ...update })).toEqual({
      method: 'tasks/update', params: { taskId: 'owned-task', inputResponses },
    })
    expect(mcpAppRequest(fromApp({ ...request, method: update.method, params: { taskId: 'other-task', inputResponses } })!)).toBeNull()
    const params = {
      mode: 'form',
      message: 'Confirm the App task',
      requestedSchema: { type: 'object' },
      _meta: { 'io.modelcontextprotocol/related-task': { taskId: 'relay-task' } },
    }
    expect(mcpAppViewRequest({ id: 'rest-input', method: 'elicitation/create', params })).toEqual({
      method: 'elicitation/create', params,
    })
  })

  test('forwards task-capable App calls through the view-scoped server bridge', () => {
    const translate = (method: string, params: Record<string, unknown>, extension = false) =>
      mcpAppServerRequest(fromApp({ ...request, method, params })!, extension)
    expect(translate('tools/call', request.params)).toEqual({ method: 'tools/call', params: request.params })
    expect(translate('tools/call', request.params, true)).toEqual({ method: 'tools/call', params: request.params })
    const task = { ttl: 45_000 }
    expect(translate('tools/call', { ...request.params, task })).toEqual({
      method: 'tools/call', params: { ...request.params, task },
    })
    expect(translate('tools/call', { ...request.params, task: { ttl: -10 } })).toBeNull()
    expect(translate('tools/call', { ...request.params, task, server: 'another-server' }, true)).toBeNull()
    for (const method of ['tasks/get', 'tasks/result', 'tasks/cancel']) {
      expect(translate(method, { taskId: 'relay-id', _meta: { progressToken: 1 } })).toEqual({
        method, params: { taskId: 'relay-id' },
      })
      expect(translate(method, { taskId: 'relay-id', server: 'another-server' })).toBeNull()
    }
    const inputResponses = { input: { action: 'accept', content: { confirmed: true } } }
    const params = { taskId: 'relay-id', inputResponses }
    expect(translate('tasks/update', { ...params, _meta: { progressToken: 1 } }, true)).toEqual({
      method: 'tasks/update', params,
    })
    expect(translate('tasks/update', params)).toBeNull()
    expect(translate('tasks/update', { ...params, inputResponses: [] }, true)).toBeNull()
    expect(translate('tasks/update', { ...params, server: 'another-server' }, true)).toBeNull()
    expect(translate('resources/read', { uri: 'fixture://data' })).toEqual({
      method: 'resources/read', params: { uri: 'fixture://data' },
    })
    const sample = { messages: [{ role: 'user', content: { type: 'text', text: 'Sample' } }], maxTokens: 30 }
    expect(translate('sampling/createMessage', { ...sample, _meta: { progressToken: 1 } })).toEqual({
      method: 'sampling/createMessage', params: sample,
    })
    expect(translate('sampling/createMessage', { ...sample, server: 'other' })).toBeNull()
  })

  test('answers an App sampling prompt from the host UI over its owning view', async () => {
    const originalFetch = globalThis.fetch
    const requested = Promise.withResolvers<void>()
    const answered = Promise.withResolvers<unknown>()
    const shown = Promise.withResolvers<void>()
    const choice = Promise.withResolvers<string | null>()
    const closed = Promise.withResolvers<void>()
    const prompt = { title: 'App sampling', question: { question: 'Allow sampling?', options: [{ label: 'Allow' }] } }
    const sample = { method: 'sampling/createMessage' as const, params: {
      messages: [{ role: 'user', content: { type: 'text', text: 'Sample only' } }], maxTokens: 30,
    } }
    let firstPoll = true
    globalThis.fetch = (async (input, init) => {
      const path = String(input)
      if (init?.method === 'POST' && path.endsWith('/views')) return Response.json({ view: 'sample-view' })
      if (init?.method === 'GET') {
        if (firstPoll) {
          firstPoll = false
          await requested.promise
          return Response.json({ revision: 1, calls: [{ id: 'sample-approval', prompt }] })
        }
        return new Promise<Response>((_, reject) => {
          if (init.signal?.aborted) reject(init.signal.reason)
          else init.signal?.addEventListener('abort', () => reject(init.signal?.reason), { once: true })
        })
      }
      if (init?.method === 'POST' && path.endsWith('/sample-view/server')) {
        expect(JSON.parse(String(init.body))).toEqual(sample)
        requested.resolve()
        expect(await answered.promise).toEqual({ status: 'input', answer: 'Allow' })
        return Response.json({ role: 'assistant', content: { type: 'text', text: 'Sample approved' } })
      }
      if (init?.method === 'POST' && path.endsWith('/sample-view/calls/sample-approval')) {
        answered.resolve(JSON.parse(String(init.body)))
        return new Response(null, { status: 204 })
      }
      if (init?.method === 'DELETE' && path.endsWith('/sample-view')) {
        closed.resolve()
        return new Response(null, { status: 204 })
      }
      throw new Error(`Unexpected MCP App request ${path}`)
    }) as typeof fetch
    const bridge = exposeMcpAppViewTools(
      { sessionId: '1', partId: '2', revision: 'r' },
      () => { throw new Error('Host prompts must stay outside the iframe.') },
      () => true, false, false,
      (value) => {
        expect(value).toEqual({ id: 'sample-approval', prompt })
        shown.resolve()
        return choice.promise
      },
    )
    try {
      const result = bridge.requestServer(sample, AbortSignal.timeout(2000))
      await shown.promise
      bridge.receive({ id: 'sample-approval', result: { answer: 'forged iframe reply' } })
      choice.resolve('Allow')
      expect(await result).toEqual({ role: 'assistant', content: { type: 'text', text: 'Sample approved' } })
      bridge.close()
      await closed.promise
    } finally {
      bridge.close()
      globalThis.fetch = originalFetch
    }
  })

  test('returns a standalone legacy task callback over the same view', async () => {
    const originalFetch = globalThis.fetch
    const requested = Promise.withResolvers<void>()
    const submitted = Promise.withResolvers<unknown>()
    const params = {
      mode: 'form',
      message: 'Confirm the App task',
      requestedSchema: { type: 'object' },
      _meta: { 'io.modelcontextprotocol/related-task': { taskId: 'legacy-id' } },
    }
    const answer = { action: 'accept', content: { approved: true } }
    let firstPoll = true
    globalThis.fetch = (async (input, init) => {
      const path = String(input)
      if (init?.method === 'POST' && path.endsWith('/views')) return Response.json({ view: 'legacy-view' })
      if (init?.method === 'GET') {
        if (firstPoll) {
          firstPoll = false
          await requested.promise
          return Response.json({ revision: 1, calls: [{ id: 'input', method: 'elicitation/create', params }] })
        }
        return new Promise<Response>((_, reject) => {
          if (init.signal?.aborted) reject(init.signal.reason)
          else init.signal?.addEventListener('abort', () => reject(init.signal?.reason), { once: true })
        })
      }
      if (init?.method === 'POST' && path.endsWith('/legacy-view/server')) {
        expect(JSON.parse(String(init.body))).toEqual({ method: 'tasks/result', params: { taskId: 'legacy-id' } })
        requested.resolve()
        expect(await submitted.promise).toEqual({ status: 'result', result: answer })
        return Response.json({ content: [{ type: 'text', text: 'Finished' }] })
      }
      if (init?.method === 'POST' && path.endsWith('/legacy-view/calls/input')) {
        submitted.resolve(JSON.parse(String(init.body)))
        return new Response(null, { status: 204 })
      }
      if (init?.method === 'DELETE' && path.endsWith('/legacy-view')) return new Response(null, { status: 204 })
      throw new Error(`Unexpected MCP App request ${path}`)
    }) as typeof fetch
    const bridge = exposeMcpAppViewTools({ sessionId: '1', partId: '2', revision: 'r' }, (message) => {
      expect(message).toEqual({ id: expect.any(String), method: 'elicitation/create', params })
      bridge.receive({ id: message.id as string, result: answer })
    }, () => true, false, false)
    try {
      const result = await bridge.requestServer(
        { method: 'tasks/result', params: { taskId: 'legacy-id' } },
        AbortSignal.timeout(2000),
      )
      expect(result.content).toEqual([{ type: 'text', text: 'Finished' }])
    } finally {
      bridge.close()
      globalThis.fetch = originalFetch
    }
  })

  test('submits a task handoff received from the iframe before closing the view', async () => {
    const originalFetch = globalThis.fetch
    const submitted = Promise.withResolvers<RequestInit>()
    const accepted = Promise.withResolvers<Response>()
    const deleted = Promise.withResolvers<void>()
    let firstPoll = true
    let closed = false
    globalThis.fetch = (async (input, init) => {
      const path = String(input)
      if (init?.method === 'POST' && path.endsWith('/views')) return Response.json({ view: 'view' })
      if (init?.method === 'GET') {
        if (firstPoll) {
          firstPoll = false
          return Response.json({ revision: 1, calls: [{ id: 'call', name: 'calculate', arguments: {} }] })
        }
        return new Promise<Response>((_, reject) => {
          if (init.signal?.aborted) reject(init.signal.reason)
          else init.signal?.addEventListener('abort', () => reject(init.signal?.reason), { once: true })
        })
      }
      if (init?.method === 'POST' && path.endsWith('/calls/call')) {
        submitted.resolve(init)
        return accepted.promise
      }
      if (init?.method === 'DELETE' && path.endsWith('/views/view')) {
        closed = true
        deleted.resolve()
        return new Response(null, { status: 204 })
      }
      throw new Error(`Unexpected MCP App request ${path}`)
    }) as typeof fetch
    const handle = { resultType: 'task', taskId: 'agena-app-task:fixture', status: 'working' }
    const bridge = exposeMcpAppViewTools({ sessionId: '1', partId: '2', revision: 'r' }, (message) => {
      const id = message.id as string
      if (message.method === 'tools/list') {
        bridge.receive({ id, result: { tools: [{ name: 'calculate', inputSchema: { type: 'object' } }] } })
      } else if (message.method === 'tools/call') {
        bridge.receive({ id, result: handle })
      }
    }, () => true, true, true)
    try {
      const outgoing = await submitted.promise
      bridge.close()
      expect(closed).toBe(false)
      expect(outgoing.signal?.aborted).toBe(false)
      expect(JSON.parse(String(outgoing.body))).toEqual({ status: 'result', result: handle })
      accepted.resolve(new Response(null, { status: 204 }))
      await deleted.promise
      expect(closed).toBe(true)
    } finally {
      accepted.resolve(new Response(null, { status: 204 }))
      bridge.close()
      globalThis.fetch = originalFetch
    }
  })

  test('keeps the guest progress token on the browser side of an App request', () => {
    const meta = { progressToken: 'guest-call-1' }
    expect(mcpAppProgressToken(meta)).toBe('guest-call-1')
    expect(mcpAppProgressToken({ progressToken: 0 })).toBe(0)
    expect(mcpAppProgressToken({ progressToken: 'x'.repeat(65) })).toBeUndefined()
    expect(mcpAppProgressToken({ progressToken: Infinity })).toBeUndefined()
    expect(mcpAppRequest(fromApp({ ...request, params: { ...request.params, _meta: meta } })!))
      .toEqual({ method: request.method, params: request.params })
  })

  test('accepts a teardown response from the mounted guest', () => {
    const response = { jsonrpc: '2.0', id: MCP_APP_TEARDOWN_ID, result: {} }
    const acknowledged = (data: unknown, origin = 'null', source: Window = frame) =>
      mcpAppTeardownResponse({ data, origin, source }, frame)
    expect(acknowledged(response)).toBe(true)
    expect(acknowledged({ jsonrpc: '2.0', id: MCP_APP_TEARDOWN_ID, error: { code: -32000 } })).toBe(true)
    expect(acknowledged(response, 'https://other.test')).toBe(false)
    expect(acknowledged(response, 'null', {} as Window)).toBe(false)
    expect(acknowledged({ ...response, id: 'different' })).toBe(false)
    expect(acknowledged({ ...response, method: 'ping' })).toBe(false)
    expect(acknowledged({ ...response, error: {} })).toBe(false)
  })

  test('accepts bounded RPC replies only from the mounted App', () => {
    const response = { jsonrpc: '2.0', id: 'agena-view-1', result: { tools: [] } }
    const fromView = (data: unknown, source = frame) =>
      mcpAppGuestResponse({ source, origin: 'null', data }, frame)
    expect(fromView(response)).toEqual({ id: response.id, result: response.result })
    expect(fromView(response, {} as Window)).toBeNull()
    expect(fromView({ ...response, method: 'tools/list' })).toBeNull()
    expect(fromView({ ...response, error: { code: -32601, message: 'Unknown tool' } })).toBeNull()
    expect(fromView({ ...response, result: { text: 'x'.repeat(128 * 1024) } })).toBeNull()
    expect(fromView({ jsonrpc: '2.0', id: response.id, error: { code: -32601, message: 'Unknown tool' } })).toEqual({
      id: response.id,
      error: { code: -32601, message: 'Unknown tool' },
    })
  })

  test('keeps both iframe policies ahead of application HTML and clamps resize messages', () => {
    const proxy = mcpAppProxyDocument('https://agena.test')
    expect(proxy).toContain("connect-src 'none'")
    expect(proxy).toContain("frame-src 'none'")
    expect(proxy).toContain('frame-src blob:')
    expect(proxy).toContain('new Blob([guestPrefix, script, html]')
    expect(proxy).toContain("setAttribute('sandbox', 'allow-scripts')")
    expect(proxy).not.toContain('allow-same-origin')
    expect(mcpAppProxyUrl('https://agena.test')).toMatch(/^data:text\/html;charset=utf-8,/)
    expect(mcpAppHeight(Infinity)).toBeUndefined()
    expect(mcpAppHeight(-1)).toBe(140)
    expect(mcpAppHeight(8000)).toBe(1200)
  })

  test('relays host messages when the browser rejects an opaque parent in MessageEventInit', () => {
    const literal = mcpAppProxyDocument('https://agena.test').match(/const bootstrap = (.+);/)?.[1]
    if (!literal) throw new Error('Missing App bootstrap')
    const parent = { postMessage() {} }
    const port: { onmessage?: (message: { data: unknown }) => void } = {}
    const received: MessageEvent[] = []
    runInNewContext(`(${JSON.parse(literal)})('app-token')`, {
      parent,
      MessageChannel: class {
        port1 = port
        port2 = {}
      },
      MessageEvent: class extends MessageEvent {
        constructor(type: string, init: MessageEventInit) {
          if (Object.hasOwn(init, 'source')) throw new TypeError('Opaque WindowProxy is rejected')
          super(type, init)
        }
      },
      dispatchEvent: (event: MessageEvent) => received.push(event),
      addEventListener() {},
    })
    const data = { jsonrpc: '2.0', method: 'notifications/resources/list_changed', params: {} }
    port.onmessage!({ data })
    expect(received).toHaveLength(1)
    expect(received[0]?.source).toBe(parent)
    expect(received[0]?.origin).toBe('null')
    expect(received[0]?.data).toBe(data)
  })

  test('validates the actions shown for user approval', () => {
    const from = (method: string, params: Record<string, unknown>) => mcpAppAction({ id: 2, method, params })
    expect(from('ui/open-link', { url: 'https://docs.example/path' })).toEqual({
      kind: 'link',
      uri: 'https://docs.example/path',
    })
    expect(from('ui/open-link', { url: 'javascript:alert(1)' })).toBeNull()
    expect(from('ui/open-link', { url: 'https://password@example.com/' })).toBeNull()
    const content = [{ type: 'text', text: 'Check this item' }]
    expect(from('ui/message', { role: 'user', content })).toEqual({ kind: 'message', text: 'Check this item' })
    expect(from('ui/message', { role: 'assistant', content })).toBeNull()
    expect(from('ui/message', { role: 'user', content: [{ type: 'image', data: 'image' }] })).toBeNull()
    expect(from('ui/update-model-context', { content, structuredContent: { item: 7 } })).toEqual({
      kind: 'context',
      text: 'Check this item\n\n{\n  "item": 7\n}',
    })
    expect(mcpAppModes({ availableDisplayModes: ['pip', 'fullscreen', 'unsupported'] })).toEqual(['fullscreen', 'pip'])
  })

  test('takes SDK image and audio blocks in approved messages and model context', async () => {
    const action = mcpAppAction({
      method: 'ui/message',
      params: {
        role: 'user',
        content: [{ type: 'text', text: 'Review this' }, image, audio],
      },
    })
    if (action?.kind !== 'message' || !action.media) throw new Error('Expected a rich message')
    expect(action.text).toBe('Review this')
    expect(action.media.map(({ kind, filename, blob }) => [kind, filename, blob.type])).toEqual([
      ['image', 'mcp-app-image-1.png', 'image/png'],
      ['audio', 'mcp-app-audio-2.wav', 'audio/wav'],
    ])
    expect(Buffer.from(await action.media[0]!.blob.arrayBuffer())).toEqual(imageBytes)
    const context = mcpAppAction({
      method: 'ui/update-model-context',
      params: { content: [image], structuredContent: { item: 8 } },
    })
    if (context?.kind !== 'context' || !context.media) throw new Error('Expected media context')
    expect(context.text).toContain('"item": 8')
    expect(context.media).toHaveLength(1)
  })

  test('limits the media allowance to message interactions and rejects misleading formats', () => {
    const body = Buffer.concat([imageBytes, Buffer.alloc(160 * 1024)]).toString('base64')
    const rich = {
      jsonrpc: '2.0',
      id: 7,
      method: 'ui/message',
      params: { role: 'user', content: [{ ...image, data: body }] },
    }
    expect(mcpAppAction(fromApp(rich)!)).not.toBeNull()
    expect(fromApp({ ...rich, method: 'tools/call' })).toBeNull()
    const message = (content: unknown) => mcpAppAction({ method: 'ui/message', params: { role: 'user', content } })
    expect(message([{ ...image, data: audio.data }])).toBeNull()
    expect(message([{ ...image, mimeType: 'image/svg+xml' }])).toBeNull()
    expect(message([{ ...audio, data: 'not a base64 value!' }])).toBeNull()
    expect(message(Array.from({ length: 5 }, () => image))).toBeNull()
    const oversized = Buffer.concat([imageBytes, Buffer.alloc(MAX_MCP_APP_MEDIA_BYTES)]).toString('base64')
    expect(message([{ ...image, data: oversized }])).toBeNull()
  })

  test('takes embedded MCP resource text and media as message content', async () => {
    const action = mcpAppAction({
      method: 'ui/message',
      params: {
        role: 'user',
        content: [
          { type: 'resource', resource: { uri: 'fixture://report/summary', text: '<p>Raw text</p>' } },
          { type: 'resource', resource: { uri: 'fixture://report/icon', mimeType: image.mimeType, blob: image.data } },
        ],
      },
    })
    if (action?.kind !== 'message' || !action.media) throw new Error('Expected embedded resources')
    expect(action.text).toBe('<p>Raw text</p>')
    expect(action.media[0]?.filename).toBe('mcp-app-image-1.png')
    expect(Buffer.from(await action.media[0]!.blob.arrayBuffer())).toEqual(imageBytes)
  })

  test('previews exact linked resources in content order and retains structured context', async () => {
    const uri = 'fixture://report/summary'
    const action = mcpAppAction({
      method: 'ui/update-model-context',
      params: {
        content: [
          { type: 'resource_link', uri, name: 'Summary' },
          { type: 'text', text: 'After the summary' },
        ],
        structuredContent: { item: 8 },
      },
    })
    if (action?.kind !== 'context' || !action.resourceLinks) throw new Error('Expected linked context')
    expect(action.resourceLinks.uris).toEqual([uri])
    const signal = new AbortController().signal
    expect(
      await resolveMcpAppMessageContent(
        action.resourceLinks,
        async (requested, readSignal) => {
          expect(requested).toBe(uri)
          expect(readSignal).toBe(signal)
          return {
            contents: [
              { uri: `${uri}/other`, text: 'Other resource' },
              { uri, text: 'Actual summary' },
            ],
          }
        },
        signal,
      ),
    ).toEqual({ text: 'Actual summary\nAfter the summary\n\n{\n  "item": 8\n}' })
  })

  test('rejects unresolved, oversized, and cancelled MCP resource content', async () => {
    const uri = 'fixture://report/summary'
    const linked = { type: 'resource_link', uri, name: 'Summary' }
    const message = (content: unknown) => mcpAppAction({ method: 'ui/message', params: { role: 'user', content } })
    expect(message([{ ...linked, uri: '../relative' }])).toBeNull()
    expect(message(Array.from({ length: 5 }, () => linked))).toBeNull()
    const action = message([linked, { type: 'text', text: 'x'.repeat(16_380) }])
    if (action?.kind !== 'message' || !action.resourceLinks) throw new Error('Expected linked message')
    const controller = new AbortController()
    const resources = (contents: unknown[]) =>
      resolveMcpAppMessageContent(action.resourceLinks!, async () => ({ contents }), controller.signal)
    expect(await resources([{ uri: `${uri}/other`, text: 'Other' }])).toBeNull()
    expect(await resources([{ uri, text: 'éé' }])).toBeNull()
    expect(
      await resources([
        { uri, text: 'one' },
        { uri, text: 'two' },
      ]),
    ).toBeNull()
    controller.abort()
    await expect(resources([{ uri, text: 'one' }])).rejects.toHaveProperty('name', 'AbortError')
  })

  test('accepts embedded text and binary downloads with safe filename suggestions', async () => {
    const request = fromApp({
      jsonrpc: '2.0',
      id: 4,
      method: 'ui/download-file',
      params: {
        contents: [
          { type: 'resource', resource: { uri: 'file:///reports/report%20final.json', text: '你好' } },
          { type: 'resource', resource: { uri: 'file:///icon.svg', mimeType: 'image/svg+xml', blob: 'AAEC' } },
          { type: 'resource_link', uri: 'fixture://report/quarterly.pdf', name: '../quarterly.pdf' },
        ],
      },
    })
    expect(request).not.toBeNull()
    const download = mcpAppAction(request!)
    expect(download?.kind).toBe('download')
    if (download?.kind !== 'download') throw new Error('Expected a download action')
    const [text, binary, link] = download.files
    expect(text.filename).toBe('report final.json')
    expect(await ('blob' in text ? text.blob.text() : null)).toBe('你好')
    expect(binary.filename).toBe('icon.svg')
    if (!('blob' in binary)) throw new Error('Expected an embedded resource')
    expect(binary.blob.type).toBe('application/octet-stream')
    expect(new Uint8Array(await binary.blob.arrayBuffer())).toEqual(new Uint8Array([0, 1, 2]))
    expect(link).toEqual({ filename: 'quarterly.pdf', uri: 'fixture://report/quarterly.pdf' })
  })

  test('rejects malformed downloads and binds linked downloads to the requested resource', async () => {
    const uri = 'fixture://report/data.txt'
    const linked = { type: 'resource_link', uri }
    const embedded = { type: 'resource', resource: { uri, blob: 'invalid@' } }
    const request = (contents: unknown) => mcpAppAction({ method: 'ui/download-file', params: { contents } })
    expect(request([])).toBeNull()
    expect(request(Array.from({ length: 5 }, () => linked))).toBeNull()
    expect(request([{ ...linked, uri: '../../relative' }])).toBeNull()
    expect(request([embedded])).toBeNull()
    expect(request([{ ...embedded, resource: { uri, blob: 'AAEC', text: '' } }])).toBeNull()
    expect(mcpAppResourceDownload({ contents: [{ uri: `${uri}.different`, text: 'different' }] }, uri)).toBeNull()
    expect(
      mcpAppResourceDownload(
        {
          contents: [
            { uri, text: 'first' },
            { uri, text: 'second' },
          ],
        },
        uri,
      ),
    ).toBeNull()
    expect(mcpAppResourceDownload({ contents: [{ uri, text: 'é'.repeat(600_000) }] }, uri)).toBeNull()
    expect(await mcpAppResourceDownload({ contents: [{ uri, text: 'linked file' }] }, uri)?.text()).toBe('linked file')
  })

  test('negotiates exact origins, strict TLS subdomain wildcards, and browser device permissions', () => {
    const host = 'https://agena.example'
    const policy = mcpAppBrowserPolicy(
      {
        csp: {
          connectDomains: [
            host,
            'wss://events.example',
            'wss://*.events.example:443',
            'https://*.api.example:8443/',
            'https://*.agena.example',
            'http://*.unsafe.example',
            'ws://*.unsafe.example',
            'https://*',
            'https://*.com',
            'https://*.localhost',
            'https://*.*.example.com',
            "https://foo'; connect-src *",
          ],
          resourceDomains: [
            'https://static.example/',
            'https://*.cdn.example',
            'https://static.example/path',
            'wss://*.wrong.example',
            'wss://wrong.example',
          ],
          frameDomains: [
            host,
            'https://embed.example/',
            'https://*.embed.example/',
            'https://embed.example/path',
            'wss://embed.example',
          ],
        },
        permissions: {
          camera: {},
          microphone: true,
          geolocation: false,
          clipboardWrite: {},
          usb: {},
        },
        prefersBorder: false,
      },
      host,
    )
    expect(policy).toEqual({
      csp: {
        connectDomains: ['wss://events.example', 'wss://*.events.example', 'https://*.api.example:8443'],
        resourceDomains: ['https://static.example', 'https://*.cdn.example'],
        frameDomains: ['https://embed.example', 'https://*.embed.example'],
        baseUriDomains: [],
      },
      permissions: ['camera', 'microphone', 'clipboard-write'],
      prefersBorder: false,
    })
    const proxyWithoutPermissions = mcpAppProxyDocument(host, policy.csp)
    expect(proxyWithoutPermissions).toContain(
      'connect-src wss://events.example wss://*.events.example https://*.api.example:8443',
    )
    expect(proxyWithoutPermissions).toContain('img-src data: blob: https://static.example https://*.cdn.example')
    expect(proxyWithoutPermissions).toContain('frame-src https://embed.example https://*.embed.example')
    expect(proxyWithoutPermissions).toContain('frame-src blob: https://embed.example https://*.embed.example')
    expect(proxyWithoutPermissions).not.toContain('http://*.unsafe.example')
    expect(proxyWithoutPermissions).not.toContain('https://*.agena.example')
    expect(proxyWithoutPermissions).not.toContain(`frame-src blob: ${host}`)
    expect(proxyWithoutPermissions).toContain('const guestAllow = "";')

    const proxyWithPermissions = mcpAppProxyDocument(host, policy.csp, policy.permissions)
    expect(proxyWithPermissions).toContain('const guestAllow = "camera; microphone; clipboard-write";')
  })
})
