import { captureResourceObservation, invalidateResourcePrefix, invalidateResources } from '../../lib/resourceSync'
import { sessionStateIsBusy, sessionStateNeedsAttention, type Session } from '../../types/chat'

// Routing hints from bodies we already received, not a second session cache.
// They let writes with an empty response reconcile one workspace without a
// preliminary GET. Bound them and retire them when backend/auth changes.
type Route = {
  workspaceId: number
  parentId: number | null
  version: number
  buckets: Set<string>
}
const routes = new Map<string, Route>()
const deleted = new Set<string>()
const deletionListeners = new Set<(ids: ReadonlySet<string>) => void>()
let routeScope = -1
export function sessionRoutingScope() {
  const scope = captureResourceObservation('sessions').scope
  if (scope !== routeScope) {
    routes.clear()
    deleted.clear()
    routeScope = scope
  }
  return scope
}

export function sessionWasDeleted(id: string) {
  sessionRoutingScope()
  return deleted.has(id)
}

export function subscribeSessionDeletions(callback: (ids: ReadonlySet<string>) => void) {
  deletionListeners.add(callback)
  return () => deletionListeners.delete(callback)
}

export function rememberSessionRoute(session: Session, scope: number) {
  if (scope !== sessionRoutingScope() || deleted.has(session.id)) return
  const workspaceId = Number(session.workspace_id)
  if (!Number.isSafeInteger(workspaceId) || workspaceId <= 0) return
  const version = Number(session.version) || 0
  if ((routes.get(session.id)?.version ?? 0) > version) return
  const parentId = Number(session.parent_id)
  const buckets = new Set<string>()
  if (session.pinned) buckets.add('pinned')
  if (session.favorite) buckets.add('favorite')
  if (sessionStateIsBusy(session.state)) buckets.add('running')
  if (sessionStateNeedsAttention(session.state)) buckets.add('attention')
  routes.delete(session.id)
  routes.set(session.id, {
    workspaceId,
    parentId: Number.isSafeInteger(parentId) && parentId > 0 ? parentId : null,
    version,
    buckets,
  })
  if (routes.size > 1024) routes.delete(routes.keys().next().value!)
}

export function reconcileSessionMutation(
  sessionId: string,
  change: 'created' | 'deleted' | 'metadata' | 'execution',
  scope: number,
  patch?: { title?: string; favorite?: boolean; pinned?: boolean },
) {
  if (scope !== sessionRoutingScope()) return
  const route = routes.get(sessionId)
  const keys = new Set<string>([`session:${sessionId}:state`])
  const membershipChanged = change !== 'metadata' || patch?.favorite !== undefined || patch?.pinned !== undefined
  const buckets = new Set(['recent', ...(route?.buckets ?? [])])
  if (!route || change === 'deleted')
    for (const bucket of ['pinned', 'favorite', 'running', 'attention']) buckets.add(bucket)
  if (change === 'execution') {
    buckets.add('running')
    buckets.add('attention')
    keys.add(`session:${sessionId}:parts`)
  }
  if (patch?.favorite !== undefined) buckets.add('favorite')
  if (patch?.pinned !== undefined) buckets.add('pinned')

  const addRow = (row: Route) => {
    const base = `workspace:${row.workspaceId}:sessions`
    keys.add(base)
    keys.add(row.parentId ? `${base}:parent:${row.parentId}` : `${base}:roots`)
    for (const bucket of row.buckets) {
      keys.add(`${base}:bucket:${bucket}`)
      keys.add(`sessions:bucket:${bucket}`)
    }
  }
  if (route) {
    addRow(route)
    if (membershipChanged) keys.add(`workspace:${route.workspaceId}:stats`)
    for (const bucket of buckets) keys.add(`workspace:${route.workspaceId}:sessions:bucket:${bucket}`)
    if ((change === 'created' || change === 'deleted') && route.parentId) {
      const parent = routes.get(String(route.parentId))
      if (parent) addRow(parent)
      else {
        // A deep parent may not have been loaded. Probe mounted branches in
        // this workspace; unchanged branches retain their existing bodies.
        invalidateResourcePrefix(`workspace:${route.workspaceId}:sessions:`)
        for (const bucket of ['pinned', 'favorite', 'running', 'attention']) keys.add(`sessions:bucket:${bucket}`)
      }
    }
    if (change === 'deleted') invalidateResourcePrefix(`workspace:${route.workspaceId}:sessions:`)
  } else {
    // An evicted routing hint never makes a successful write invisible.
    // This fallback checks observed workspace versions, not every list body.
    invalidateResourcePrefix('workspace:')
  }
  for (const bucket of buckets) {
    keys.add(`sessions:bucket:${bucket}`)
    if (membershipChanged) keys.add(`sessions:bucket:${bucket}:count`)
  }
  invalidateResources(keys)
  if (change === 'deleted') {
    const removed = new Set([sessionId])
    // A deletion cascades to descendants. Retire routing hints from any
    // loaded page so a pre-delete response cannot resurrect those rows.
    for (let previous = 0; previous !== removed.size; ) {
      previous = removed.size
      for (const [id, row] of routes) if (row.parentId && removed.has(String(row.parentId))) removed.add(id)
    }
    for (const id of removed) {
      routes.delete(id)
      deleted.add(id)
    }
    while (deleted.size > 1024) deleted.delete(deleted.values().next().value!)
    for (const listener of deletionListeners) listener(removed)
  }
}
