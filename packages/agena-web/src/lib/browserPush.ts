// The worker has no fetch handler and never caches the app. Its only job is to
// receive Web Push after Chrome has frozen/discarded the page.
let worker: Promise<ServiceWorkerRegistration> | undefined

export function supportsBrowserPush() {
  try {
    return typeof navigator !== 'undefined' && !!navigator.serviceWorker && typeof PushManager !== 'undefined'
  } catch {
    return false
  }
}

export function browserNotificationPlatformHint(): 'iosInstall' | '' {
  if (typeof navigator === 'undefined' || typeof window === 'undefined') return ''
  const ios =
    /iPad|iPhone|iPod/.test(navigator.userAgent) || (navigator.platform === 'MacIntel' && navigator.maxTouchPoints > 1)
  const standalone =
    (navigator as Navigator & { standalone?: boolean }).standalone ||
    window.matchMedia?.('(display-mode: standalone)').matches
  return ios && !standalone ? 'iosInstall' : ''
}

export async function notificationWorker(): Promise<ServiceWorkerRegistration> {
  if (!navigator.serviceWorker) throw new Error('Service workers are unavailable')
  if (!worker) {
    worker = navigator.serviceWorker
      .register('/agena-notifications-sw.js', { scope: '/', updateViaCache: 'none' })
      .then(async (registration) => {
        if (registration.active?.state === 'activated') return registration
        await new Promise<void>((resolve, reject) => {
          const installing = registration.installing || registration.waiting || registration.active
          const timeout = setTimeout(() => finish(new Error('Notification worker activation timed out')), 20_000)
          function finish(error?: Error) {
            clearTimeout(timeout)
            installing?.removeEventListener('statechange', changed)
            if (error) reject(error)
            else resolve()
          }
          function changed() {
            if (installing?.state === 'activated') finish()
            else if (!installing || installing.state === 'redundant') finish(new Error('Notification worker failed'))
          }
          installing?.addEventListener('statechange', changed)
          changed()
        })
        return registration
      })
      .catch((error) => {
        worker = undefined
        throw error
      })
  }
  return worker
}

export async function subscribeBrowserPush(publicKey: string) {
  let timer: ReturnType<typeof setTimeout> | undefined
  try {
    return await Promise.race([
      subscribe(publicKey),
      new Promise<PushSubscription>((_resolve, reject) => {
        timer = setTimeout(() => reject(new Error('Browser push subscription timed out')), 25_000)
      }),
    ])
  } finally {
    clearTimeout(timer)
  }
}

async function subscribe(publicKey: string) {
  const registration = await notificationWorker()
  let subscription = await registration.pushManager.getSubscription()
  const base64 = publicKey.replace(/-/g, '+').replace(/_/g, '/')
  const bytes = Uint8Array.from(atob(base64 + '='.repeat((4 - (base64.length % 4)) % 4)), (char) => char.charCodeAt(0))
  const previousKey = subscription?.options.applicationServerKey
  if (subscription && previousKey && !equalBytes(new Uint8Array(previousKey), bytes)) {
    await subscription.unsubscribe()
    subscription = null
  }
  return subscription || registration.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: bytes })
}

function equalBytes(a: Uint8Array, b: Uint8Array) {
  return a.length === b.length && a.every((value, index) => value === b[index])
}
