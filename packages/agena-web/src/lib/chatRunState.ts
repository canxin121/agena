export function normalizeRunState(value: unknown): string {
  return typeof value === 'string' ? value.trim().toLowerCase() : ''
}

export function isKnownRunState(value: unknown): boolean {
  switch (normalizeRunState(value)) {
    case 'pending':
    case 'in_progress':
    case 'running':
    case 'completed':
    case 'policy_denied':
    case 'user_declined':
    case 'capability_unavailable':
    case 'tool_unavailable':
    case 'failed':
    case 'error':
    case 'cancelled':
    case 'canceled':
      return true
    default:
      return false
  }
}

export function isRunInFlight(value: unknown): boolean {
  const state = normalizeRunState(value)
  return state === 'pending' || state === 'in_progress' || state === 'running'
}

export function isRunFailureState(value: unknown): boolean {
  const state = normalizeRunState(value)
  return (
    state === 'failed' ||
    state === 'error' ||
    state === 'policy_denied' ||
    state === 'user_declined' ||
    state === 'capability_unavailable' ||
    state === 'tool_unavailable'
  )
}

export function isRunCancelled(value: unknown): boolean {
  const state = normalizeRunState(value)
  return state === 'cancelled' || state === 'canceled'
}

export function isRunTerminal(value: unknown): boolean {
  return normalizeRunState(value) === 'completed' || isRunFailureState(value) || isRunCancelled(value)
}

export function isAssistantMessageStreaming(
  input:
    | {
        role?: unknown
        runState?: unknown
        finish?: unknown
        error?: unknown
      }
    | null
    | undefined,
): boolean {
  if (!input || normalizeRunState(input.role) !== 'assistant' || input.error) return false
  return isRunInFlight(input.runState || input.finish)
}
