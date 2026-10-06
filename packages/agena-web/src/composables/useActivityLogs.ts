import type { Ref } from 'vue'
import { conditionalJsonObserved } from '@/lib/conditionalJson'
import { useVisibleResource } from '@/pages/chat/useVisibleResource'
import { mergeActivityLog, type ActivityLog, type SessionActivity } from '@/types/activity'

/** The dock and expanded launch receipt share the activity's log clock. */
export function useActivityLogs(key: Ref<string>, activity: () => SessionActivity | null) {
  const logs = useVisibleResource<ActivityLog>({
    key,
    resource: (key) => `activity:${key.slice(key.indexOf('/') + 1)}:logs`,
    minInterval: 1000,
    // Quiet active processes need no polling; real log clocks wake the read.
    interval: (): number | null => (logs.data.value?.has_more ? 1000 : null),
    async load(key, signal, previous, force, observe) {
      const id = key.slice(key.indexOf('/') + 1)
      const cursor = previous?.last_seq ?? Math.max(0, (activity()?.last_seq || 0) - 200)
      const { value: next, observation } = await conditionalJsonObserved<ActivityLog>(
        `activity:${id}:logs`,
        `/api/v1/activities/${encodeURIComponent(id)}/logs?since_seq=${cursor}&limit=200`,
        { signal },
        force,
      )
      observe(observation)
      return mergeActivityLog(previous, next)
    },
  })
  return logs
}
