import { computed, ref, shallowRef } from 'vue'
import type { Session } from '@/types/chat'
import type { BotProfile } from '@/lib/bots'
import type { OptionMenuGroup } from '@/components/ui/optionMenu.types'

export const SETTINGS_SESSION_PAGE_SIZE = 30

type SessionPage = { sessions: Session[]; total?: number; hasMore: boolean }
type CatalogDependencies = {
  listSessions: (options: {
    limit: number
    offset: number
    search?: string
    excludeSubagents: boolean
    includeTotal: boolean
    force: boolean
    signal: AbortSignal
  }) => Promise<SessionPage>
  listBots: (signal: AbortSignal) => Promise<{ items: BotProfile[] }>
  getSession: (id: string, signal: AbortSignal) => Promise<Session>
  cacheMetadata: (sessions: Session[]) => void
}

export function sessionCatalogId(value: unknown): string {
  const raw = String(value ?? '').trim()
  const numeric = Number(raw)
  return /^\d+$/.test(raw) && Number.isSafeInteger(numeric) && numeric > 0 ? String(numeric) : ''
}

function errorText(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause)
}

/** A settings-only catalog. Browsing it never opens a chat or changes recents. */
export function createSettingsSessionCatalog(dependencies: CatalogDependencies) {
  const sessions = shallowRef<Session[]>([])
  const bots = shallowRef<BotProfile[]>([])
  const selected = shallowRef<Session | null>(null)
  const selectedId = ref('')
  const query = ref('')
  const page = ref(1)
  const total = ref(0)
  const loading = ref(false)
  const botsLoading = ref(false)
  const selectedLoading = ref(false)
  const error = ref('')
  const botsError = ref('')
  const selectedError = ref('')
  const pageCount = computed(() => Math.max(1, page.value, Math.ceil(total.value / SETTINGS_SESSION_PAGE_SIZE)))
  let listGeneration = 0
  let botGeneration = 0
  let selectedGeneration = 0
  let listController: AbortController | null = null
  let botController: AbortController | null = null
  let selectedController: AbortController | null = null
  let searchTimer: ReturnType<typeof setTimeout> | undefined
  let disposed = false

  async function loadPage(target = page.value) {
    if (disposed) return
    clearTimeout(searchTimer)
    listController?.abort()
    const controller = new AbortController()
    listController = controller
    const generation = ++listGeneration
    const next = Math.max(1, Math.floor(target) || 1)
    page.value = next
    sessions.value = []
    error.value = ''
    loading.value = true
    try {
      const result = await dependencies.listSessions({
        limit: SETTINGS_SESSION_PAGE_SIZE,
        offset: (next - 1) * SETTINGS_SESSION_PAGE_SIZE,
        search: query.value.trim() || undefined,
        excludeSubagents: true,
        includeTotal: true,
        force: true,
        signal: controller.signal,
      })
      if (disposed || generation !== listGeneration) return
      sessions.value = result.sessions.filter(
        (session) =>
          sessionCatalogId(session.id) &&
          Number(session.workspace_id) > 0 &&
          !session.owner_bot_id &&
          session.relation_kind !== 'subagent',
      )
      const fallbackTotal = (next - 1) * SETTINGS_SESSION_PAGE_SIZE + sessions.value.length + (result.hasMore ? 1 : 0)
      total.value =
        typeof result.total === 'number' && Number.isFinite(result.total) ? Math.max(0, result.total) : fallbackTotal
    } catch (cause) {
      if (!disposed && generation === listGeneration && !controller.signal.aborted) error.value = errorText(cause)
    } finally {
      if (!disposed && generation === listGeneration) loading.value = false
    }
  }

  function search(value: string) {
    if (disposed) return
    query.value = value
    clearTimeout(searchTimer)
    listController?.abort()
    listGeneration++
    sessions.value = []
    page.value = 1
    total.value = 0
    error.value = ''
    loading.value = true
    searchTimer = setTimeout(() => void loadPage(1), 180)
  }

  async function loadBots() {
    if (disposed) return
    botController?.abort()
    const controller = new AbortController()
    botController = controller
    const generation = ++botGeneration
    botsLoading.value = true
    botsError.value = ''
    try {
      const result = await dependencies.listBots(controller.signal)
      if (disposed || generation !== botGeneration) return
      bots.value = result.items.filter((bot) => !bot.archived && sessionCatalogId(bot.local_session_id))
    } catch (cause) {
      if (!disposed && generation === botGeneration && !controller.signal.aborted) botsError.value = errorText(cause)
    } finally {
      if (!disposed && generation === botGeneration) botsLoading.value = false
    }
  }

  async function loadSelected(value: string) {
    const id = sessionCatalogId(value)
    selectedController?.abort()
    const generation = ++selectedGeneration
    selectedId.value = id
    selected.value = null
    selectedError.value = ''
    selectedLoading.value = false
    if (disposed || !id) return
    const controller = new AbortController()
    selectedController = controller
    selectedLoading.value = true
    try {
      const session = await dependencies.getSession(id, controller.signal)
      if (disposed || generation !== selectedGeneration) return
      if (sessionCatalogId(session.id) !== id) throw new Error('Invalid conversation response')
      selected.value = session
      dependencies.cacheMetadata([session])
    } catch (cause) {
      if (!disposed && generation === selectedGeneration && !controller.signal.aborted)
        selectedError.value = errorText(cause)
    } finally {
      if (!disposed && generation === selectedGeneration) selectedLoading.value = false
    }
  }

  async function refresh() {
    await Promise.all([loadPage(), loadBots(), loadSelected(selectedId.value)])
  }

  function dispose() {
    disposed = true
    listGeneration++
    botGeneration++
    selectedGeneration++
    clearTimeout(searchTimer)
    listController?.abort()
    botController?.abort()
    selectedController?.abort()
  }

  return {
    sessions,
    bots,
    selected,
    selectedId,
    query,
    page,
    total,
    pageCount,
    loading,
    botsLoading,
    selectedLoading,
    error,
    botsError,
    selectedError,
    loadPage,
    search,
    loadBots,
    loadSelected,
    refresh,
    dispose,
  }
}

