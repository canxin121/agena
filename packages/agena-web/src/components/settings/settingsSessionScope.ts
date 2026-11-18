/**
 * The conversation a session-scoped settings page edits.
 *
 * Settings pages are not conversations, so they cannot rely on the chat pane
 * on screen — a focused Files or Terminal pane has no session at all. Every
 * page shares one settings target: an explicit `targetSessionId` in this
 * settings window, or the conversation this client used last. The settings
 * target never opens a conversation or changes the chat selection.
 */
import { computed } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import { settingsText as st } from '@/i18n/settingsText'
import { lastSessionId } from '@/lib/lastSession'
import { useChatStoreUnscoped } from '@/stores/chat'

function normalizedId(value: unknown): string {
  // A repeated query value arrives as an array; use its first usable entry.
  const candidates = Array.isArray(value) ? value : [value]
  for (const candidate of candidates) {
    const id = String(candidate ?? '').trim()
    if (!/^\d+$/.test(id)) continue
    const numeric = Number(id)
    if (Number.isSafeInteger(numeric) && numeric > 0) return String(numeric)
  }
  return ''
}

/**
 * Normalize a positive conversation identifier without reading or changing
 * chat state. The retired `sessionId` settings query is never consulted.
 */
export function settingsSessionId(recorded: unknown): string {
  return normalizedId(recorded)
}

export function resolveSettingsSessionId(explicitTarget: unknown, recorded: unknown): string {
  return normalizedId(explicitTarget) || normalizedId(recorded)
}

export function useSettingsSessionTarget() {
  // The raw store owns the record. Reading it through a pane-scoped view would
  // report this settings pane's own (empty) route instead.
  const chat = useChatStoreUnscoped()
  const route = useRoute()
  const router = useRouter()

  // `lastSessionId` is the shared in-memory record; the raw store's selection
  // is only a fallback for a session that has not been recorded yet.
  const sessionId = computed(() =>
    resolveSettingsSessionId(route.query.targetSessionId, lastSessionId.value || chat.selectedSessionId),
  )

  // The target may not be on the loaded session page (it can be older than the
  // page the sidebar read), so its label never depends on that page.
  const selected = computed(() => {
    const id = sessionId.value
    if (!id) return null
    const session = chat.getSessionById(id)
    return {
      id,
      title: String(session?.title || st('Session {id}', { id })),
      project: chat.getSessionDirectory(id) || undefined,
    }
  })

  async function selectSession(value: string) {
    const id = settingsSessionId(value)
    if (!id || settingsSessionId(route.query.targetSessionId) === id) return
    const { sessionId: _retiredSessionId, ...query } = route.query
    // Route guards run before the target changes, so cancelling an unsaved
    // draft keeps the picker and every settings panel on the previous session.
    return await router.replace({ path: route.path, query: { ...query, targetSessionId: id }, hash: route.hash })
  }

  return { sessionId, selected, selectSession }
}
