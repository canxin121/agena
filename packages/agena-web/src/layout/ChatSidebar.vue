<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { RiAddLine, RiDeleteBinLine, RiLoader4Line, RiRefreshLine } from '@remixicon/vue'

import RenameSessionDialog from '@/components/chat/RenameSessionDialog.vue'
import IconButton from '@/components/ui/IconButton.vue'
import OptionMenu from '@/components/ui/OptionMenu.vue'
import type { OptionMenuGroup, OptionMenuItem } from '@/components/ui/optionMenu.types'
import SessionRow from '@/layout/chatSidebar/components/SessionRow.vue'
import {
  buildSessionActionItemsForSessionI18n,
  useSessionActionMenu,
  type SessionActionItem,
} from '@/layout/chatSidebar/useSessionActionMenu'
import { useChatStore } from '@/stores/chat'
import { useToastsStore } from '@/stores/toasts'
import { useUiStore } from '@/stores/ui'

const props = withDefaults(
  defineProps<{
    mobileVariant?: boolean
    navigateToChat?: boolean
  }>(),
  {
    mobileVariant: false,
    navigateToChat: true,
  },
)

const route = useRoute()
const router = useRouter()

const ui = useUiStore()
const chat = useChatStore()
const toasts = useToastsStore()
const { t } = useI18n()

// ─── session list ───────────────────────────────────────────────────────────

onMounted(() => {
  void chat.refreshSessions().catch(() => {})
})

// Keep the flat list fresh when navigating into the sidebar.
watch(
  () => route.fullPath,
  () => {
    void chat.refreshSessions().catch(() => {})
  },
)

const creatingSession = ref(false)

async function selectSession(sessionId: string) {
  const sid = String(sessionId || '').trim()
  if (!sid) return
  await chat.selectSession(sid)
  ui.setGlobalSelection('chat-session', sid, { meta: { source: 'chat-sidebar' } })
  if (props.mobileVariant) ui.setSessionSwitcherOpen(false)
  if (props.navigateToChat) {
    await router.push('/chat').catch(() => {})
  }
}

async function createNewSession() {
  if (creatingSession.value) return
  creatingSession.value = true
  try {
    const created = await chat.createSession()
    if (created?.id) {
      await chat.selectSession(String(created.id))
      ui.setGlobalSelection('chat-session', String(created.id), { meta: { source: 'chat-sidebar' } })
      if (props.mobileVariant) ui.setSessionSwitcherOpen(false)
      if (props.navigateToChat) {
        await router.push('/chat').catch(() => {})
      }
    }
  } catch (err) {
    toasts.push('error', err instanceof Error ? err.message : String(err))
  } finally {
    creatingSession.value = false
  }
}

async function deleteSession(sessionId: string) {
  const sid = (sessionId || '').trim()
  if (!sid) return
  await chat.deleteSession(sid)
  void chat.refreshSessions().catch(() => {})
}

// ─── session action menu (desktop) + mobile dialog ──────────────────────────

const {
  sessionActionMenuAnchorRef,
  sessionActionMenuQuery,
  sessionActionMenuTarget,
  filteredSessionActionItems,
  openSessionActionMenu,
  runSessionActionMenu: runSessionActionMenuBase,
  setSessionActionMenuRef,
} = useSessionActionMenu({ chat, ui, selectSession })

type SessionDialogActionItem = SessionActionItem &
  Pick<OptionMenuItem, 'variant' | 'confirmTitle' | 'confirmDescription' | 'confirmText' | 'cancelText'>

const sessionActionsOpen = ref(false)
const sessionActionsTarget = ref<{ id: string } | null>(null)
const sessionActionsDialogQuery = ref('')

const sessionActionsDialogItems = computed<SessionDialogActionItem[]>(() => {
  const base: SessionDialogActionItem[] = [
    {
      id: 'delete',
      label: String(t('chat.sidebar.sessionActions.delete.label')),
      description: String(t('chat.sidebar.sessionActions.delete.description')),
      icon: RiDeleteBinLine,
      variant: 'destructive',
      confirmTitle: String(t('chat.sidebar.sessionActions.delete.confirmTitle')),
      confirmDescription: String(t('chat.sidebar.sessionActions.delete.confirmDescription')),
      confirmText: String(t('chat.sidebar.sessionActions.delete.confirmText')),
      cancelText: String(t('common.cancel')),
    },
  ]
  return [...base, ...buildSessionActionItemsForSessionI18n(t)]
})