export function settingsSessionGroups(options: {
  sessions: Session[]
  bots: BotProfile[]
  selectedId: string
  selected: Session | null
  query: string
  labels: { projects: string; bots: string; selected: string; session: (id: string) => string }
}): OptionMenuGroup[] {
  const groups: OptionMenuGroup[] = []
  const listed = new Set<string>()
  const projects = options.sessions.flatMap((session) => {
    const id = sessionCatalogId(session.id)
    if (!id || listed.has(id) || session.owner_bot_id) return []
    listed.add(id)
    return [{ id, label: String(session.title || options.labels.session(id)), description: options.labels.session(id) }]
  })
  if (projects.length) groups.push({ id: 'project-conversations', title: options.labels.projects, items: projects })
  const search = options.query.trim().toLowerCase()
  const bots = options.bots.flatMap((bot) => {
    const id = sessionCatalogId(bot.local_session_id)
    if (
      !id ||
      listed.has(id) ||
      bot.archived ||
      (search && ![id, bot.name, options.labels.bots].join(' ').toLowerCase().includes(search))
    )
      return []
    listed.add(id)
    return [{ id, label: bot.name, description: options.labels.session(id) }]
  })
  if (bots.length) groups.push({ id: 'bot-conversations', title: options.labels.bots, items: bots })
  const id = sessionCatalogId(options.selectedId)
  if (id && !listed.has(id))
    groups.unshift({
      id: 'selected-conversation',
      title: options.labels.selected,
      items: [
        {
          id,
          label: String(options.selected?.title || options.labels.session(id)),
          description: options.labels.session(id),
        },
      ],
    })
  return groups
}
