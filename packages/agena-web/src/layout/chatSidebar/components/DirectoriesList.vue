<script setup lang="ts">
import type { SidebarSession as SessionLike, SessionMenuTarget, MenuRefEl, SessionToggleSelectionOptions, DirectoryToggleSelectionOptions } from '@/layout/chatSidebar/sessionMenu.types'
import { vKeyboardCollection } from '@/lib/keyboardCollection'
import { RiDeleteBinLine } from '@remixicon/vue'
import { useI18n } from 'vue-i18n'

import ConfirmPopover from '@/components/ui/ConfirmPopover.vue'
import SelectionToolbar from '@/components/ui/SelectionToolbar.vue'
import SidebarIconButton from '@/components/ui/SidebarIconButton.vue'
import SidebarTextButton from '@/components/ui/SidebarTextButton.vue'
import Skeleton from '@/components/ui/Skeleton.vue'
import { directoryEntryLabel, sessionLabel } from '@/features/sessions/model/labels'
import type { DirectoryEntry } from '@/features/sessions/model/types'
import type { FlatTreeRow } from '@/features/sessions/model/tree'
import type { SessionActionItem } from '@/layout/chatSidebar/useSessionActionMenu'
import type { JsonValue } from '@/types/json'
import DirectoryRow from '@/layout/chatSidebar/components/DirectoryRow.vue'
import SidebarPager from '@/layout/chatSidebar/components/SidebarPager.vue'
import SessionRow from '@/layout/chatSidebar/components/SessionRow.vue'
import SidebarSectionSkeleton from '@/layout/chatSidebar/components/SidebarSectionSkeleton.vue'

type SessionSearchHit = {
  directory: DirectoryEntry
  session: {
    id?: string | number | null
    title?: string | null
    slug?: string | null
    [k: string]: JsonValue
  }
}

type ThreadSessionRow = {
  id: string
  session: SessionLike | null
  directory: DirectoryEntry | null
  renderKey: string
  depth: number
  isParent: boolean
  isExpanded: boolean
}

type DirectoryActivityState = 'running' | 'attention' | 'mixed' | null

const props = defineProps<{
  uiIsCompactLayout: boolean
  uiIsTouchPointer: boolean
  directories: DirectoryEntry[]
  pagedDirectories: DirectoryEntry[]
  visibleDirectories: DirectoryEntry[]
  sidebarQueryNorm: string
  searchWarming: boolean
  sessionSearchHits: SessionSearchHit[]
  locateFromSearch: (hit: SessionSearchHit) => Promise<void>

  locatedSessionId: string
  setSessionEl: (sessionId: string, el: MenuRefEl) => void

  pinnedSessionIds: string[]
  chatSelectedSessionId: string | null

  creatingSession: boolean
  directoryMultiSelectEnabled: boolean
  isDirectorySelected: (directoryId: string) => boolean
  isSessionMultiSelectEnabledForDirectory: (directoryId: string) => boolean
  toggleDirectorySessionMultiSelect: (directoryId: string) => void
  selectableSessionIds: string[]
  selectedSessionIds: ReadonlySet<string>
  selectAllSessionsForDirectory: (directoryId: string) => void
  invertSessionsForDirectory: (directoryId: string) => void
  deleteSelectedSessionsForDirectory: (directoryId: string) => Promise<void>
  isSessionSelected: (sessionId: string) => boolean
  toggleDirectorySelected: (directoryId: string, opts?: DirectoryToggleSelectionOptions) => void
  toggleSessionSelected: (sessionId: string, opts?: SessionToggleSelectionOptions) => void

  directoryPageLoading: boolean
  isDirectoryExpandLoading: (directoryId: string) => boolean

  isDirectoryCollapsed: (directoryId: string) => boolean
  toggleDirectoryCollapse: (directoryId: string, directoryPath: string) => void
  isDirectoryFocused: (directory: DirectoryEntry) => boolean
  directoryActivityState: (directory: DirectoryEntry) => DirectoryActivityState

  openDirectoryActions: (directory: DirectoryEntry, event?: MouseEvent | PointerEvent) => void
  refreshDirectoryInline: (directory: DirectoryEntry) => Promise<void>
  newSessionInline: (directory: DirectoryEntry) => Promise<void>
  removeDirectoryInline: (directory: DirectoryEntry) => Promise<void>

  sessionCountForDirectory: (directoryId: string, directoryPath: string) => number
  hasDirectorySidebarSection: (directoryId: string) => boolean
  selectDirectory: (directoryId: string, directoryPath: string) => Promise<void>
  selectSession: (sessionId: string) => Promise<void>

  openSessionActions: (directory: DirectoryEntry, session: SessionLike, event?: MouseEvent | PointerEvent) => void
  openSessionActionMenu: (directory: DirectoryEntry, session: SessionLike, event?: MouseEvent | PointerEvent) => void

  togglePin: (sessionId: string) => void
  toggleFavorite: (sessionId: string, favoriteHint?: boolean) => void
  deleteSession: (sessionId: string) => Promise<void>

  hasAttention: (sessionId: string) => 'permission' | 'question' | null
  statusLabelForSessionId: (sessionId: string) => { label: string; dotClass: string }

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

  setChildSessionPage: (directoryId: string, sessionId: string, page: number) => Promise<void>
  setDirectoryPinnedPage: (directoryId: string, page: number) => Promise<void>
  directoryPinnedPage: (directoryId: string) => number
  directoryPinnedPageCount: (directoryId: string) => number
  pinnedRowsForDirectory: (directoryId: string) => ThreadSessionRow[]
  pagedRowsForDirectory: (directoryId: string) => FlatTreeRow[]
  toggleExpandedParent: (sessionId: string) => void
  sessionRootPageCount: (directoryId: string) => number
  sessionRootPage: (directoryId: string) => number
  setSessionRootPage: (directoryId: string, page: number) => void
}>()