const filteredSessionActionsDialogItems = computed<SessionDialogActionItem[]>(() => {
  const q = sessionActionsDialogQuery.value.trim().toLowerCase()
  const list = sessionActionsDialogItems.value
  if (!q) return list
  return list.filter((item) => {
    const label = item.label.toLowerCase()
    const desc = String(item.description || '').toLowerCase()
    return label.includes(q) || desc.includes(q) || item.id.includes(q)
  })
})

const sessionActionMenuGroups = computed<OptionMenuGroup[]>(() => [
  {
    id: 'session-actions',
    items: filteredSessionActionsDialogItems.value as OptionMenuItem[],
  },
])

const renameDialogOpen = ref(false)
const isSidebarDialogOpen = computed(() => Boolean(sessionActionsOpen.value || renameDialogOpen.value))

function openSessionActions(session: { id: string }) {
  sessionActionsTarget.value = session
  sessionActionsDialogQuery.value = ''
  sessionActionsOpen.value = true
}

// ─── rename (dialog on mobile, inline on desktop) ───────────────────────────

const renameBusy = ref(false)
const renameDraft = ref('')
const renameTargetSessionId = ref('')

function resetRenameState() {
  renameDialogOpen.value = false
  renameBusy.value = false
  renameDraft.value = ''
  renameTargetSessionId.value = ''
}

function beginRenameForSession(session: { id: string; title?: string }, mode: 'dialog' | 'inline') {
  const sid = String(session?.id || '').trim()
  if (!sid) return
  const title = typeof session?.title === 'string' ? session.title.trim() : ''
  renameTargetSessionId.value = sid
  renameDraft.value = title || sid
  renameBusy.value = false
  renameDialogOpen.value = mode === 'dialog'
}

function isRenamingSession(sessionId: string): boolean {
  const sid = String(sessionId || '').trim()
  if (!sid) return false
  if (props.mobileVariant) return false
  return !renameDialogOpen.value && sid === renameTargetSessionId.value.trim()
}

function updateRenameDraft(next: string) {
  renameDraft.value = String(next || '')
}

function cancelRenameFromSidebar() {
  resetRenameState()
}

async function saveRenameFromSidebar() {
  const sid = renameTargetSessionId.value.trim()
  const next = renameDraft.value.trim()
  if (!sid) return
  if (!next) {
    toasts.push('error', String(t('chat.toasts.titleCannotBeEmpty')))
    return
  }
  renameBusy.value = true
  try {
    await chat.renameSession(sid, next)
    resetRenameState()
    toasts.push('success', String(t('chat.toasts.sessionRenamed')))
  } catch (err) {
    toasts.push('error', err instanceof Error ? err.message : String(err))
  } finally {
    renameBusy.value = false
  }
}

function closeDesktopSessionActionMenu() {
  sessionActionMenuTarget.value = null
  sessionActionMenuAnchorRef.value = null
  sessionActionMenuQuery.value = ''
}

async function runSessionActionMenu(item: SessionActionItem) {
  if (item.disabled) return
  const target = sessionActionMenuTarget.value
  if (!target) return
  if (!props.mobileVariant && item.id === 'rename') {
    closeDesktopSessionActionMenu()
    beginRenameForSession(target, 'inline')
    return
  }
  await runSessionActionMenuBase(item)
}

async function runSessionDialogAction(item: SessionDialogActionItem) {
  if (item.disabled) return
  if (item.id === 'delete') {
    const target = sessionActionsTarget.value
    if (target) await deleteSession(target.id)
    sessionActionsOpen.value = false
    return
  }
  if (item.id === 'rename') {
    const target = sessionActionsTarget.value
    if (!target) return
    sessionActionsOpen.value = false
    beginRenameForSession(target, 'dialog')
    return
  }
  const target = sessionActionsTarget.value
  if (!target) return
  sessionActionsOpen.value = false
  if (target.id && target.id !== chat.selectedSessionId) {
    await selectSession(target.id)
  }
  ui.requestSessionAction(item.id)
}
</script>

