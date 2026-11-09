import { defineStore } from 'pinia'
import { ref } from 'vue'
import { listBots, type BotProfile } from '@/lib/bots'
import { subscribeServerAuthTokenVersion } from '@/lib/serverAuthToken'
import { reconcileBots } from '@/features/bots/profiles'
import { useUiStore } from '@/stores/ui'

export const useBotsStore = defineStore('bots', () => {
  const profiles = ref<BotProfile[]>([])
  const loading = ref(false)
  const error = ref('')
  const request = ref<{
    kind: 'manager' | 'create' | 'edit' | 'rename' | 'archive'
    id?: string
    serial: number
  } | null>(null)
  const version = ref(0)
  const authoritative = ref(0)
  const managerOpen = ref(false)
  const writes = new Map<string, BotProfile>()
  let sequence = 0
  let serial = 0
  let controller: AbortController | null = null
  let pending: Promise<void> | null = null
  function reset() {
    version.value++
    sequence++
    controller?.abort()
    controller = null
    pending = null
    writes.clear()
    profiles.value = []
    error.value = ''
    loading.value = false
    request.value = null
    managerOpen.value = false
  }
  async function refresh(force = false) {
    if (pending && !force) return pending
    controller?.abort()
    const abort = new AbortController()
    controller = abort
    const current = ++sequence
    loading.value = true
    error.value = ''
    const task = (async () => {
      try {
        const result = await listBots(abort.signal)
        if (current !== sequence) return
        const listed = new Set(result.items.map((p) => p.id.toLowerCase()))
        for (const [id, profile] of writes) {
          if (!listed.has(id) && !profile.archived) writes.set(id, { ...profile, archived: true })
        }
        profiles.value = reconcileBots(result.items, writes)
        authoritative.value++
        for (const profile of profiles.value) {
          for (const tab of useUiStore().workspaceWindows) {
            if (tab.mainTab === 'bot' && tab.routeQuery.botId === profile.id)
              useUiStore().setWorkspaceWindowTitle(tab.id, profile.name)
          }
        }
      } catch (cause) {
        if (current === sequence && !abort.signal.aborted)
          error.value = cause instanceof Error ? cause.message : String(cause)
      } finally {
        if (current === sequence) {
          loading.value = false
          pending = null
          controller = null
        }
      }
    })()
    pending = task
    return task
  }
  function apply(profile: BotProfile) {
    const old = profiles.value.find((p) => p.id.toLowerCase() === profile.id.toLowerCase())
    if (old && old.revision > profile.revision) return
    const written = writes.get(profile.id.toLowerCase())
    if (
      written &&
      (written.revision > profile.revision ||
        (written.archived && !profile.archived && written.revision === profile.revision))
    )
      return
    sequence++
    controller?.abort()
    controller = null
    pending = null
    loading.value = false
    writes.set(profile.id.toLowerCase(), profile)
    profiles.value = profiles.value.filter((p) => p.id.toLowerCase() !== profile.id.toLowerCase())
    if (!profile.archived) profiles.value.push(profile)
    else authoritative.value++
    for (const tab of useUiStore().workspaceWindows) {
      if (tab.mainTab === 'bot' && tab.routeQuery.botId === profile.id)
        useUiStore().setWorkspaceWindowTitle(tab.id, profile.name)
    }
  }
  function open(kind: 'manager' | 'create' | 'edit' | 'rename' | 'archive', id?: string) {
    request.value = { kind, id, serial: ++serial }
  }
  function start() {
    const focus = () => {
      void refresh()
    }
    const auth = subscribeServerAuthTokenVersion(() => {
      reset()
      void refresh()
    })
    window.addEventListener('focus', focus)
    window.addEventListener('agena:bots-changed', focus)
    void refresh()
    return () => {
      auth()
      window.removeEventListener('focus', focus)
      window.removeEventListener('agena:bots-changed', focus)
      reset()
    }
  }
  return { profiles, loading, error, request, version, authoritative, managerOpen, refresh, apply, open, start }
})
