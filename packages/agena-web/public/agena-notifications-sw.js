/* This worker only handles notifications. No app-shell or API caching. */
self.addEventListener('install', (event) => event.waitUntil(self.skipWaiting()))
self.addEventListener('activate', (event) => event.waitUntil(self.clients.claim()))

self.addEventListener('push', (event) => {
  event.waitUntil(
    (async () => {
      const payload = event.data?.json() || {}
      if (typeof payload.id !== 'string' || typeof payload.body !== 'string') return
      const sessionId = payload.session_id
      const url =
        Number.isSafeInteger(sessionId) && sessionId > 0
          ? `/chat?sessionId=${sessionId}`
          : '/settings/interface?view=web-appearance'
      try {
        await self.registration.showNotification(payload.title || 'Agena', {
          body: payload.body,
          icon: '/android-chrome-192x192.png?v=shift',
          // Retries replace the same event silently; a new reply/request gets
          // its own identity and can alert even if an older notice is visible.
          tag: `agena:push:${payload.id}`,
          requireInteraction: true,
          data: { url, eventId: payload.id },
        })
        await report({ type: 'agena:notification-shown', id: payload.id })
      } catch (error) {
        await report({
          type: 'agena:notification-error',
          id: payload.id,
          error: String(error?.message || error || 'Browser rejected the notification'),
        })
        throw new Error('Browser could not display the push notification')
      }
    })(),
  )
})

async function report(message) {
  try {
    const clients = await self.clients.matchAll({ type: 'window', includeUncontrolled: true })
    for (const client of clients) {
      try {
        client.postMessage(message)
      } catch {
        // The page can close or be discarded between matching and reporting.
      }
    }
  } catch {
    // A diagnostic receipt must not invalidate a displayed notification.
  }
}

self.addEventListener('notificationclick', (event) => {
  event.notification.close()
  event.waitUntil(
    (async () => {
      const url = new URL(event.notification.data?.url || '/', self.location.origin)
      if (url.origin !== self.location.origin) return
      const clients = await self.clients.matchAll({ type: 'window', includeUncontrolled: true })
      for (const client of clients) {
        if (new URL(client.url).origin !== url.origin) continue
        try {
          const focused = await client.focus()
          if (await focused.navigate(url.href)) return
        } catch {
          // A discarded or closing window may disappear during the click.
        }
      }
      await self.clients.openWindow(url.href)
    })(),
  )
})
