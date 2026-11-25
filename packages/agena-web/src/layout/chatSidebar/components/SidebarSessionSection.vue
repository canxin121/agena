<script setup lang="ts">
import type {
  SidebarSession as SessionLike,
  SessionMenuTarget,
  ThreadSessionRow,
} from '@/layout/chatSidebar/sessionMenu.types'
import { useI18n } from 'vue-i18n'

import type { DirectoryEntry } from '@/features/sessions/model/types'
import type { SessionActionItem } from '@/layout/chatSidebar/useSessionActionMenu'
import SidebarPagedSection from '@/layout/SidebarPagedSection.vue'
import SessionRow from '@/layout/chatSidebar/components/SessionRow.vue'

const { t } = useI18n()

const props = defineProps<{
  kind: 'favorites' | 'pinned' | 'recent' | 'running'
  open: boolean
  page: number
  paging?: boolean
  loading?: boolean
  pageCount: number
  total: number
  rows: ThreadSessionRow[]
  selectedSessionId: string | null

  uiIsCompactLayout: boolean
  pinnedSessionIds: string[]
  hasAttention: (sessionId: string) => 'permission' | 'question' | null
  statusLabelForSessionId: (sessionId: string) => { label: string; dotClass: string }

  openSessionActions: (directory: DirectoryEntry, session: SessionLike, event?: MouseEvent | PointerEvent) => void
  openSessionActionMenu: (directory: DirectoryEntry, session: SessionLike, event?: MouseEvent | PointerEvent) => void
  togglePin: (sessionId: string) => void
  toggleFavorite: (sessionId: string, favoriteHint?: boolean) => void
  deleteSession: (sessionId: string) => Promise<void>

  sessionActionMenuTarget: SessionMenuTarget | null
  sessionActionMenuAnchorEl: HTMLElement | null
  sessionActionMenuQuery: string
  filteredSessionActionItems: SessionActionItem[]
  closeSessionActionMenu: () => void
  runSessionActionMenu: (item: SessionActionItem) => Promise<void>

  isSessionRenaming: (sessionId: string) => boolean
  renameOrigin: HTMLElement | null
  requestRenameSession: (session: SessionLike, row: HTMLElement) => void
  renameDraft: string
  renameBusy: boolean
  updateRenameDraft: (value: string) => void
  saveRename: () => Promise<void> | void
  cancelRename: () => void
}>()

const emit = defineEmits<{
  (e: 'update:open', v: boolean): void
  (e: 'update:page', v: number): void
  (e: 'open-session', sessionId: string): void
  (e: 'toggle-thread', sessionId: string): void
  (e: 'child-page', sessionId: string, page: number): void
  (e: 'update:sessionActionMenuQuery', v: string): void
}>()

function isPinned(sessionId: string) {
  return props.pinnedSessionIds.includes(sessionId)
}

function statusMeta(sessionId: string) {
  return props.statusLabelForSessionId(sessionId)
}
</script>

<template>
  <SidebarPagedSection
    tree
    :title="t(`chat.sidebar.footers.${kind}.title`)"
    :toggle-label="t(`chat.sidebar.footers.${kind}.toggleAria`)"
    :prev-label="t(`chat.sidebar.footers.${kind}.prevPage`)"
    :next-label="t(`chat.sidebar.footers.${kind}.nextPage`)"
    :open="open"
    :count="Math.max(0, Math.floor(total || 0))"
    :page="page"
    :page-count="pageCount"
    :paging="paging"
    :loading="loading"
    :empty-message="total === 0 ? t(`chat.sidebar.footers.${kind}.empty`) : undefined"
    @update:open="emit('update:open', $event)"
    @update:page="emit('update:page', $event)"
  >
    <SessionRow
      v-for="item in rows"
      :key="item.renderKey"
      :session-id="item.id"
      :session="item.session"
      :directory="item.directory"
      :ui-is-compact-layout="uiIsCompactLayout"
      :selected="selectedSessionId === item.id"
      :indent-px="2 + item.depth * 10"
      :is-parent="item.isParent"
      :is-expanded="item.isExpanded"
      :child-page="item.childPage"
      :child-page-count="item.childPageCount"
      :show-thread-placeholder="true"
      :show-directory="true"
      :status-label="statusMeta(item.id).label"
      :status-dot-class="statusMeta(item.id).dotClass"
      :attention="hasAttention(item.id)"
      :pinned="isPinned(item.id)"
      :actions-enabled="Boolean(item.session && item.directory)"
      :session-action-menu-open="sessionActionMenuTarget?.session?.id === item.id"
      :session-action-menu-anchor-el="sessionActionMenuAnchorEl"
      :session-action-menu-query="sessionActionMenuQuery"
      :filtered-session-action-items="filteredSessionActionItems"
      :close-session-action-menu="closeSessionActionMenu"
      :run-session-action-menu="runSessionActionMenu"
      :renaming="isSessionRenaming(item.id)"
      :rename-origin="renameOrigin"
      :rename-draft="renameDraft"
      :rename-busy="renameBusy"
      menu-placement="top-end"
      @open="emit('open-session', item.id)"
      @toggle-thread="emit('toggle-thread', item.id)"
      @child-page="(page) => emit('child-page', item.id, page)"
      @open-actions="item.session && item.directory ? openSessionActions(item.directory, item.session) : undefined"
      @open-context-menu="
        (event) =>
          item.session && item.directory ? openSessionActions(item.directory, item.session, event) : undefined
      "
      @open-action-menu="
        (event) =>
          item.session && item.directory ? openSessionActionMenu(item.directory, item.session, event) : undefined
      "
      @toggle-pin="togglePin(item.id)"
      @toggle-favorite="(favorite) => toggleFavorite(item.id, favorite)"
      @delete="deleteSession(item.id)"
      @update:renameDraft="updateRenameDraft"
      @rename="(origin) => item.session && requestRenameSession(item.session, origin)"
      @rename-save="saveRename"
      @rename-cancel="cancelRename"
      @update:sessionActionMenuQuery="(v) => emit('update:sessionActionMenuQuery', v)"
    />
  </SidebarPagedSection>
</template>
