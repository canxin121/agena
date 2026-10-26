import assert from 'node:assert/strict'
import { afterEach, beforeEach, test } from 'node:test'

import {
  browserNotificationPermission,
  createBrowserNotifier,
  requestBrowserNotificationPermission,
  showBrowserNotification,
  type BrowserNotificationSnapshot,
} from '../src/lib/browserNotifications'
import type { SseEvent } from '../src/lib/sse'
import { browserNotificationPlatformHint, supportsBrowserPush } from '../src/lib/browserPush'

class FakeNotification {
  static permission: NotificationPermission = 'granted'
  static requested = 0
  static fail = false
  static asyncFail = false
  static async requestPermission() {
    this.requested++
    this.permission = 'granted'
    return this.permission
  }
  onclick?: () => void
  onclose?: () => void
  onshow?: (() => void) | null
  onerror?: (() => void) | null
  closed = false
  constructor(
    readonly title: string,
    readonly options: NotificationOptions,
  ) {
    if (FakeNotification.fail) throw new TypeError('Notification constructor is unavailable')
    shown.push(this)
    queueMicrotask(() => (FakeNotification.asyncFail ? this.onerror?.() : this.onshow?.()))
  }
  close() {
    this.closed = true
    this.onclose?.()
  }
}

let shown: FakeNotification[]
let storage: Map<string, string>
let opened: string[]
let focused: number
let enabled: boolean
let scope: string
let locks: Promise<unknown>
const descriptors = new Map<string, PropertyDescriptor | undefined>()
const notifiers: ReturnType<typeof createBrowserNotifier>[] = []

function setGlobal(name: string, value: unknown) {
  if (!descriptors.has(name)) descriptors.set(name, Object.getOwnPropertyDescriptor(globalThis, name))
  Object.defineProperty(globalThis, name, { configurable: true, writable: true, value })
}

beforeEach(() => {
  shown = []
  storage = new Map()
  opened = []
  focused = 0
  enabled = true
  scope = 'https://agena.test/'
  locks = Promise.resolve()
  FakeNotification.permission = 'granted'
  FakeNotification.requested = 0
  FakeNotification.fail = false
  FakeNotification.asyncFail = false
  setGlobal('window', { isSecureContext: true, Notification: FakeNotification, focus: () => focused++ })
  setGlobal('document', { hidden: true, visibilityState: 'hidden' })
  setGlobal('localStorage', {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => storage.set(key, value),
  })
  setGlobal('navigator', {
    locks: {
      request: (_name: string, run: () => unknown) => {
        const next = locks.then(run)
        locks = next.catch(() => {})
        return next
      },
    },
  })
})

afterEach(() => {
  notifiers.splice(0).forEach((notifier) => notifier.dispose())
  for (const [name, descriptor] of descriptors) {
    if (descriptor) Object.defineProperty(globalThis, name, descriptor)
    else Reflect.deleteProperty(globalThis, name)
  }
  descriptors.clear()
})

function notifier(
  readSession?: (sid: string, replyIds: number[] | undefined) => Promise<BrowserNotificationSnapshot | null>,
) {
  const instance = createBrowserNotifier({
    enabled: () => enabled,
    scope: () => scope,
    sessionTitle: (sid) => `Session ${sid}`,
    message: (kind) => kind,
    openSession: (sid) => opened.push(sid),
    readSession,
  })
  notifiers.push(instance)
  return instance
}

function partEvent(part: Record<string, unknown> = {}, sessionId = 7): SseEvent {
  return {
    type: 'session_changed',
    properties: {
      kind: 'part_updated',
      session_id: sessionId,
      part: {
        part_id: 10,
        role: 'assistant',
        kind: 'text',
        origin_session_id: sessionId,
        message_boundary: true,
        state: 'completed',
        run_state: 'completed',
        created_at_ms: 1234,
        revision: 4,
        ...part,
      },
    },
  } as SseEvent
}

function requestEvent(kind: 'permission' | 'user_input', requestId = 'request-1', part: Record<string, unknown> = {}) {
  const request = { kind, request_id: requestId, session_id: 7 }
  const content =
    kind === 'permission'
      ? { authorization: { permissions: [{ request }] } }
      : { user_input: { requests: [{ request }] } }
  return partEvent({ kind: 'tool_call', state: 'running', run_state: 'running', content, ...part })
}

