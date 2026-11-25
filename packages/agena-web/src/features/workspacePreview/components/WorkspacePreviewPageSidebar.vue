<script setup lang="ts">
import { computed, reactive, ref, useId, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import {
  RiAddLine,
  RiDeleteBinLine,
  RiEditLine,
  RiListCheck3,
  RiPlayLine,
  RiPencilLine,
  RiRefreshLine,
  RiStopLine,
} from '@remixicon/vue'

import Button from '@/components/ui/Button.vue'
import FormDialog from '@/components/ui/FormDialog.vue'
import IconButton from '@/components/ui/IconButton.vue'
import Input from '@/components/ui/Input.vue'
import ConfirmPopover from '@/components/ui/ConfirmPopover.vue'
import OptionMenu from '@/components/ui/OptionMenu.vue'
import type { OptionMenuGroup, OptionMenuItem } from '@/components/ui/optionMenu.types'
import SearchInput from '@/components/ui/SearchInput.vue'
import SelectionToolbar from '@/components/ui/SelectionToolbar.vue'
import SidebarIconButton from '@/components/ui/SidebarIconButton.vue'
import { useUnifiedMultiSelect } from '@/composables/useUnifiedMultiSelect'
import type { WorkspacePreviewSession } from '@/features/workspacePreview/api/workspacePreviewApi'
import PreviewCreateSessionDialog from './PreviewCreateSessionDialog.vue'
import PreviewSessionRow from './PreviewSessionRow.vue'
import SidebarPagedSection from '@/layout/SidebarPagedSection.vue'
import { usePreviewHealth, type PreviewHealthState } from '../model/usePreviewHealth'
import { normalizeDirForCompare } from '@/features/sessions/model/labels'
import { writeWorkspaceWindowTemplateToDataTransfer } from '@/layout/workspaceWindowDrag'
import { usePaneVisibility } from '@/composables/usePaneVisibility'
import { useVisibleSubscription } from '@/composables/useVisibleSubscription'
import { useChatStore } from '@/stores/chat'
import { useDirectoryStore } from '@/stores/directory'
import { useToastsStore } from '@/stores/toasts'
import { useUiStore } from '@/stores/ui'
import { useWorkspacePreviewStore } from '@/stores/workspacePreview'

const { t } = useI18n()
const ui = useUiStore()
const toasts = useToastsStore()
const chat = useChatStore()
const preview = useWorkspacePreviewStore()
const directoryStore = useDirectoryStore()
const visible = usePaneVisibility()
useVisibleSubscription(() => preview.retainLiveSessions(), visible)

const currentDirectory = computed(() => String(directoryStore.currentDirectory || '').trim())
const currentChatSessionId = computed(() => String(chat.selectedSessionId || '').trim())
const currentDirectoryNorm = computed(() => normalizeDirForCompare(currentDirectory.value))
const sidebarQueryNorm = computed(() =>
  String(preview.sidebarQuery || '')
    .trim()
    .toLowerCase(),
)

const createDialog = ref<InstanceType<typeof PreviewCreateSessionDialog> | null>(null)

function onSessionCreated() {
  clearSessionHealth()
  probeVisibleSessions()
}

const editFormId = useId()
const editDialogOpen = ref(false)
const editSessionId = ref('')
const editTargetUrl = ref('')
const editBusy = ref(false)
const editError = ref('')

const rowMenuOpen = ref(false)
const rowMenuSessionId = ref('')
const rowMenuAnchorEl = ref<HTMLElement | null>(null)

type PreviewSidebarScope = 'chat' | 'directory' | 'all'
const rowMenuScope = ref<PreviewSidebarScope>('directory')

const renamingSessionId = ref('')
const renamingScope = ref<PreviewSidebarScope>('directory')
const renameFormId = useId()
const renameDialogOpen = ref(false)
const renameDraft = ref('')
const renameBusy = ref(false)
const renameError = ref('')

const renameDraftNorm = computed(() => String(renameDraft.value || '').trim())
const renameDraftValid = computed(() => /^[A-Za-z0-9_-]+$/.test(renameDraftNorm.value))
const canSaveRename = computed(() => !renameBusy.value && renameDraftNorm.value.length > 0 && renameDraftValid.value)

const rowMenuSession = computed(
  () => previewSessions.value.find((session) => session.id === rowMenuSessionId.value) || null,
)

const previewSessions = computed<WorkspacePreviewSession[]>(() =>
  Array.isArray(preview.sessions) ? preview.sessions : [],
)
const previewFilteredSessions = computed<WorkspacePreviewSession[]>(() =>
  Array.isArray(preview.filteredSessions) ? preview.filteredSessions : [],
)

const previewMultiSelect = useUnifiedMultiSelect()
const allPreviewSessionIds = computed(() => {
  const ids = new Set<string>()
  for (const session of previewSessions.value) {
    const id = String(session.id || '').trim()
    if (id) ids.add(id)
  }
  return Array.from(ids)
})

const filteredPreviewSessionIds = computed(() => {
  const ids = new Set<string>()
  for (const session of previewFilteredSessions.value) {
    const id = String(session.id || '').trim()
    if (id) ids.add(id)
  }
  return Array.from(ids)
})

function normalizeSessionState(input: unknown): string {
  return String(input || '')
    .trim()
    .toLowerCase()
}

function sessionStateLooksError(state: unknown): boolean {
  const s = normalizeSessionState(state)
  return s === 'error' || s === 'failed' || s === 'crashed'
}

function sessionHealthState(sessionId: string): PreviewHealthState {
  const entry = healthBySessionId.value[sessionId]
  return entry?.state || 'unknown'
}

function sessionHealthLabel(state: PreviewHealthState): string {
  if (state === 'ok') return String(t('header.status.online'))
  if (state === 'error') return String(t('header.status.offline'))
  if (state === 'checking') return String(t('workspaceDock.loading'))
  return String(t('common.unknown'))
}

function sessionDotClass(session: WorkspacePreviewSession): string {
  const state = normalizeSessionState(session.state)
  if (state === 'stopped' || state === 'idle') return 'bg-muted-foreground/50'
  if (state === 'starting' || state === 'building') return 'bg-muted-foreground/60 animate-pulse'

  const health = sessionHealthState(session.id)
  if (health === 'ok') return 'bg-primary'
  if (health === 'error') return 'bg-destructive'
  if (sessionStateLooksError(session.state)) return 'bg-destructive'
  if (health === 'checking') return 'bg-muted-foreground/60 animate-pulse'
  return 'bg-muted-foreground/40'
}

function sessionDotLabel(session: WorkspacePreviewSession): string {
  const health = sessionHealthState(session.id)
  if (health === 'unknown' && sessionStateLooksError(session.state)) return String(t('header.status.offline'))
  return sessionHealthLabel(health)
}

const SECTION_PAGE_SIZE = 4

const sectionScopes = ['chat', 'directory', 'all'] as const
const sectionOpen = reactive({ chat: Boolean(currentChatSessionId.value), directory: true, all: false })
const sectionPage = reactive({ chat: 0, directory: 0, all: 0 })

const sections = computed(() =>
  sectionScopes.map((scope) => {
    const context = scope === 'chat' ? currentChatSessionId.value : currentDirectoryNorm.value
    const matchesScope = (session: WorkspacePreviewSession) => {
      if (scope === 'all') return true
      if (!context) return false
      return scope === 'chat'
        ? String(session.agenaSessionId || '').trim() === context
        : normalizeDirForCompare(session.directory) === context
    }
    const rows = previewFilteredSessions.value.filter(matchesScope)
    const count = rows.length
    const pageCount = Math.max(1, Math.ceil(count / SECTION_PAGE_SIZE))
    const page = Math.max(0, Math.min(pageCount - 1, sectionPage[scope]))
    let emptyMessage = ''
    if (scope === 'chat' && !context) emptyMessage = t('workspaceDock.preview.sidebar.sections.chat.emptyNoSession')
    else if (scope === 'directory' && !context) emptyMessage = t('workspaceDock.preview.sidebar.sections.directory.emptyNoDirectory')
    else if (!count) {
      emptyMessage = sidebarQueryNorm.value && previewSessions.value.some(matchesScope)
        ? t('workspaceDock.preview.sidebar.noMatchingSessions')
        : t(`workspaceDock.preview.sidebar.sections.${scope}.empty`)
    }
    return {
      scope,
      open: sectionOpen[scope],
      title: t(`workspaceDock.preview.sidebar.sections.${scope}.title`),
      count,
      pageCount,
      page,
      emptyMessage,
      rows: rows.slice(page * SECTION_PAGE_SIZE, (page + 1) * SECTION_PAGE_SIZE),
    }
  }),
)

const visibleSessionsForHealth = computed(() => {
  const sessions = new Map<string, WorkspacePreviewSession>()
  for (const section of sections.value) {
    if (!section.open) continue
    for (const session of section.rows) {
      if (session.id) sessions.set(session.id, session)
    }
  }
  return Array.from(sessions.values())
})

const {
  entries: healthBySessionId,
  clear: clearSessionHealth,
  refresh: probeVisibleSessions,
} = usePreviewHealth({
  sessions: visibleSessionsForHealth,
  visible,
  loading: computed(() => preview.loading),
})

function canEditTargetUrl(session: WorkspacePreviewSession | null | undefined): boolean {
  return Boolean(String(session?.targetUrl || '').trim())
}

function isSessionRunning(session: WorkspacePreviewSession | null | undefined): boolean {
  if (!session) return false
  const s = normalizeSessionState(session.state)
  if (s === 'running') return true
  return Boolean((session as { pid?: unknown }).pid)
}

const rowMenuGroups = computed<OptionMenuGroup[]>(() => {
  const session = rowMenuSession.value
  if (!session) return []

  const canEditUrl = canEditTargetUrl(session)
  const running = isSessionRunning(session)
  return [
    {
      id: 'preview-session-actions',
      items: [
        ...(running
          ? [
              {
                id: 'stop',
                label: String(t('workspaceDock.preview.sidebar.actions.stop')),
                description: String(t('workspaceDock.preview.sidebar.actions.stopDescription')),
                icon: RiStopLine,
              },
            ]
          : [
              {
                id: 'start',
                label: String(t('workspaceDock.preview.sidebar.actions.start')),
                description: String(t('workspaceDock.preview.sidebar.actions.startDescription')),
                icon: RiPlayLine,
              },
            ]),
        {
          id: 'rename',
          label: String(t('workspaceDock.preview.sidebar.actions.rename')),
          description: String(t('workspaceDock.preview.sidebar.actions.renameDescription')),
          icon: RiEditLine,
        },
        {
          id: 'edit-url',
          label: String(t('workspaceDock.preview.sidebar.actions.editUrl')),
          description: canEditUrl
            ? String(t('workspaceDock.preview.sidebar.actions.editUrlDescription'))
            : String(t('workspaceDock.preview.sidebar.actions.editUrlUnavailable')),
          icon: RiPencilLine,
          disabled: !canEditUrl,
        },
        {
          id: 'delete',
          label: String(t('common.delete')),
          description: String(t('workspaceDock.preview.sidebar.actions.deleteDescription')),
          icon: RiDeleteBinLine,
          variant: 'destructive',
          confirmTitle: String(t('workspaceDock.preview.sidebar.actions.deleteConfirmTitle')),
          confirmDescription: String(t('workspaceDock.preview.sidebar.actions.deleteConfirmDescription')),
          confirmText: String(t('common.delete')),
          cancelText: String(t('common.cancel')),
        },
      ],
    },
  ]
})

async function refreshSessions(opts?: { forceFrameReload?: boolean }) {
  clearSessionHealth()
  await preview.refreshSessions({ force: true })
  if (opts?.forceFrameReload) preview.bumpRefreshToken()
  probeVisibleSessions()
}

function isRenamingSession(session: WorkspacePreviewSession, scope: PreviewSidebarScope): boolean {
  return renamingSessionId.value === session.id && renamingScope.value === scope
}

function isInlineRenameSession(session: WorkspacePreviewSession, scope: PreviewSidebarScope): boolean {
  return !ui.isCompactLayout && isRenamingSession(session, scope)
}

function startRenameSession(sessionId: string, scope: PreviewSidebarScope) {
  if (renameBusy.value) return
  const session = previewSessions.value.find((item) => item.id === sessionId)
  if (!session) return

  renamingSessionId.value = session.id
  renamingScope.value = scope
  renameDraft.value = session.id
  renameBusy.value = false
  renameError.value = ''

  sectionOpen[scope] = true

  if (ui.isCompactLayout) {
    renameDialogOpen.value = true
  }
}

function cancelRenameSession() {
  renamingSessionId.value = ''
  renameDraft.value = ''
  renameBusy.value = false
  renameError.value = ''
  renameDialogOpen.value = false
}

async function saveRenameSession() {
  if (renameBusy.value) return
  const sessionId = String(renamingSessionId.value || '').trim()
  if (!sessionId) return

  if (!renameDraftNorm.value) {
    renameError.value = String(t('workspaceDock.preview.emptyState.sessionIdRequired'))
    return
  }
  if (!renameDraftValid.value) {
    renameError.value = String(t('workspaceDock.preview.emptyState.sessionIdInvalid'))
    return
  }

  renameBusy.value = true
  renameError.value = ''
  try {
    const updated = await preview.renameSession(sessionId, renameDraftNorm.value)
    cancelRenameSession()
    clearSessionHealth(sessionId)
    clearSessionHealth(updated.id)
    probeVisibleSessions()
  } catch (err) {
    renameError.value = err instanceof Error ? err.message : String(err)
  } finally {
    renameBusy.value = false
  }
}

function openRowMenu(sessionId: string, scope: PreviewSidebarScope, event: MouseEvent) {
  rowMenuSessionId.value = sessionId
  rowMenuScope.value = scope
  rowMenuAnchorEl.value = event.currentTarget as HTMLElement | null
  rowMenuOpen.value = true
}

function setRowMenuOpen(next: boolean) {
  rowMenuOpen.value = Boolean(next)
  if (!rowMenuOpen.value) {
    rowMenuSessionId.value = ''
    rowMenuAnchorEl.value = null
    rowMenuScope.value = 'directory'
  }
}

function openEditDialog(sessionId: string) {
  const session = previewSessions.value.find((item) => item.id === sessionId)
  if (!session) return
  if (!canEditTargetUrl(session)) return

  editError.value = ''
  editSessionId.value = session.id
  editTargetUrl.value = String(session.targetUrl || '').trim()
  editDialogOpen.value = true
}

async function saveEditDialog() {
  if (editBusy.value) return
  const sessionId = String(editSessionId.value || '').trim()
  const targetUrl = String(editTargetUrl.value || '').trim()
  if (!sessionId || !targetUrl) return

  editBusy.value = true
  editError.value = ''
  try {
    await preview.updateSession(sessionId, { targetUrl })
    editDialogOpen.value = false
    clearSessionHealth(sessionId)
    probeVisibleSessions()
  } catch (err) {
    editError.value = err instanceof Error ? err.message : String(err)
  } finally {
    editBusy.value = false
  }
}

async function onRowMenuSelect(item: OptionMenuItem) {
  const session = rowMenuSession.value
  if (!session) return

  if (item.id === 'start') {
    setRowMenuOpen(false)
    try {
      await preview.startSession(session.id)
      clearSessionHealth(session.id)
      probeVisibleSessions()
    } catch (err) {
      const detail = err instanceof Error ? err.message : String(err)
      toasts.push('error', String(t('workspaceDock.preview.sidebar.actions.startFailed', { detail })), 4500)
    }
    return
  }

  if (item.id === 'stop') {
    setRowMenuOpen(false)
    try {
      await preview.stopSession(session.id)
      clearSessionHealth(session.id)
      probeVisibleSessions()
    } catch (err) {
      const detail = err instanceof Error ? err.message : String(err)
      toasts.push('error', String(t('workspaceDock.preview.sidebar.actions.stopFailed', { detail })), 4500)
    }
    return
  }

  if (item.id === 'rename') {
    const scope = rowMenuScope.value
    setRowMenuOpen(false)
    startRenameSession(session.id, scope)
    return
  }

  if (item.id === 'edit-url') {
    setRowMenuOpen(false)
    openEditDialog(session.id)
    return
  }

  if (item.id === 'delete') {
    setRowMenuOpen(false)
    await deletePreviewSessionById(session.id)
  }
}

function togglePreviewMultiSelectMode() {
  const next = !previewMultiSelect.enabled.value
  if (next) {
    cancelRenameSession()
    setRowMenuOpen(false)
  }
  previewMultiSelect.setEnabled(next)
}

function handlePreviewSessionRowClick(
  session: WorkspacePreviewSession,
  scope: PreviewSidebarScope,
  event?: MouseEvent,
) {
  if (isInlineRenameSession(session, scope)) return
  if (previewMultiSelect.enabled.value) {
    previewMultiSelect.selectByInteraction(session.id, filteredPreviewSessionIds.value, event)
    return
  }
  selectSession(session.id)
}

function canDragPreviewSession(session: WorkspacePreviewSession, scope: PreviewSidebarScope): boolean {
  if (previewMultiSelect.enabled.value) return false
  if (isInlineRenameSession(session, scope)) return false
  return Boolean(String(session.id || '').trim())
}

function previewSessionSubtitle(session: WorkspacePreviewSession): string {
  const directory = String(session.directory || '').trim()
  if (directory) return directory
  const runDirectory = String(session.runDirectory || '').trim()
  if (runDirectory) return runDirectory
  const targetUrl = String(session.targetUrl || '').trim()
  if (targetUrl) return targetUrl
  const command = String(session.command || '').trim()
  if (command) return command
  return String(t('nav.preview'))
}

function handlePreviewSessionRowDragStart(
  event: DragEvent,
  session: WorkspacePreviewSession,
  scope: PreviewSidebarScope,
) {
  if (!canDragPreviewSession(session, scope)) {
    event.preventDefault()
    return
  }

  const transfer = event.dataTransfer
  if (!transfer) {
    event.preventDefault()
    return
  }

  transfer.effectAllowed = 'copyMove'
  const ok = writeWorkspaceWindowTemplateToDataTransfer(transfer, {
    tab: 'preview',
    title: session.id,
  })
  if (!ok) {
    event.preventDefault()
  }
}

function selectAllPreviewSessions() {
  if (!previewMultiSelect.enabled.value) return
  previewMultiSelect.selectAll(filteredPreviewSessionIds.value)
}

function invertPreviewSessionsSelection() {
  if (!previewMultiSelect.enabled.value) return
  previewMultiSelect.invertSelection(filteredPreviewSessionIds.value)
}

async function deletePreviewSessionById(sessionId: string): Promise<boolean> {
  const id = String(sessionId || '').trim()
  if (!id) return false
  try {
    await preview.deleteSession(id)
    clearSessionHealth(id)
    probeVisibleSessions()
    return true
  } catch (err) {
    const detail = err instanceof Error ? err.message : String(err)
    toasts.push('error', String(t('workspaceDock.preview.sidebar.actions.deleteFailed', { detail })), 4500)
    return false
  }
}

async function deleteSelectedPreviewSessions() {
  const targets = [...previewMultiSelect.selectedList.value]
  if (targets.length === 0) return
  for (const sessionId of targets) {
    await deletePreviewSessionById(sessionId)
  }
  previewMultiSelect.setEnabled(false)
}

watch(
  () => allPreviewSessionIds.value,
  (ids) => {
    previewMultiSelect.retain(ids)
  },
  { immediate: true },
)

watch(sections, (items) => {
  for (const item of items) sectionPage[item.scope] = item.page
})

watch(sidebarQueryNorm, () => {
  for (const scope of sectionScopes) sectionPage[scope] = 0
})

watch(currentChatSessionId, (next) => {
  sectionPage.chat = 0
  if (next) sectionOpen.chat = true
})

watch(currentDirectoryNorm, () => {
  sectionPage.directory = 0
})

function selectSession(sessionId: string) {
  preview.selectSession(sessionId)
  if (ui.isCompactLayout) ui.setSessionSwitcherOpen(false)
  probeVisibleSessions()
}
</script>

<template>
  <section
    data-selection-context
    class="flex h-full min-h-0 flex-col bg-sidebar text-sidebar-foreground"
    :class="ui.isCompactLayout ? '' : 'border-r border-border'"
  >
    <div class="h-9 pt-1 select-none pl-3.5 pr-2 flex-shrink-0">
      <div class="flex h-full items-center justify-between gap-2">
        <div class="min-w-0 flex items-center gap-2">
          <p class="typography-ui-label font-medium text-muted-foreground">
            {{ t('workspaceDock.preview.sessionsTitle') }}
            <span class="ml-1 font-mono text-[10px] text-muted-foreground/60"
              >({{ previewFilteredSessions.length }})</span
            >
          </p>
        </div>

        <div class="flex items-center gap-1">
          <IconButton
            :tooltip="String(t('workspaceDock.preview.emptyState.addAction'))"
            :aria-label="String(t('workspaceDock.preview.emptyState.addAction'))"
            :is-touch-pointer="ui.isTouchPointer"
            :disabled="createDialog?.busy"
            @click="createDialog?.open()"
          >
            <RiAddLine class="h-4 w-4" />
          </IconButton>

          <IconButton
            :tooltip="String(t('workspaceDock.preview.refresh'))"
            :aria-label="String(t('workspaceDock.preview.refresh'))"
            :is-touch-pointer="ui.isTouchPointer"
            :disabled="preview.loading"
            @click="refreshSessions({ forceFrameReload: true })"
          >
            <RiRefreshLine class="h-4 w-4" :class="preview.loading ? 'animate-spin' : ''" />
          </IconButton>

          <IconButton
            v-if="!previewMultiSelect.enabled.value"
            data-selection-trigger
            :tooltip="t('workspaceDock.preview.sidebar.actions.enterMultiSelect')"
            :is-touch-pointer="ui.isTouchPointer"
            @click="togglePreviewMultiSelectMode"
          >
            <RiListCheck3 class="size-4" aria-hidden="true" />
          </IconButton>
        </div>
      </div>
    </div>

    <div class="px-3 py-2 flex-shrink-0">
      <SearchInput
        :model-value="preview.sidebarQuery"
        @update:model-value="(v) => (preview.sidebarQuery = String(v || ''))"
        @search="() => {}"
        @clear="() => {}"
        :placeholder="String(t('workspaceDock.preview.sidebar.searchPlaceholder'))"
        class="text-xs"
        input-class="h-8 text-xs"
        :input-aria-label="String(t('workspaceDock.preview.sidebar.searchAria'))"
        :input-title="String(t('workspaceDock.preview.sidebar.searchAria'))"
        :search-aria-label="String(t('common.search'))"
        :search-title="String(t('common.search'))"
        :clear-aria-label="String(t('common.clear'))"
        :clear-title="String(t('common.clear'))"
        :is-touch-pointer="ui.isTouchPointer"
      />
    </div>

    <SelectionToolbar
      v-if="previewMultiSelect.enabled.value"
      class="shrink-0 border-b border-sidebar-border/60"
      :selectable-ids="filteredPreviewSessionIds"
      :selected-ids="previewMultiSelect.selectedIds.value"
      :selection-label="
        t('workspaceDock.preview.sidebar.multiSelect.selectedCount', {
          count: previewMultiSelect.selectedCount.value,
        })
      "
      :exit-label="t('workspaceDock.preview.sidebar.actions.exitMultiSelect')"
      :is-touch-pointer="ui.isTouchPointer"
      @select-all="selectAllPreviewSessions"
      @invert="invertPreviewSessionsSelection"
      @exit="togglePreviewMultiSelectMode"
    >
      <template #default="{ size }">
        <ConfirmPopover
          :title="t('workspaceDock.preview.sidebar.confirmDeleteSelected.title')"
          :description="
            t('workspaceDock.preview.sidebar.confirmDeleteSelected.description', {
              count: previewMultiSelect.selectedCount.value,
            })
          "
          :confirm-text="t('workspaceDock.preview.sidebar.actions.deleteSelected')"
          variant="destructive"
          @confirm="deleteSelectedPreviewSessions"
        >
          <SidebarIconButton
            data-keyboard-option
            :size="size"
            destructive
            :tooltip="t('workspaceDock.preview.sidebar.actions.deleteSelected')"
            :is-touch-pointer="ui.isTouchPointer"
            :disabled="!previewMultiSelect.selectedCount.value"
            @click.stop
          >
            <RiDeleteBinLine class="size-3.5" aria-hidden="true" />
          </SidebarIconButton>
        </ConfirmPopover>
      </template>
    </SelectionToolbar>

    <div class="min-h-0 flex-1 overflow-x-hidden overflow-y-auto pb-2">
      <SidebarPagedSection
        v-for="section in sections"
        :key="section.scope"
        :title="section.title"
        :open="section.open"
        :count="section.count"
        :page="section.page"
        :page-count="section.pageCount"
        :paging="preview.loading"
        :loading="preview.loading && !previewSessions.length"
        :empty-message="section.emptyMessage"
        @update:open="sectionOpen[section.scope] = $event"
        @update:page="sectionPage[section.scope] = $event"
      >
        <PreviewSessionRow
          v-for="session in section.rows"
          :key="session.id"
          v-model:draft="renameDraft"
          :session-id="session.id"
          :subtitle="previewSessionSubtitle(session)"
          :status-class="sessionDotClass(session)"
          :status-label="sessionDotLabel(session)"
          :active="!previewMultiSelect.enabled.value && preview.activeSessionId === session.id"
          :multi-select="previewMultiSelect.enabled.value"
          :selected="previewMultiSelect.isSelected(session.id)"
          :renaming="isInlineRenameSession(session, section.scope)"
          :rename-busy="renameBusy"
          :rename-error="renameError"
          :can-save-rename="canSaveRename"
          :compact-touch="ui.isCompactTouch"
          :draggable="canDragPreviewSession(session, section.scope)"
          @click="handlePreviewSessionRowClick(session, section.scope, $event)"
          @dragstart="handlePreviewSessionRowDragStart($event, session, section.scope)"
          @menu="openRowMenu(session.id, section.scope, $event)"
          @rename="startRenameSession(session.id, section.scope)"
          @save="saveRenameSession"
          @cancel="cancelRenameSession"
        />
      </SidebarPagedSection>
    </div>

    <OptionMenu
      :open="rowMenuOpen"
      :groups="rowMenuGroups"
      :title="String(t('common.actions'))"
      :mobile-title="String(t('common.actions'))"
      :searchable="false"
      :is-compact-touch="ui.isCompactTouch"
      :desktop-fixed="true"
      desktop-placement="bottom-end"
      :desktop-anchor-el="rowMenuAnchorEl"
      @update:open="setRowMenuOpen"
      @select="onRowMenuSelect"
    />

    <PreviewCreateSessionDialog ref="createDialog" :directory="currentDirectory" @created="onSessionCreated" />

    <FormDialog
      :open="editDialogOpen"
      :title="String(t('workspaceDock.preview.editDialog.title'))"
      :description="String(t('workspaceDock.preview.editDialog.description'))"
      max-width="max-w-md"
      initial-focus="[data-preview-target-url]"
      @update:open="editDialogOpen = $event"
    >
      <form :id="editFormId" class="grid gap-3" novalidate @submit.prevent="saveEditDialog">
        <label class="grid gap-1 text-xs font-medium">
          {{ t('workspaceDock.preview.emptyState.targetUrlLabel') }}
          <Input
            v-model="editTargetUrl"
            data-preview-target-url
            type="url"
            :readonly="editBusy"
            :aria-invalid="editError ? true : undefined"
            :aria-describedby="editError ? `${editFormId}-error` : undefined"
            class="font-mono text-xs"
          />
        </label>
        <p v-if="editError" :id="`${editFormId}-error`" class="text-xs text-destructive" role="alert">
          {{ editError }}
        </p>
      </form>
      <template #footer>
        <div class="flex w-full flex-wrap items-center justify-end gap-2">
          <Button type="button" size="sm" variant="outline" :disabled="editBusy" @click="editDialogOpen = false">
            {{ t('common.cancel') }}
          </Button>
          <Button type="submit" :form="editFormId" size="sm" :disabled="editBusy || !editTargetUrl.trim()">
            {{ t('common.save') }}
          </Button>
        </div>
      </template>
    </FormDialog>

    <FormDialog
      :open="renameDialogOpen"
      :title="String(t('workspaceDock.preview.renameDialog.title'))"
      :description="String(t('workspaceDock.preview.renameDialog.description'))"
      max-width="max-w-md"
      initial-focus="[data-preview-rename-name]"
      @update:open="(open) => !open && cancelRenameSession()"
    >
      <form :id="renameFormId" class="grid gap-3" novalidate @submit.prevent="saveRenameSession">
        <div class="grid gap-1">
          <label :for="`${renameFormId}-name`" class="text-xs font-medium">
            {{ t('workspaceDock.preview.emptyState.sessionIdLabel') }}
          </label>
          <Input
            :id="`${renameFormId}-name`"
            v-model="renameDraft"
            data-preview-rename-name
            autocapitalize="off"
            autocomplete="off"
            :spellcheck="false"
            :placeholder="t('workspaceDock.preview.emptyState.sessionIdPlaceholder')"
            :readonly="renameBusy"
            :aria-invalid="renameError ? true : undefined"
            :aria-describedby="renameError ? `${renameFormId}-error` : `${renameFormId}-help`"
            class="font-mono text-xs"
          />
          <p :id="`${renameFormId}-help`" class="text-xs text-muted-foreground">
            {{ t('workspaceDock.preview.emptyState.sessionIdHelp') }}
          </p>
        </div>
        <p v-if="renameError" :id="`${renameFormId}-error`" class="text-xs text-destructive" role="alert">
          {{ renameError }}
        </p>
      </form>
      <template #footer>
        <div class="flex w-full flex-wrap items-center justify-end gap-2">
          <Button type="button" size="sm" variant="outline" :disabled="renameBusy" @click="cancelRenameSession">
            {{ t('common.cancel') }}
          </Button>
          <Button type="submit" :form="renameFormId" size="sm" :disabled="renameBusy">
            {{ renameBusy ? t('common.saving') : t('common.save') }}
          </Button>
        </div>
      </template>
    </FormDialog>
  </section>
</template>
