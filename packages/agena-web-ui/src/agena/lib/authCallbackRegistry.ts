type AuthCallbackPayload = {
  code?: string
  state?: string
  error?: string
}

type AuthCallbackHandler = (payload: AuthCallbackPayload) => void | Promise<void>

let handler: AuthCallbackHandler | null = null

export function setAuthCallbackHandler(next: AuthCallbackHandler | null) {
  handler = next
}

export async function dispatchAuthCallback(payload: AuthCallbackPayload) {
  await handler?.(payload)
}