test('permission prompts run only on explicit request and respect browser restrictions', async () => {
  FakeNotification.permission = 'default'
  assert.equal(browserNotificationPermission(), 'default')
  assert.equal(await showBrowserNotification('test', {}), null)
  assert.equal(FakeNotification.requested, 0)
  assert.equal(await requestBrowserNotificationPermission(), 'granted')
  assert.equal(FakeNotification.requested, 1)
  FakeNotification.permission = 'denied'
  assert.equal(await requestBrowserNotificationPermission(), 'denied')
  assert.equal(FakeNotification.requested, 1)
  setGlobal('window', { isSecureContext: false, Notification: FakeNotification })
  assert.equal(await requestBrowserNotificationPermission(), 'insecure')
  setGlobal('window', { isSecureContext: true })
  assert.equal(await requestBrowserNotificationPermission(), 'unsupported')
})

test('desktop Chrome, Edge, Firefox, and Safari use capability checks instead of browser exclusions', () => {
  setGlobal('PushManager', class {})
  for (const userAgent of ['Chrome/144.0', 'Chrome/144.0 Edg/144.0', 'Firefox/144.0', 'Version/18.0 Safari/605.1.15']) {
    setGlobal('navigator', { userAgent, platform: 'MacIntel', maxTouchPoints: 0, serviceWorker: {} })
    assert.equal(browserNotificationPlatformHint(), '')
    assert.equal(browserNotificationPermission(), 'granted')
    assert.equal(supportsBrowserPush(), true)
  }
  setGlobal(
    'navigator',
    Object.defineProperty({}, 'serviceWorker', {
      get: () => {
        throw new Error('Blocked by browser')
      },
    }),
  )
  assert.equal(supportsBrowserPush(), false)
})

test('iPhone and desktop-mode iPad require a Home Screen app before offering notification permission', () => {
  setGlobal('navigator', { userAgent: 'iPhone', platform: 'iPhone', maxTouchPoints: 5 })
  assert.equal(browserNotificationPlatformHint(), 'iosInstall')
  assert.equal(browserNotificationPermission(), 'unsupported')
  setGlobal('navigator', { userAgent: 'Macintosh Safari', platform: 'MacIntel', maxTouchPoints: 5 })
  assert.equal(browserNotificationPlatformHint(), 'iosInstall')
  setGlobal('navigator', { userAgent: 'iPad', platform: 'MacIntel', maxTouchPoints: 5, standalone: true })
  assert.equal(browserNotificationPlatformHint(), '')
  assert.equal(browserNotificationPermission(), 'granted')
  setGlobal('navigator', { userAgent: 'iPhone', platform: 'iPhone', maxTouchPoints: 5 })
  setGlobal('window', { isSecureContext: true, Notification: FakeNotification, matchMedia: () => ({ matches: true }) })
  assert.equal(browserNotificationPlatformHint(), '')
  assert.equal(browserNotificationPermission(), 'granted')
})

test('browser restrictions on service workers still allow a desktop page notification', async () => {
  setGlobal(
    'navigator',
    Object.defineProperty({}, 'serviceWorker', {
      get: () => {
        throw new DOMException('Service workers are blocked', 'SecurityError')
      },
    }),
  )
  const notification = await showBrowserNotification('Page fallback', { body: 'AI finished' })
  assert.ok(notification)
  assert.equal(shown.length, 1)
  assert.equal(shown[0]!.options.body, 'AI finished')
})

test('a finished reply notifies in a background tab and clicking focuses its session', async () => {
  await notifier().applyEvent(partEvent())
  assert.equal(shown.length, 1)
  assert.equal(shown[0]!.title, 'Session 7')
  assert.equal(shown[0]!.options.body, 'completed')
  shown[0]!.onclick?.()
  assert.equal(focused, 1)
  assert.deepEqual(opened, ['7'])
  assert.equal(shown[0]!.closed, true)
})

test('content completion and tool progress cannot prematurely announce a finished reply', async () => {
  const client = notifier()
  await client.applyEvent(partEvent({ state: 'completed', run_state: 'running' }))
  await client.applyEvent(partEvent({ message_boundary: false, part_id: 11 }))
  await client.applyEvent(partEvent({ role: 'user', part_id: 12 }))
  await client.applyEvent(partEvent({ origin_session_id: 8, part_id: 13 }))
  assert.equal(shown.length, 0)
  await client.applyEvent(partEvent({ run_state: 'completed', revision: 5 }))
  assert.equal(shown.length, 1)
})

test('terminal failures and interruptions use distinct notification messages', async () => {
  const client = notifier()
  await client.applyEvent(partEvent({ run_state: 'failed' }))
  await client.applyEvent(partEvent({ run_state: 'cancelled', part_id: 11 }))
  assert.deepEqual(
    shown.map((item) => item.options.body),
    ['failed', 'cancelled'],
  )
})

