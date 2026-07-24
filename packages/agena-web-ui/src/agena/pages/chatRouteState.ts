export function readChatRouteSessionId(value: unknown): number | null {
  if (typeof value !== 'string') return null
  const parsed = Number(value)
  return Number.isFinite(parsed) ? parsed : null
}

export function readChatRouteWorkspaceId(value: unknown): number | null {
  if (typeof value !== 'string') return null
  const parsed = Number(value)
  return Number.isFinite(parsed) ? parsed : null
}

export function readChatRouteSlash(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}
