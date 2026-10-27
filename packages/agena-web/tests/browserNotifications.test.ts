import assert from 'node:assert/strict'
import { afterEach, beforeEach, test } from 'node:test'

import {
  browserNotificationPermission,
  createBrowserNotifier,
  requestBrowserNotificationPermission,
  showBrowserNotification,
  type PluginAttentionNotification,
} from '../src/lib/browserNotifications'
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

function notifier() {
  const client = createBrowserNotifier({
    enabled: () => enabled,
    scope: () => scope,
    message: (kind) => kind,
    openSession: (sid) => opened.push(sid),
  })
  notifiers.push(client)
  return client
}

function event(
  id = 'execution:one:1',
  kind: NonNullable<PluginAttentionNotification['attention']>['kind'] = 'completed',
  createdAt = Date.now() + 1,
): PluginAttentionNotification {
  return {
    plugin_id: 'agena.terminal',
    session_id: 7,
    title: 'Session 7',
    attention: { id, kind, created_at_ms: createdAt, resolved: false },
  }
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

test('a plugin completion notifies and clicking opens its owning session', async () => {
  await notifier().applyNotifications([event()])
  assert.equal(shown.length, 1)
  assert.equal(shown[0]!.title, 'Session 7')
  assert.equal(shown[0]!.options.body, 'completed')
  assert.equal(shown[0]!.options.tag, 'agena:push:execution:one:1')
  shown[0]!.onclick?.()
  assert.equal(focused, 1)
  assert.deepEqual(opened, ['7'])
})

test('separate replies with identical text notify once for each stable plugin identity', async () => {
  const client = notifier()
  const events = [event('execution:one:1'), event('execution:two:1')]
  await client.applyNotifications(events)
  await client.applyNotifications(events)
  assert.equal(shown.length, 2)
  assert.notEqual(shown[0]!.options.tag, shown[1]!.options.tag)
})

test('the browser renders only explicit plugin attention, without inferring AI state', async () => {
  const client = notifier()
  await client.applyNotifications([{ plugin_id: 'agena.terminal', title: 'running', session_id: 7 }])
  assert.equal(shown.length, 0)
  await client.applyNotifications([event('failed', 'failed'), event('cancelled', 'cancelled')])
  assert.deepEqual(
    shown.map((item) => item.options.body),
    ['failed', 'cancelled'],
  )
})

test('questions and approvals have independent identities and resolved requests stop alerting', async () => {
  const client = notifier()
  const permission = event('request:7:one', 'permission')
  const question = event('request:7:two', 'question')
  await client.applyNotifications([permission, question])
  assert.deepEqual(
    shown.map((item) => item.options.body),
    ['permission', 'question'],
  )
  permission.attention!.resolved = true
  await client.applyNotifications([permission, question])
  assert.equal(shown.length, 2)
  assert.equal(shown[0]!.closed, true)
  const other = notifier()
  await other.applyNotifications([
    { ...permission, attention: { ...permission.attention!, id: 'resolved-before-visit' } },
  ])
  assert.equal(shown.length, 2)
})

test('startup recovers pending questions without replaying old completed replies', async () => {
  const client = notifier()
  const past = Date.now() - 60_000
  await client.applyNotifications([event('old-reply', 'completed', past), event('pending', 'question', past)])
  assert.deepEqual(
    shown.map((item) => item.options.body),
    ['question'],
  )
})

test('a remote server clock supplies the history baseline when the device clock differs', async () => {
  const client = notifier()
  const serverNow = Date.now() - 12 * 60 * 60 * 1000
  await client.applyNotifications([event('old-remote-reply', 'completed', serverNow - 60_000)], serverNow)
  await client.applyNotifications([event('new-remote-reply', 'completed', serverNow + 1)], serverNow + 100)
  assert.equal(shown.length, 1)
  assert.equal(shown[0]!.options.tag, 'agena:push:new-remote-reply')
})

test('delivery errors remain retryable and do not poison deduplication', async () => {
  const client = notifier()
  const notifications = [event()]
  FakeNotification.asyncFail = true
  await assert.rejects(client.applyNotifications(notifications), /delivery failed/)
  assert.equal(shown[0]!.closed, true)
  FakeNotification.asyncFail = false
  await client.applyNotifications(notifications)
  await client.applyNotifications(notifications)
  assert.equal(shown.length, 2)
  assert.equal(shown[1]!.closed, false)
})

test('concurrent tabs share delivery claims for plugin events', async () => {
  const notification = event()
  await Promise.all([notifier().applyNotifications([notification]), notifier().applyNotifications([notification])])
  assert.equal(shown.length, 1)
})

test('disabling and disposing the page renderer prevents further alerts', async () => {
  const client = notifier()
  enabled = false
  const notification = event()
  await client.applyNotifications([notification])
  assert.equal(shown.length, 0)
  enabled = true
  await client.applyNotifications([notification])
  assert.equal(shown.length, 1)
  client.dispose()
  assert.equal(shown[0]!.closed, true)
  await client.applyNotifications([event('later')])
  assert.equal(shown.length, 1)
})

test('reenabling resets reply history while still recovering an unanswered question', async () => {
  const client = notifier()
  const notifications = [event('reply-while-disabled'), event('pending-while-disabled', 'question')]
  await new Promise((resolve) => setTimeout(resolve, 10))
  client.reset()
  await client.applyNotifications(notifications)
  assert.deepEqual(
    shown.map((item) => item.options.body),
    ['question'],
  )
})

test('blocked storage and locks do not prevent delivery or in-memory deduplication', async () => {
  setGlobal('localStorage', {
    getItem: () => {
      throw new Error('blocked')
    },
    setItem: () => {
      throw new Error('blocked')
    },
  })
  setGlobal('navigator', {
    locks: {
      request: async () => {
        throw new Error('blocked')
      },
    },
  })
  const client = notifier()
  const notifications = [event()]
  await client.applyNotifications(notifications)
  await client.applyNotifications(notifications)
  assert.equal(shown.length, 1)
})

test('closing an old worker notification handle cannot close its replacement', async () => {
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
  const previous = await showBrowserNotification('Old', { tag: 'same-event' })
  const replacement = await showBrowserNotification('New', { tag: 'same-event' })
  previous!.close()
  assert.equal(shown[0]!.closed, true)
  assert.equal(shown[1]!.closed, false)
  replacement!.close()
  assert.equal(shown[1]!.closed, true)
})
