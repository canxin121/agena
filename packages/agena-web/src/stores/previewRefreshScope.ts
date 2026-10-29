import { normalizeWindowScopeId } from '@/app/windowScope'

export type PreviewRefreshScope = {
  sequence: number
  byWindow: Record<string, number>
  bySession: Record<string, number>
}

export function createPreviewRefreshScope(): PreviewRefreshScope {
  return { sequence: 0, byWindow: {}, bySession: {} }
}

/** A manual reload belongs to one pane; a changed service belongs to its viewers. */
export function readPreviewRefreshToken(
  scope: PreviewRefreshScope,
  windowId: string | null | undefined,
  sessionId: string,
): number {
  const windowKey = normalizeWindowScopeId(windowId)
  return Math.max(
    Object.hasOwn(scope.byWindow, windowKey) ? scope.byWindow[windowKey]! : 0,
    Object.hasOwn(scope.bySession, sessionId) ? scope.bySession[sessionId]! : 0,
  )
}

export function refreshPreviewWindow(scope: PreviewRefreshScope, windowId?: string | null): PreviewRefreshScope {
  const sequence = scope.sequence + 1
  return { ...scope, sequence, byWindow: { ...scope.byWindow, [normalizeWindowScopeId(windowId)]: sequence } }
}

export function refreshPreviewSession(scope: PreviewRefreshScope, sessionId: string): PreviewRefreshScope {
  const sequence = scope.sequence + 1
  return { ...scope, sequence, bySession: { ...scope.bySession, [sessionId]: sequence } }
}

export function forgetPreviewRefreshSession(scope: PreviewRefreshScope, sessionId: string): PreviewRefreshScope {
  const { [sessionId]: _removed, ...bySession } = scope.bySession
  return { ...scope, bySession }
}
