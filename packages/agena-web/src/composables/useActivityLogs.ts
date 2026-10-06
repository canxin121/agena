import type { Ref } from 'vue'
import { conditionalJson } from '@/lib/conditionalJson'
import { useVisibleResource } from '@/pages/chat/useVisibleResource'
import { mergeActivityLog, type ActivityLog, type SessionActivity } from '@/types/activity'

/** The dock and expanded launch receipt read the same activity resource. */
export function useActivityLogs(key: Ref<string>, activity: () => SessionActivity | null) {
  const logs = useVisibleResource<ActivityLog>({
    key,
    resource: (key) => `activity:${key.slice(key.indexOf('/') + 1)}`,
    minInterval: 1000,
    // Quiet active processes need no polling; real log clocks wake the read.
    interval: (): number | null => (logs.data.value?.has_more ? 1000 : null),
    async load(key, signal, previous, force) {
      const id = key.slice(key.indexOf('/') + 1)
      const cursor = previous?.last_seq ?? Math.max(0, (activity()?.last_seq || 0) - 200)
      const next = await conditionalJson<ActivityLog>(
        `activity:${id}`,
        `/api/v1/activities/${encodeURIComponent(id)}/logs?since_seq=${cursor}&limit=200`,
        { signal },
        force,
      )
      return mergeActivityLog(previous, next)
    },
  })
  return logs
}