test('questions and approval requests notify once per request, including parallel requests', async () => {
  const client = notifier()
  await client.applyEvent(requestEvent('user_input'))
  await client.applyEvent(requestEvent('user_input', 'request-1', { revision: 5 }))
  await client.applyEvent(requestEvent('permission', 'request-2', { revision: 6 }))
  assert.deepEqual(
    shown.map((item) => item.options.body),
    ['question', 'permission'],
  )
})

test('answered requests, completed tools, and inherited requests do not notify', async () => {
  const client = notifier()
  await client.applyEvent(
    requestEvent('user_input', 'answered', {
      content: { user_input: { requests: [{ request: { kind: 'user_input', request_id: 'answered' }, reply: {} }] } },
    }),
  )
  await client.applyEvent(requestEvent('permission', 'completed', { message_boundary: false, state: 'completed' }))
  await client.applyEvent(requestEvent('user_input', 'forked', { origin_session_id: 8 }))
  assert.equal(shown.length, 0)
})

test('older part revisions cannot reintroduce a resolved interaction', async () => {
  const client = notifier()
  await client.applyEvent(partEvent({ message_boundary: false, kind: 'tool_call', run_state: 'running', revision: 10 }))
  await client.applyEvent(requestEvent('permission', 'stale', { revision: 9 }))
  assert.equal(shown.length, 0)
})

test('repeated events and concurrent browser tabs share one delivery claim', async () => {
  const first = notifier()
  const second = notifier()
  await Promise.all([first.applyEvent(partEvent()), second.applyEvent(partEvent())])
  await first.applyEvent(partEvent({ revision: 5 }))
  await notifier().applyEvent(partEvent())
  assert.equal(shown.length, 1)
})

test('different sessions and backend servers have independent notification identities', async () => {
  const client = notifier()
  await client.applyEvent(partEvent())
  await client.applyEvent(partEvent({}, 8))
  scope = 'https://other-backend.test/'
  await client.applyEvent(partEvent())
  assert.deepEqual(
    shown.map((item) => item.title),
    ['Session 7', 'Session 8', 'Session 7'],
  )
})

test('disabling notifications prevents new alerts; disposal closes outstanding alerts', async () => {
  const client = notifier()
  enabled = false
  await client.applyEvent(partEvent())
  assert.equal(shown.length, 0)
  enabled = true
  await client.applyEvent(partEvent({ part_id: 11 }))
  client.dispose()
  assert.equal(shown[0]!.closed, true)
  await client.applyEvent(partEvent({ part_id: 12 }))
  assert.equal(shown.length, 1)
})

test('blocked storage and unavailable locks do not prevent delivery or in-memory deduplication', async () => {
  setGlobal('localStorage', {
    getItem: () => {
      throw new Error('storage blocked')
    },
    setItem: () => {
      throw new Error('storage blocked')
    },
  })
  setGlobal('navigator', { locks: { request: () => Promise.reject(new Error('locks blocked')) } })
  const client = notifier()
  await client.applyEvent(partEvent())
  await client.applyEvent(partEvent())
  assert.equal(shown.length, 1)
})

test('unavailable delivery is retryable and never claims that a notification was delivered', async () => {
  const client = notifier()
  FakeNotification.permission = 'denied'
  await assert.rejects(client.applyEvent(partEvent()), /delivery failed/)
  FakeNotification.permission = 'granted'
  FakeNotification.fail = true
  await assert.rejects(client.applyEvent(partEvent()), /delivery failed/)
  assert.equal(shown.length, 0)
  assert.equal(storage.size, 0)
  FakeNotification.fail = false
  await client.applyEvent(partEvent())
  assert.equal(shown.length, 1)
})

test('asynchronous notification errors do not poison deduplication and recovery retries the reply', async () => {
  const client = notifier(async () => ({ parts: [partEvent().properties!.part] }))
  FakeNotification.asyncFail = true
  await assert.rejects(client.applyEvent(partEvent()), /delivery failed/)
  assert.equal(storage.size, 0)
  assert.equal(shown[0]!.closed, true)
  FakeNotification.asyncFail = false
  await client.reconcile()
  assert.equal(shown.length, 2)
  assert.equal(storage.size, 1)
  await client.applyEvent(partEvent())
  assert.equal(shown.length, 2)
})

