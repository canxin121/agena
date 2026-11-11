/**
 * The conversation a session-scoped settings page edits.
 *
 * Settings pages are not conversations, so they cannot rely on the chat pane
 * on screen — a focused Files or Terminal pane has no session at all. Every
 * page therefore resolves the conversation this client used last, which the
 * chat store records whenever any pane or bot binding opens one. There is no
 * per-page session picker and no `?sessionId=`: the last used conversation is
 * the target, so the same setting is edited from one place rather than from a
 * URL that different doors spell differently.
 */
import { computed } from 'vue'

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
    if (Number.isSafeInteger(numeric) && numeric > 0) return id
  }
  return ''
}

/**
 * The conversation session-scoped settings edit. `recorded` is the value the
 * client last opened; a URL naming a conversation is only a hint and never
 * outlives the record, so a deep link cannot silently retarget the page.
 */
export function settingsSessionId(recorded: unknown): string {
  return normalizedId(recorded)
}

export function useSettingsSessionTarget() {
  // The raw store owns the record. Reading it through a pane-scoped view would
  // report this settings pane's own (empty) route instead.
  const chat = useChatStoreUnscoped()

  // `lastSessionId` is the shared in-memory record; the raw store's selection
  // is only a fallback for a session that has not been recorded yet.
  const sessionId = computed(() => settingsSessionId(lastSessionId.value || chat.selectedSessionId))

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

  return { sessionId, selected }
}
