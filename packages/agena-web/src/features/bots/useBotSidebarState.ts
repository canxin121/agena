import { ref } from 'vue'

import { getLocalJson, setLocalJson } from '@/lib/persist'
import { localStorageKeys } from '@/lib/persistence/storageKeys'
import type { SessionOwnerSurface } from '@/types/chat'

export type BotSidebarGroupKey = SessionOwnerSurface | 'local-history'
type SidebarState = {
  bots: Record<string, boolean>
  groups: Record<string, Partial<Record<BotSidebarGroupKey, boolean>>>
}

function readState(): SidebarState {
  const stored = getLocalJson<Partial<SidebarState>>(localStorageKeys.ui.botSidebarTree, {})
  const bots: Record<string, boolean> = {}
  const groups: SidebarState['groups'] = {}
  if (stored && typeof stored === 'object') {
    for (const [id, open] of Object.entries(stored.bots || {})) {
      if (typeof open === 'boolean') bots[id] = open
    }
    for (const [id, values] of Object.entries(stored.groups || {})) {
      if (!values || typeof values !== 'object') continue
      const group: Partial<Record<BotSidebarGroupKey, boolean>> = {}
      for (const key of ['local', 'direct', 'group', 'broadcast', 'local-history'] as const) {
        if (typeof values[key] === 'boolean') group[key] = values[key]
      }
      groups[id] = group
    }
  }
  return { bots, groups }
}

export function useBotSidebarState() {
  const state = ref(readState())
  const save = () => setLocalJson(localStorageKeys.ui.botSidebarTree, state.value)

  function setBotOpen(id: string, open: boolean) {
    state.value.bots[id] = open
    save()
  }

  function setGroupOpen(id: string, key: BotSidebarGroupKey, open: boolean) {
    state.value.groups[id] = { ...state.value.groups[id], [key]: open }
    save()
  }

  return {
    hasBotState: (id: string) => Object.prototype.hasOwnProperty.call(state.value.bots, id),
    hasGroupState: (id: string, key: BotSidebarGroupKey) =>
      Object.prototype.hasOwnProperty.call(state.value.groups[id] || {}, key),
    isBotOpen: (id: string) => state.value.bots[id] ?? false,
    isGroupOpen: (id: string, key: BotSidebarGroupKey) => state.value.groups[id]?.[key] ?? key !== 'local-history',
    setBotOpen,
    setGroupOpen,
  }
}
