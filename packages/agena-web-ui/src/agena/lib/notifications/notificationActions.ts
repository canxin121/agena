// Dispatches a resolved notification `ActionTarget` to the matching web
// capability: frontend route (navigate), clipboard (copy), or a runtime-side
// notice for command/recovery targets that the host resolves.

import type { Router } from 'vue-router'

import type { NotificationActionTarget } from './types'

export async function dispatchNotificationAction(
  target: NotificationActionTarget,
  router: Pick<Router, 'push'>,
  onHandledLocally?: (message: string) => void,
): Promise<void> {
  switch (target.target) {
    case 'navigate': {
      const route = target.route.trim()
      if (/^https?:\/\//i.test(route)) {
        if (typeof window !== 'undefined') window.open(route, '_blank', 'noopener,noreferrer')
      } else if (route) {
        await router.push(route)
      }
      return
    }
    case 'copy': {
      const text = target.text ?? ''
      try {
        if (typeof navigator !== 'undefined' && navigator.clipboard?.writeText) {
          await navigator.clipboard.writeText(text)
          onHandledLocally?.('Copied to clipboard.')
        } else {
          onHandledLocally?.('Clipboard access is unavailable in this browser context.')
        }
      } catch {
        onHandledLocally?.('Clipboard write failed. Try again.')
      }
      return
    }
    case 'command':
    case 'recovery': {
      // The REST action endpoint already resolved the target host-side; command
      // execution happens in the runtime. Surface a notice so the user knows
      // the action was accepted.
      onHandledLocally?.('Action accepted by the runtime.')
      return
    }
  }
}
