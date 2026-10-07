import { apiJson } from './api'
import { captureResourceObservation, invalidateResourcePrefix, invalidateResources } from './resourceSync'
import { activityIsActive, type SessionActivity } from '../types/activity'

type ActivityMutation = {
  activity: SessionActivity
  dismissed: boolean
  observation: ReturnType<typeof captureResourceObservation>
}
const mutationListeners = new Set<(mutation: ActivityMutation) => void>()

/** All mounted activity lists can consume a control response immediately. */
export function subscribeActivityMutations(callback: (mutation: ActivityMutation) => void) {
  mutationListeners.add(callback)
  return () => mutationListeners.delete(callback)
}

export async function controlActivity(id: string, action: string, signal?: AbortSignal): Promise<SessionActivity> {
  const observation = captureResourceObservation('activities')
  const result = await apiJson<SessionActivity>(
    `/api/v1/activities/${encodeURIComponent(id)}/${encodeURIComponent(action)}`,
    { method: 'POST', signal: signal ?? AbortSignal.timeout(15_000) },
  )
  if (observation.scope === captureResourceObservation('activities').scope) {
    const mutation = { activity: result, dismissed: action === 'dismiss' || action === 'delete', observation }
    for (const callback of mutationListeners) {
      try {
        callback(mutation)
      } catch (error) {
        console.error('Activity mutation projection failed', error)
      }
    }
    invalidateResources(
      [
        'activities',
        `activity:${id}`,
        `activity:${id}:logs`,
        ...[result.session_id, result.parent_session_id]
          .filter((sid) => typeof sid === 'number' && sid > 0)
          .map((sid) => `session:${sid}:state`),
      ],
      'mutation',
    )
  }
  return result
}

export async function clearFinishedActivities(
  activities: readonly {
    status: string
    session_id?: number | null
    parent_session_id?: number | null
  }[],
): Promise<void> {
  const scope = captureResourceObservation('activities').scope
  await apiJson('/api/v1/activities/clear-finished', { method: 'POST', signal: AbortSignal.timeout(15_000) })
  if (scope !== captureResourceObservation('activities').scope) return
  invalidateResources([
    'activities',
    ...activities
      .filter((row) => !activityIsActive(row.status))
      .flatMap((row) => [row.session_id, row.parent_session_id])
      .filter((sid) => typeof sid === 'number' && sid > 0)
      .map((sid) => `session:${sid}:state`),
  ])
  // The response contains a count, not removed IDs. Check mounted descriptor
  // versions; no unopened logs or conversations are read by this fallback.
  invalidateResourcePrefix('activity:')
}