<template>
  <div class="oc-chat-sidebar flex h-full flex-col bg-sidebar overflow-hidden">
    <OptionMenu
      :open="sessionActionsOpen"
      v-model:query="sessionActionsDialogQuery"
      :groups="sessionActionMenuGroups"
      :title="String(t('chat.sidebar.sessionActions.menuTitle'))"
      :mobile-title="String(t('chat.sidebar.sessionActions.menuTitle'))"
      :searchable="true"
      :search-placeholder="String(t('common.searchActions'))"
      :empty-text="String(t('common.noActionsFound'))"
      :is-mobile-pointer="ui.isMobilePointer"
      filter-mode="external"
      @update:open="
        (v) => {
          sessionActionsOpen = v
          if (!v) sessionActionsTarget = null
        }
      "
      @select="runSessionDialogAction"
    />

    <RenameSessionDialog
      :open="renameDialogOpen"
      :draft="renameDraft"
      :busy="renameBusy"
      @update:open="
        (v) => {
          renameDialogOpen = v
          if (!v) cancelRenameFromSidebar()
        }
      "
      @update:draft="(v) => (renameDraft = v)"
      @save="saveRenameFromSidebar"
    />

    <div class="flex flex-col flex-1 min-h-0" :class="isSidebarDialogOpen ? 'pointer-events-none' : ''">
      <div class="border-b border-sidebar-border/60 px-2 py-2">
        <div class="flex items-center gap-2">
          <button
            type="button"
            class="flex min-w-0 flex-1 items-center justify-center gap-1.5 rounded-lg border border-primary/30 bg-primary/10 px-3 py-2 text-sm font-medium text-primary transition-colors hover:bg-primary/15 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            :disabled="creatingSession"
            @click="createNewSession"
          >
            <RiLoader4Line v-if="creatingSession" class="h-4 w-4 animate-spin" />
            <RiAddLine v-else class="h-4 w-4" />
            {{ t('hub.newSession') }}
          </button>

          <IconButton
            size="sm"
            :tooltip="String(t('chat.sidebar.header.refresh'))"
            :title="String(t('chat.sidebar.header.refresh'))"
            :aria-label="String(t('chat.sidebar.header.refresh'))"
            :is-touch-pointer="ui.isTouchPointer"
            :disabled="chat.sessionsLoading"
            @click="() => void chat.refreshSessions().catch(() => {})"
          >
            <RiRefreshLine class="h-4 w-4" />
          </IconButton>
        </div>
      </div>

      <div class="flex-1 min-h-0 overflow-x-hidden overflow-y-auto">
        <div v-if="chat.sessionsLoading && chat.sessions.length === 0" class="space-y-1 p-2">
          <div v-for="i in 6" :key="i" class="flex items-center gap-2 px-2 py-2">
            <div class="h-3 w-40 rounded bg-muted/30 animate-pulse" />
          </div>
        </div>

        <div
          v-else-if="!chat.sessionsLoading && chat.sessionsError && chat.sessions.length === 0"
          class="p-4 text-center text-xs text-destructive"
        >
          {{ chat.sessionsError }}
        </div>

        <div
          v-else-if="chat.sessions.length === 0"
          class="p-6 text-center text-xs text-muted-foreground"
        >
          {{ t('chat.sidebar.directoriesList.noSessionsYet') }}
        </div>

        <div v-else class="flex flex-col py-1">
          <SessionRow
            v-for="session in chat.sessions"
            :key="session.id"
            :session-id="String(session.id)"
            :session="session"
            :ui-is-compact-layout="ui.isCompactLayout"
            :selected="chat.selectedSessionId === String(session.id)"
            :session-action-menu-open="sessionActionMenuTarget?.id === String(session.id)"
            :session-action-menu-anchor-el="sessionActionMenuAnchorRef"
            :session-action-menu-query="sessionActionMenuQuery"
            :filtered-session-action-items="filteredSessionActionItems"
            :set-session-action-menu-ref="setSessionActionMenuRef"
            :run-session-action-menu="runSessionActionMenu"
            :renaming="isRenamingSession(String(session.id))"
            :rename-draft="renameDraft"
            :rename-busy="renameBusy"
            @open="() => void selectSession(String(session.id))"
            @open-actions="() => openSessionActions(session)"
            @open-action-menu="(event) => openSessionActionMenu(session, event)"
            @update:sessionActionMenuQuery="(v) => (sessionActionMenuQuery = v)"
            @update:renameDraft="updateRenameDraft"
            @rename-save="() => void saveRenameFromSidebar()"
            @rename-cancel="cancelRenameFromSidebar"
          />
        </div>
      </div>
    </div>
  </div>
</template>
