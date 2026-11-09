import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { useWorkspacePaneContext } from '@/app/workspace/workspacePaneContext'
import { botIdQuery, resolveBotSession, type BotProfile } from '@/lib/bots'
import { useBotsStore } from '@/stores/bots'
import { useChatStore } from '@/stores/chat'
import { useUiStore } from '@/stores/ui'
import { router as shellRouter } from '@/router'
import { matchesConversationRoute } from '@/app/navigation/conversationOwnership'

export function useBotConversation() {
  const { t } = useI18n()
  const route = useRoute()
  const router = useRouter()
  const pane = useWorkspacePaneContext()
  const ui = useUiStore()
  const chat = useChatStore()
  const bots = useBotsStore()
  const isBot = computed(() => route.path === '/bot')
  const requestedId = computed(() => botIdQuery(route.query))
  const target = computed(() => (pane ? ui.getWorkspaceWindowById(pane.windowId.value) : null))
  const targetPath = computed(() => (pane ? target.value?.routePath || '' : route.path))
  const targetId = computed(() => (pane ? botIdQuery(target.value?.routeQuery || {}) : requestedId.value))
  const documentActive = ref(true)
  const leavingDocument = () => {
    documentActive.value = false
  }
  const enteringDocument = () => {
    documentActive.value = true
  }
  onMounted(() => {
    window.addEventListener('beforeunload', leavingDocument)
    window.addEventListener('pagehide', leavingDocument)
    window.addEventListener('pageshow', enteringDocument)
  })
  onBeforeUnmount(() => {
    window.removeEventListener('beforeunload', leavingDocument)
    window.removeEventListener('pagehide', leavingDocument)
    window.removeEventListener('pageshow', enteringDocument)
  })
  const sessionId = ref<string | null>(null)
  const botId = ref<string | null>(null)
  const name = ref('')
  const loading = ref(false)
  const error = ref('')
  let binding: { initialId: string; id: string; generation: number } | null = null
  let generation = 0
  let request: AbortController | null = null
  function cancel() {
    generation++
    binding = null
    request?.abort()
    request = null
  }
  async function load(force = false) {
    if (
      !force &&
      binding &&
      binding.generation === generation &&
      isBot.value &&
      targetPath.value === '/bot' &&
      [binding.initialId, binding.id].includes(requestedId.value) &&
      [binding.initialId, binding.id].includes(targetId.value)
    ) {
      // The scoped pane may commit its route after router.replace resolves.
      // Retain ownership until both route authorities have adopted the binding.
      if (
        requestedId.value === binding.id &&
        targetId.value === binding.id &&
        route.query.sessionId === sessionId.value &&
        (!pane || target.value?.routeQuery.sessionId === sessionId.value)
      )
        binding = null
      return
    }
    // Adding the server's binding to the URL doesn't issue a second ensure.
    if (
      !force &&
      isBot.value &&
      botId.value &&
      requestedId.value === botId.value &&
      targetId.value === botId.value &&
      sessionId.value &&
      chat.selectedSessionId === sessionId.value &&
      (!pane || target.value?.routeQuery.sessionId === sessionId.value) &&
      !error.value
    )
      return
    cancel()
    sessionId.value = null
    botId.value = null
    name.value = ''
    error.value = ''
    loading.value = isBot.value
    if (!isBot.value || targetPath.value !== '/bot' || targetId.value !== requestedId.value) return
    const sequence = generation
    const selectedId = requestedId.value
    const windowId = pane?.windowId.value
    const abort = new AbortController()
    request = abort
    const isCurrent = () =>
      sequence === generation &&
      !abort.signal.aborted &&
      isBot.value &&
      targetPath.value === '/bot' &&
      requestedId.value === selectedId &&
      targetId.value === selectedId &&
      pane?.windowId.value === windowId
    try {
      const resolved = await resolveBotSession(selectedId || undefined, abort.signal)
      if (!isCurrent()) return
      const id = String(resolved.session_id)
      await chat.hydrateSession(id, { windowId, isCurrent })
      if (!isCurrent()) return
      if (!pane) await chat.selectSession(id)
      if (!isCurrent()) return
      const ownBinding = { initialId: selectedId, id: resolved.bot_id, generation: sequence }
      binding = ownBinding
      sessionId.value = id
      botId.value = resolved.bot_id
      name.value = resolved.name
      loading.value = false
      await router.replace({
        path: '/bot',
        query: {
          ...(windowId ? { windowId } : {}),
          botId: resolved.bot_id,
          sessionId: id,
          ...(route.query.editPersona === '1' ? { editPersona: '1' } : {}),
        },
      })
      if (
        binding === ownBinding &&
        requestedId.value === resolved.bot_id &&
        targetId.value === resolved.bot_id &&
        route.query.sessionId === id &&
        (!pane || target.value?.routeQuery.sessionId === id)
      )
        binding = null
      return resolved
    } catch (cause) {
      if (isCurrent()) {
        const message = cause instanceof Error ? cause.message : String(cause)
        error.value =
          message === 'Invalid Bot ID'
            ? t('bot.profiles.invalidId')
            : message === 'Invalid Bot session response'
              ? t('bot.profiles.invalidResponse')
              : message
      }
    } finally {
      if (sequence === generation) loading.value = false
    }
  }
  watch(
    [isBot, requestedId, targetPath, targetId, () => route.query.sessionId, () => target.value?.routeQuery.sessionId],
    () => void load(),
    { immediate: true },
  )
  onBeforeUnmount(cancel)
  const ready = computed(
    () =>
      !isBot.value ||
      (targetPath.value === '/bot' &&
        !loading.value &&
        !error.value &&
        !!botId.value &&
        requestedId.value === botId.value &&
        targetId.value === botId.value &&
        !!sessionId.value &&
        chat.selectedSessionId === sessionId.value),
  )
  const ownsConversation = () => {
    if (targetPath.value !== route.path || !ready.value) return false
    if (!pane) return true
    return !!target.value && (target.value.routeQuery.sessionId || null) === chat.selectedSessionId
  }
  const canSendConversation = () => {
    if (!documentActive.value || !ownsConversation()) return false
    if (pane && (!pane.isVisible.value || !pane.isFocused.value)) return false
    const id = pane?.windowId.value
    const sid = chat.selectedSessionId
    if (!matchesConversationRoute(shellRouter.currentRoute.value, route.path, sid, id, botId.value)) return false
    const url = new URL(window.location.href)
    return matchesConversationRoute(
      { path: url.pathname, query: Object.fromEntries(url.searchParams) },
      route.path,
      sid,
      id,
      botId.value,
    )
  }
  function updateProfile(profile: BotProfile) {
    if (profile.id !== botId.value) return
    if (profile.archived) {
      void load(true)
      return
    }
    name.value = profile.name
  }
  watch(
    () => bots.profiles,
    (profiles) => {
      const profile = profiles.find((p) => p.id === botId.value)
      if (profile) updateProfile(profile)
    },
  )
  watch(
    () => bots.authoritative,
    () => {
      if (botId.value && !bots.profiles.some((p) => p.id === botId.value)) {
        cancel()
        sessionId.value = null
        name.value = ''
        error.value = t('bot.profiles.archived')
      }
    },
  )
  watch(
    () => bots.version,
    () => {
      cancel()
      sessionId.value = null
      botId.value = null
      name.value = ''
      if (isBot.value && requestedId.value) void load(true)
      else if (isBot.value) {
        loading.value = false
        error.value = t('bot.profiles.choose')
      }
    },
    { flush: 'sync' },
  )
  // Switching is pane-local navigation. It never updates the server's default.
  async function select(id: string) {
    cancel()
    sessionId.value = null
    botId.value = null
    loading.value = true
    await router.push({ path: '/bot', query: { botId: id, ...(pane ? { windowId: pane.windowId.value } : {}) } })
    if (requestedId.value === id && targetId.value === id && !request) await load()
  }
  return {
    isBot,
    botId,
    name,
    ready,
    loading,
    error,
    retry: () => load(true),
    select,
    updateProfile,
    ownsConversation,
    canSendConversation,
  }
}