test('enabling notifications allows a still-pending request to notify after previously disabled events', async () => {
  const client = notifier()
  enabled = false
  await client.applyEvent(requestEvent('user_input'))
  enabled = true
  await client.applyEvent(requestEvent('user_input'))
  assert.equal(shown.length, 1)
})

test('retry generations of the same assistant boundary produce separate notifications', async () => {
  const client = notifier()
  await client.applyEvent(partEvent({ content: { generation: 1 } }))
  await client.applyEvent(partEvent({ content: { generation: 2 }, revision: 20 }))
  assert.equal(shown.length, 2)
})

test('malformed data, non-session events, and deleted sessions do not produce alerts', async () => {
  const client = notifier()
  await client.applyEvent({ type: 'runtime_signal', properties: { kind: 'activity' } })
  await client.applyEvent({ type: 'session_changed', properties: { kind: 'part_added', part: null } })
  await client.applyEvent(partEvent())
  await client.applyEvent({ type: 'session_changed', properties: { kind: 'session_deleted', session_id: 7 } })
  assert.equal(shown.length, 1)
  assert.equal(shown[0]!.closed, true)
})

test('a lost completion is recovered by rereading the observed reply, including long executions', async () => {
  const reads: Array<[string, number[] | undefined]> = []
  const client = notifier(async (sid, ids) => {
    reads.push([sid, ids])
    return { parts: [partEvent().properties!.part] }
  })
  await client.applyEvent(partEvent({ run_state: 'running' }))
  await client.reconcile()
  await client.reconcile()
  assert.deepEqual(reads, [['7', [10]]])
  assert.equal(shown.length, 1)
  assert.equal(shown[0]!.options.body, 'completed')
})

test('recovery finds pending questions without replaying old completed replies', async () => {
  const reads: Array<number[] | undefined> = []
  const client = notifier(async (_sid, ids) => {
    reads.push(ids)
    return { parts: [], requests: [{ kind: 'user_input', request_id: 'recovered', session_id: 7 }] }
  })
  await client.reconcile()
  assert.equal(reads.length, 0)
  await client.applyEvent({ type: 'session_changed', properties: { kind: 'session_meta_updated', session_id: 7 } })
  await client.reconcile()
  assert.deepEqual(reads, [undefined])
  assert.equal(shown[0]!.options.body, 'question')
})

test('sessions already active at startup can recover completion after a stream gap', async () => {
  const client = notifier(async (_sid, ids) => {
    assert.deepEqual(ids, [])
    return { parts: [partEvent().properties!.part] }
  })
  client.trackSessions(['7'])
  await client.reconcile()
  assert.equal(shown.length, 1)
})

test('live interaction changes take precedence over an older recovery snapshot', async () => {
  let resolveRead!: (snapshot: BrowserNotificationSnapshot) => void
  const client = notifier(
    () =>
      new Promise((resolve) => {
        resolveRead = resolve
      }),
  )
  await client.applyEvent({ type: 'session_changed', properties: { kind: 'session_meta_updated', session_id: 7 } })
  const read = client.reconcile()
  await client.applyEvent(partEvent({ kind: 'tool_call', run_state: 'running', message_boundary: false, revision: 10 }))
  resolveRead({ parts: [], requests: [{ kind: 'user_input', request_id: 'obsolete', session_id: 7 }] })
  await read
  assert.equal(shown.length, 0)
})

test('recovery retires an active session that was deleted while disconnected', async () => {
  let reads = 0
  const client = notifier(async () => {
    reads++
    return null
  })
  client.trackSessions(['7'])
  await client.reconcile()
  await client.reconcile()
  assert.equal(reads, 1)
  assert.equal(shown.length, 0)
})

test('replacing a service-worker notification closes only the old handle, leaving the new alert visible', async () => {
  const registration = {
    active: { state: 'activated' },
    showNotification: async (title: string, options: NotificationOptions) => {
      for (const item of shown) if (item.options.tag === options.tag) item.close()
      new FakeNotification(title, options)
    },
    getNotifications: async ({ tag }: { tag: string }) =>
      shown.filter((item) => !item.closed && item.options.tag === tag),
  }
  setGlobal('navigator', { serviceWorker: { register: async () => registration } })
  const client = notifier()
  await client.applyEvent(partEvent())
  await client.applyEvent(partEvent({ part_id: 11 }))
  assert.equal(shown.length, 2)
  assert.equal(shown[0]!.closed, true)
  assert.equal(shown[1]!.closed, false)
  client.dispose()
  assert.equal(shown[1]!.closed, true)
})
