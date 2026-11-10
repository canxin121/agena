/**
 * The session a session-scoped settings page edits, and the grouped picker that
 * chooses it.
 *
 * Settings pages are not conversations, so they cannot rely on a chat tab's
 * selection being on screen. They resolve it from the conversation this client
 * used last — the chat store persists that choice across reloads — unless the
 * URL names one explicitly, so a shared link opens the same conversation.
 * Conversations are browsed by project and paged, using the same catalog the
 * sidebar reads, so both surfaces describe one list.
 */
import { computed, onMounted } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import { settingsText as st } from '@/i18n/settingsText'
import { useChatStore } from '@/stores/chat'
import { sessionGroupsForPicker, usePagedSessionGroups } from './sessionCatalog'

export const SETTINGS_SESSION_QUERY_KEY = 'sessionId'

function normalizedId(value: unknown): string {
  const raw = Array.isArray(value) ? value.find((item) => typeof item === 'string') : value
  const id = String(raw || '').trim()
  if (!/^\d+$/.test(id)) return ''
  const numeric = Number(id)
  return Number.isSafeInteger(numeric) && numeric > 0 ? id : ''
}

/** URL first (shareable), then the conversation this client used last. */
export function settingsSessionId(requested: unknown, lastUsed: unknown): string {
  return normalizedId(requested) || normalizedId(lastUsed)
}

export function useSettingsSessionTarget() {
  const route = useRoute()
  const router = useRouter()
  const chat = useChatStore()

  const requested = computed(() => normalizedId(route.query[SETTINGS_SESSION_QUERY_KEY]))
  const sessionId = computed(() => settingsSessionId(requested.value, chat.selectedSessionId))

  // One page of conversations, grouped under collapsible project headers. The
  // pager requests another page from the server rather than growing the list.
  const catalog = usePagedSessionGroups()
  const pickerGroups = computed(() =>
    sessionGroupsForPicker(catalog.groups.value, { selectedId: sessionId.value }),
  )

  // The chosen conversation may live on another page or may have just opened,
  // so its label never depends on the page currently loaded.
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

  function select(id: string) {
    const next = normalizedId(id)
    if (next === requested.value) return
    const query = { ...route.query }
    if (next) query[SETTINGS_SESSION_QUERY_KEY] = next
    else delete query[SETTINGS_SESSION_QUERY_KEY]
    void router.replace({ path: route.path, query, hash: route.hash })
  }

  onMounted(() => {
    if (!catalog.sessions.value.length) void catalog.load(1)
  })

  return { sessionId, selected, pickerGroups, catalog, select }
}
