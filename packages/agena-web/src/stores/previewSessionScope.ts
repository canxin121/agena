import { DEFAULT_WINDOW_SCOPE_ID, normalizeWindowScopeId } from '@/app/windowScope'

/**
 * Which preview session each workspace window is showing.
 *
 * Preview panes are independent surfaces: selecting a session in one must not
 * re-point another pane's iframe.  The active session is tracked per window
 * scope, with the persisted value kept as the default scope that a pane inherits
 * until it chooses - and that a single-pane or compact shell keeps using.
 */

export type PreviewSessionScope = {
  defaultId: string
  byWindow: Record<string, string>
}

export function createPreviewSessionScope(defaultId = ''): PreviewSessionScope {
  return { defaultId: String(defaultId || '').trim(), byWindow: {} }
}

function normalizeId(value: unknown): string {
  return String(value || '').trim()
}

export function readPreviewSessionId(scope: PreviewSessionScope, windowId?: string | null): string {
  const scoped = normalizeId(scope.byWindow[normalizeWindowScopeId(windowId)])
  return scoped || normalizeId(scope.defaultId)
}

export function writePreviewSessionId(
  scope: PreviewSessionScope,
  windowId: string | null | undefined,
  sessionId: string,
): PreviewSessionScope {
  const scopeId = normalizeWindowScopeId(windowId)
  const id = normalizeId(sessionId)
  const byWindow = { ...scope.byWindow }
  if (id) byWindow[scopeId] = id
  else delete byWindow[scopeId]

  return {
    // The default scope is the persisted one, so it stays in step with the last
    // explicit choice; a pane remembers its own id separately.
    defaultId: scopeId === DEFAULT_WINDOW_SCOPE_ID ? id : scope.defaultId,
    byWindow,
  }
}

/** Drop ids that no longer exist, so a deleted session cannot stay selected. */
export function prunePreviewSessionScope(
  scope: PreviewSessionScope,
  knownSessionIds: Iterable<string>,
): PreviewSessionScope {
  const known = new Set<string>()
  for (const id of knownSessionIds) {
    const value = normalizeId(id)
    if (value) known.add(value)
  }

  const byWindow: Record<string, string> = {}
  let changed = false
  for (const [scopeId, value] of Object.entries(scope.byWindow)) {
    const id = normalizeId(value)
    if (!id || !known.has(id)) {
      changed = true
      continue
    }
    byWindow[scopeId] = id
  }

  const defaultId = normalizeId(scope.defaultId)
  const nextDefault = defaultId && !known.has(defaultId) ? '' : defaultId
  if (!changed && nextDefault === defaultId) return scope
  return { defaultId: nextDefault, byWindow }
}

export function scopeHoldsPreviewSession(scope: PreviewSessionScope, sessionId: string): boolean {
  const id = normalizeId(sessionId)
  if (!id) return false
  if (normalizeId(scope.defaultId) === id) return true
  return Object.values(scope.byWindow).includes(id)
}

export function clearPreviewSessionId(scope: PreviewSessionScope, sessionId: string): PreviewSessionScope {
  const id = normalizeId(sessionId)
  if (!id) return scope

  const byWindow: Record<string, string> = {}
  let changed = false
  for (const [scopeId, value] of Object.entries(scope.byWindow)) {
    if (normalizeId(value) === id) {
      changed = true
      continue
    }
    byWindow[scopeId] = value
  }

  const previousDefault = normalizeId(scope.defaultId)
  const defaultId = previousDefault === id ? '' : previousDefault
  if (!changed && defaultId === previousDefault) return scope
  return { defaultId, byWindow }
}

export function renamePreviewSessionId(
  scope: PreviewSessionScope,
  previousId: string,
  nextId: string,
): { scope: PreviewSessionScope; replaced: boolean } {
  const previous = normalizeId(previousId)
  const next = normalizeId(nextId)
  if (!previous || !next) return { scope, replaced: false }

  let replaced = false
  const byWindow: Record<string, string> = {}
  for (const [scopeId, value] of Object.entries(scope.byWindow)) {
    if (normalizeId(value) === previous) {
      byWindow[scopeId] = next
      replaced = true
      continue
    }
    byWindow[scopeId] = value
  }

  const previousDefault = normalizeId(scope.defaultId)
  const defaultId = previousDefault === previous ? next : previousDefault
  if (!replaced && defaultId === previousDefault) return { scope, replaced: false }
  return { scope: { defaultId, byWindow }, replaced: true }
}