const emit = defineEmits<{
  (e: 'update:sessionActionMenuQuery', v: string): void
}>()

const { t } = useI18n()

function pinnedRows(directoryId: string) {
  const rows = props.pinnedRowsForDirectory(directoryId)
  return Array.isArray(rows) ? rows : []
}

function pagedRows(directoryId: string) {
  const rows = props.pagedRowsForDirectory(directoryId)
  return Array.isArray(rows) ? rows : []
}

function orderedDirectoryIds(): string[] {
  if (!Array.isArray(props.directories)) return []
  return props.directories.map((entry) => String(entry?.id || '').trim()).filter(Boolean)
}

function statusMeta(sessionId: string) {
  return props.statusLabelForSessionId(sessionId)
}
</script>

<template>
  <div
    v-keyboard-collection="{ tree: true }"
    :aria-label="String(t('header.openSessions'))"
    class="min-h-0 overflow-x-hidden overflow-visible"
  >
    <div class="space-y-0.5 pb-0.5 pl-2 pr-1">
      <div v-if="directories.length === 0" class="px-2 py-6 text-center text-muted-foreground">
        <div class="typography-ui-label font-semibold">{{ t('chat.sidebar.directoriesList.empty.title') }}</div>
        <div class="typography-meta mt-1">{{ t('chat.sidebar.directoriesList.empty.description') }}</div>
      </div>

      <div v-else>
        <div v-if="sidebarQueryNorm" class="px-1.5 pb-2">
          <div class="flex items-center justify-between gap-2">
            <div class="text-[11px] font-semibold tracking-wider text-muted-foreground uppercase">
              {{ t('chat.sidebar.directoriesList.search.sessionsTitle') }}
            </div>
            <div v-if="searchWarming" class="text-[10px] font-mono text-muted-foreground/60">
              {{ t('common.loading') }}
            </div>
          </div>

          <div v-if="sessionSearchHits.length" class="mt-1 space-y-0.5 animate-in fade-in-0 duration-200">
            <SidebarTextButton
              v-for="hit in sessionSearchHits"
              :key="String(hit.session.id)"
              :data-keyboard-item="String(hit.session.id)"
              :data-keyboard-label="sessionLabel(hit.session)"
              class="w-full rounded-md px-2 py-1 hover:dark:bg-accent/40 hover:bg-primary/6"
              @click="props.locateFromSearch(hit)"
            >
              <div class="typography-ui-label font-medium truncate">{{ sessionLabel(hit.session) }}</div>
              <div class="text-[10px] text-muted-foreground/70 truncate">{{ directoryEntryLabel(hit.directory) }}</div>
            </SidebarTextButton>
          </div>

          <div v-else-if="searchWarming" class="mt-2 rounded-md border border-sidebar-border/40 bg-muted/10 px-2 py-2">
            <div class="space-y-1.5">
              <Skeleton class="h-3 w-4/5" />
              <Skeleton class="h-3 w-3/5" />
              <Skeleton class="h-3 w-2/3" />
            </div>
          </div>

          <div v-else class="mt-2 text-xs text-muted-foreground animate-in fade-in-0 duration-200">
            <div class="font-medium">{{ t('chat.sidebar.directoriesList.search.noMatchingSessions') }}</div>
          </div>
        </div>

        <div v-if="visibleDirectories.length === 0" class="px-2 py-6 text-center text-muted-foreground">
          <div class="typography-ui-label font-semibold">
            {{ t('chat.sidebar.directoriesList.noMatchingDirectories.title') }}
          </div>
          <div class="typography-meta mt-1">
            {{ t('chat.sidebar.directoriesList.noMatchingDirectories.description') }}
          </div>
        </div>

        <div v-else class="space-y-0.5">
          <div v-for="directory in pagedDirectories" :key="directory.id" data-selection-context class="relative">
            <div
              class="sticky top-0 z-20 w-full border-b bg-sidebar pt-1.5 pb-1 select-none group"
              :class="
                props.isDirectoryCollapsed(directory.id)
                  ? 'bg-sidebar border-sidebar-border/30'
                  : 'bg-sidebar border-sidebar-border/60'
              "
            >
              <DirectoryRow
                :directory="directory"
                :ui-is-compact-layout="uiIsCompactLayout"
                :collapsed="props.isDirectoryCollapsed(directory.id)"
                :focused="props.isDirectoryFocused(directory)"
                :activity-state="props.directoryActivityState(directory)"
                :loading="props.directoryPageLoading"
                :creating-session="props.creatingSession"
                :multi-select-enabled="props.directoryMultiSelectEnabled"
                :multi-selected="props.isDirectorySelected(directory.id)"
                :session-multi-select-enabled="props.isSessionMultiSelectEnabledForDirectory(directory.id)"
                @row-click="
                  (event) =>
                    props.directoryMultiSelectEnabled
                      ? props.toggleDirectorySelected(directory.id, {
                          event,
                          orderedDirectoryIds: orderedDirectoryIds(),
                        })
                      : props.toggleDirectoryCollapse(directory.id, directory.path)
                "
                @toggle-collapse="props.toggleDirectoryCollapse(directory.id, directory.path)"
                @toggle-session-multi-select="props.toggleDirectorySessionMultiSelect(directory.id)"
                @open-actions="props.openDirectoryActions(directory)"
                @open-context-menu="(event) => props.openDirectoryActions(directory, event)"
                @refresh="props.refreshDirectoryInline(directory)"
                @new-session="props.newSessionInline(directory)"
                @remove="props.removeDirectoryInline(directory)"
              />
            </div>

            <div v-if="!props.isDirectoryCollapsed(directory.id)" class="py-0.5 pl-1">
              <div class="space-y-0.5">
                <SelectionToolbar
                  v-if="props.isSessionMultiSelectEnabledForDirectory(directory.id)"
                  class="mx-1.5 mt-0.5 mb-1 rounded-md border border-sidebar-border/50 bg-muted/20"
                  :selectable-ids="props.selectableSessionIds"
                  :selected-ids="props.selectedSessionIds"
                  :selection-label="
                    t('chat.sidebar.multiSelect.sessionCount', {
                      count: props.selectedSessionIds.size,
                    })
                  "
                  :exit-label="t('chat.sidebar.multiSelect.actions.exitSessionMultiSelect')"
                  :is-touch-pointer="props.uiIsTouchPointer"
                  @select-all="props.selectAllSessionsForDirectory(directory.id)"
                  @invert="props.invertSessionsForDirectory(directory.id)"
                  @exit="props.toggleDirectorySessionMultiSelect(directory.id)"
                >
                  <template #default="{ size }">
                    <ConfirmPopover
                      :title="t('chat.sidebar.multiSelect.confirmDelete.title')"
                      :description="
                        t('chat.sidebar.multiSelect.confirmDelete.sessionsDescription', {
                          sessions: props.selectedSessionIds.size,
                        })
                      "
                      :confirm-text="t('chat.sidebar.multiSelect.actions.deleteSelected')"
                      variant="destructive"
                      @confirm="props.deleteSelectedSessionsForDirectory(directory.id)"
                    >
                      <SidebarIconButton
                        data-keyboard-option
                        :size="size"
                        destructive
                        :tooltip="t('chat.sidebar.multiSelect.actions.deleteSelected')"
                        :is-touch-pointer="props.uiIsTouchPointer"
                        :disabled="!props.selectedSessionIds.size"
                        @click.stop
                      >
                        <RiDeleteBinLine class="size-3.5" aria-hidden="true" />
                      </SidebarIconButton>
                    </ConfirmPopover>
                  </template>
                </SelectionToolbar>

                <div
                  v-if="
                    props.isDirectoryExpandLoading(directory.id) &&
                    pinnedRows(directory.id).length === 0 &&
                    pagedRows(directory.id).length === 0
                  "
                  class="px-1.5 py-1.5"
                >
                  <SidebarSectionSkeleton :rows="3" :with-header="false" compact />
                </div>

                <div
                  v-else-if="
                    props.directoryPageLoading && props.sessionCountForDirectory(directory.id, directory.path) === 0
                  "
                  class="px-1.5 py-1.5"
                >
                  <SidebarSectionSkeleton :rows="3" :with-header="false" compact />
                </div>

                <div
                  v-else-if="
                    props.directoryPageLoading &&
                    props.sessionCountForDirectory(directory.id, directory.path) > 0 &&
                    pinnedRows(directory.id).length === 0 &&
                    pagedRows(directory.id).length === 0
                  "
                  class="px-1.5 py-1.5"
                >
                  <SidebarSectionSkeleton :rows="4" compact />
                </div>

                <div v-else-if="!props.hasDirectorySidebarSection(directory.id)" class="px-1.5 py-1.5">
                  <SidebarSectionSkeleton :rows="3" :with-header="false" compact />
                </div>

                <div
                  v-else-if="props.sessionCountForDirectory(directory.id, directory.path) === 0"
                  class="px-1.5 py-0.5 text-xs text-muted-foreground"
                >
                  {{ t('chat.sidebar.directoriesList.noSessionsYet') }}
                </div>

                <template v-else>
                  <div v-if="pinnedRows(directory.id).length" class="space-y-0.5 animate-in fade-in-0 duration-200">
                    <div class="px-1.5 text-[11px] font-semibold tracking-wider text-muted-foreground uppercase">
                      {{ t('chat.sidebar.directoriesList.pinnedTitle') }}
                    </div>
                    <SidebarPager
                      v-if="props.directoryPinnedPageCount(directory.id) > 1"
                      :page="props.directoryPinnedPage(directory.id)"
                      :page-count="props.directoryPinnedPageCount(directory.id)"
                      :disabled="props.directoryPageLoading"
                      :prev-label="String(t('chat.sidebar.directoriesList.prevSessionsPage'))"
                      :next-label="String(t('chat.sidebar.directoriesList.nextSessionsPage'))"
                      @update:page="(page) => props.setDirectoryPinnedPage(directory.id, page)"
                    />

                    <div
                      v-for="row in pinnedRows(directory.id)"
                      :key="row.renderKey"
                      :ref="(el) => props.setSessionEl(row.id, el)"
                    >
                      <SessionRow
                        :session-id="row.id"
                        :session="row.session"
                        :directory="row.directory || directory"
                        :ui-is-compact-layout="uiIsCompactLayout"
                        :selected="
                          props.isSessionMultiSelectEnabledForDirectory(directory.id)
                            ? props.isSessionSelected(row.id)
                            : chatSelectedSessionId === row.id
                        "
                        :highlighted="locatedSessionId === row.id"
                        :indent-px="2 + row.depth * 10"
                        :is-parent="row.isParent"
                        :is-expanded="row.isExpanded"
                        :show-thread-placeholder="true"
                        :multi-select-enabled="props.isSessionMultiSelectEnabledForDirectory(directory.id)"
                        :multi-selected="props.isSessionSelected(row.id)"
                        :status-label="statusMeta(row.id).label"
                        :status-dot-class="statusMeta(row.id).dotClass"
                        :attention="props.hasAttention(row.id)"
                        :pinned="props.pinnedSessionIds.includes(row.id)"
                        :actions-enabled="
                          !props.isSessionMultiSelectEnabledForDirectory(directory.id) &&
                          Boolean(row.session && row.directory)
                        "
                        :session-action-menu-open="sessionActionMenuTarget?.session?.id === row.id"
                        :session-action-menu-anchor-el="sessionActionMenuAnchorEl"
                        :session-action-menu-query="sessionActionMenuQuery"
                        :filtered-session-action-items="filteredSessionActionItems"
                        :close-session-action-menu="props.closeSessionActionMenu"
                        :run-session-action-menu="props.runSessionActionMenu"
                        :renaming="props.isSessionRenaming(row.id)"
                        :rename-origin="props.renameOrigin"
                        :rename-draft="props.renameDraft"
                        :rename-busy="props.renameBusy"
                        @open="
                          async () => {
                            await props.selectDirectory(directory.id, directory.path)
                            await props.selectSession(row.id)
                          }
                        "
                        @toggle-select="
                          (event) =>
                            props.toggleSessionSelected(row.id, {
                              event,
                              orderedSessionIds: pinnedRows(directory.id).map((item) => item.id),
                            })
                        "
                        @toggle-thread="props.toggleExpandedParent(row.id)"
                        @open-actions="
                          row.session && row.directory
                            ? props.openSessionActions(row.directory, row.session)
                            : undefined
                        "
                        @open-context-menu="
                          (event) =>
                            row.session && row.directory
                              ? props.openSessionActions(row.directory, row.session, event)
                              : undefined
                        "
                        @open-action-menu="
                          (event) =>
                            row.session && row.directory
                              ? props.openSessionActionMenu(row.directory, row.session, event)
                              : undefined
                        "
                        @toggle-pin="props.togglePin(row.id)"
                        @toggle-favorite="(favorite) => props.toggleFavorite(row.id, favorite)"
                        @delete="props.deleteSession(row.id)"
                        @update:renameDraft="props.updateRenameDraft"
                        @rename="(origin) => row.session && props.requestRenameSession(row.session, origin)"
                        @rename-save="props.saveRename"
                        @rename-cancel="props.cancelRename"
                        @update:sessionActionMenuQuery="(v) => emit('update:sessionActionMenuQuery', v)"
                      />
                    </div>
                  </div>

                  <div class="space-y-0.5 animate-in fade-in-0 duration-200">
                    <div class="px-1.5 flex items-center justify-between gap-2">
                      <div class="text-[11px] font-semibold tracking-wider text-muted-foreground uppercase">
                        {{ t('chat.sidebar.directoriesList.recentTitle') }}
                      </div>
                      <SidebarPager
                        v-if="props.sessionRootPageCount(directory.id) > 1"
                        :page="props.sessionRootPage(directory.id)"
                        :page-count="props.sessionRootPageCount(directory.id)"
                        :disabled="props.directoryPageLoading"
                        :prev-label="String(t('chat.sidebar.directoriesList.prevSessionsPage'))"
                        :next-label="String(t('chat.sidebar.directoriesList.nextSessionsPage'))"
                        @update:page="(next) => props.setSessionRootPage(directory.id, next)"
                      />
                    </div>

                    <div
                      v-for="row in pagedRows(directory.id)"
                      :key="row.id"
                      :ref="(el) => props.setSessionEl(row.id, el)"
                    >
                      <SessionRow
                        :session-id="row.id"
                        :session="row.session"
                        :directory="directory"
                        :ui-is-compact-layout="uiIsCompactLayout"
                        :selected="
                          props.isSessionMultiSelectEnabledForDirectory(directory.id)
                            ? props.isSessionSelected(row.id)
                            : chatSelectedSessionId === row.id
                        "
                        :highlighted="locatedSessionId === row.id"
                        :indent-px="2 + row.depth * 10"
                        :is-parent="row.isParent"
                        :is-expanded="row.isExpanded"
                        :show-thread-placeholder="true"
                        :multi-select-enabled="props.isSessionMultiSelectEnabledForDirectory(directory.id)"
                        :multi-selected="props.isSessionSelected(row.id)"
                        :status-label="statusMeta(row.id).label"
                        :status-dot-class="statusMeta(row.id).dotClass"
                        :attention="props.hasAttention(row.id)"
                        :pinned="props.pinnedSessionIds.includes(row.id)"
                        :actions-enabled="
                          !props.isSessionMultiSelectEnabledForDirectory(directory.id) && Boolean(row.session)
                        "
                        :session-action-menu-open="sessionActionMenuTarget?.session?.id === (row.session?.id || row.id)"
                        :session-action-menu-anchor-el="sessionActionMenuAnchorEl"
                        :session-action-menu-query="sessionActionMenuQuery"
                        :filtered-session-action-items="filteredSessionActionItems"
                        :close-session-action-menu="props.closeSessionActionMenu"
                        :run-session-action-menu="props.runSessionActionMenu"
                        :renaming="props.isSessionRenaming(row.id)"
                        :rename-origin="props.renameOrigin"
                        :rename-draft="props.renameDraft"
                        :rename-busy="props.renameBusy"
                        @open="
                          async () => {
                            await props.selectDirectory(directory.id, directory.path)
                            await props.selectSession(row.id)
                          }
                        "
                        @toggle-select="
                          (event) =>
                            props.toggleSessionSelected(row.id, {
                              event,
                              orderedSessionIds: pagedRows(directory.id).map((item) => item.id),
                            })
                        "
                        @toggle-thread="props.toggleExpandedParent(row.id)"
                        @open-actions="row.session ? props.openSessionActions(directory, row.session) : undefined"
                        @open-context-menu="
                          (event) => (row.session ? props.openSessionActions(directory, row.session, event) : undefined)
                        "
                        @open-action-menu="
                          (event) =>
                            row.session ? props.openSessionActionMenu(directory, row.session, event) : undefined
                        "
                        @toggle-pin="props.togglePin(row.id)"
                        @toggle-favorite="(favorite) => props.toggleFavorite(row.id, favorite)"
                        @delete="props.deleteSession(row.id)"
                        @update:renameDraft="props.updateRenameDraft"
                        @rename="(origin) => row.session && props.requestRenameSession(row.session, origin)"
                        @rename-save="props.saveRename"
                        @rename-cancel="props.cancelRename"
                        @update:sessionActionMenuQuery="(v) => emit('update:sessionActionMenuQuery', v)"
                      />
                      <SidebarPager
                        v-if="row.isExpanded && (row.childPageCount || 1) > 1"
                        :page="row.childPage || 0"
                        :page-count="row.childPageCount || 1"
                        :disabled="props.directoryPageLoading"
                        :prev-label="String(t('chat.sidebar.directoriesList.prevSessionsPage'))"
                        :next-label="String(t('chat.sidebar.directoriesList.nextSessionsPage'))"
                        @update:page="(page) => props.setChildSessionPage(directory.id, row.id, page)"
                      />
                    </div>
                  </div>
                </template>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
