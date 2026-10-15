<script setup lang="ts">
import { computed, nextTick, ref, type Component } from 'vue'
import { useI18n } from 'vue-i18n'
import {
  RiChat4Line,
  RiCloseLine,
  RiFolder6Line,
  RiGitMergeLine,
  RiGlobalLine,
  RiMore2Line,
  RiSettings3Line,
  RiTerminalBoxLine,
} from '@remixicon/vue'

import { WORKSPACE_MAIN_TABS, type MainTabId } from '@/app/navigation/mainTabs'
import OptionMenu from '@/components/ui/OptionMenu.vue'
import type { OptionMenuGroup, OptionMenuItem } from '@/components/ui/optionMenu.types'
import {
  WORKSPACE_WINDOW_DRAG_MIME,
  hasWorkspaceWindowDragDataTransfer,
  readWorkspaceWindowDragIdFromDataTransfer,
  readWorkspaceWindowTemplateFromDataTransfer,
  writeWorkspaceWindowTemplateToDataTransfer,
  type WorkspaceWindowTemplateDragData,
} from '@/layout/workspaceWindowDrag'
import { cn } from '@/lib/utils'
import { useUiStore, type WorkspaceWindowTab } from '@/stores/ui'
import { useWorkspacePaneHosts } from '@/layout/workspacePaneHosts'
import { useWorkspaceNavigation } from '@/app/navigation/useWorkspaceNavigation'
import { focusKeyboardRegion, isPlainNavigationKey, navigationIndex } from '@/lib/keyboardNavigation'
import { isImeComposing } from '@/lib/imeKeyboard'

const props = defineProps<{
  groupId: string
}>()

const ui = useUiStore()
const paneHosts = useWorkspacePaneHosts()
const workspaceNavigation = useWorkspaceNavigation()
const { t } = useI18n()

const TAB_ICONS: Record<MainTabId, Component> = {
  chat: RiChat4Line,
  files: RiFolder6Line,
  preview: RiGlobalLine,
  terminal: RiTerminalBoxLine,
  git: RiGitMergeLine,
  settings: RiSettings3Line,
}

const TAB_LABEL_KEYS = WORKSPACE_MAIN_TABS.reduce(
  (acc, item) => {
    acc[item.id] = item.labelKey
    return acc
  },
  {} as Record<MainTabId, string>,
)

const windowMap = computed(() => {
  const out = new Map<string, WorkspaceWindowTab>()
  for (const item of ui.workspaceWindows) {
    out.set(item.id, item)
  }
  return out
})

const group = computed(() => ui.getWorkspaceGroupById(props.groupId))

const groupTabs = computed<WorkspaceWindowTab[]>(() => {
  const target = group.value
  if (!target) return []
  const out: WorkspaceWindowTab[] = []
  for (const tabId of target.tabIds) {
    const item = windowMap.value.get(String(tabId || '').trim())
    if (item) out.push(item)
  }
  return out
})

const activeWindow = computed<WorkspaceWindowTab | null>(() => {
  const target = group.value
  const tabs = groupTabs.value
  if (!target || !tabs.length) return null

  const preferredId = String(target.activeWindowId || '').trim()
  if (preferredId) {
    const matched = tabs.find((item) => item.id === preferredId)
    if (matched) return matched
  }

  return tabs[0] || null
})

const activeWindowId = computed(() => String(activeWindow.value?.id || '').trim())
const canDragTabs = computed(() => !ui.isCompactLayout)
const isTabStripDropActive = ref(false)
const tabStripEl = ref<HTMLElement | null>(null)
const tabContextMenuOpen = ref(false)
const tabContextWindowId = ref('')
const tabContextMenuAnchorEl = ref<HTMLElement | null>(null)
const tabContextReturnId = ref('')
function tabContextReturnTarget() {
  const id = tabContextReturnId.value
  return id ? document.querySelector<HTMLElement>(`[data-workspace-tab-primary="${CSS.escape(id)}"]`) : null
}

const tabContextWindowIndex = computed(() =>
  groupTabs.value.findIndex((item) => item.id === String(tabContextWindowId.value || '').trim()),
)
const tabContextMenuTitle = computed(() => {
  const target = windowMap.value.get(String(tabContextWindowId.value || '').trim())
  return target ? windowTitle(target) : ''
})

const tabContextMenuGroups = computed<OptionMenuGroup[]>(() => {
  const index = tabContextWindowIndex.value
  const count = groupTabs.value.length
  const validTarget = index >= 0
  return [
    {
      id: 'tab-layout-actions',
      items: [
        { id: 'duplicate', label: String(t('keyboard.duplicateTab')), disabled: !validTarget },
        { id: 'split', label: String(t('keyboard.splitTab')), disabled: !validTarget },
        { id: 'detach', label: String(t('keyboard.detachTab')), disabled: !validTarget || count < 2 },
        { id: 'tab-left', label: String(t('keyboard.moveTabLeft')), disabled: index <= 0 },
        { id: 'tab-right', label: String(t('keyboard.moveTabRight')), disabled: !validTarget || index >= count - 1 },
        {
          id: 'group-left',
          label: String(t('keyboard.moveGroupLeft')),
          disabled: ui.workspaceGroups.findIndex((g) => g.id === props.groupId) <= 0,
        },
        {
          id: 'group-right',
          label: String(t('keyboard.moveGroupRight')),
          disabled: ui.workspaceGroups.findIndex((g) => g.id === props.groupId) >= ui.workspaceGroups.length - 1,
        },
        ...ui.workspaceGroups
          .filter((g) => g.id !== props.groupId)
          .flatMap((g) => {
            const number = ui.workspaceGroups.findIndex((candidate) => candidate.id === g.id) + 1
            return [
              { id: `move-group:${g.id}`, label: String(t('keyboard.moveToGroup', { number })) },
              { id: `merge-group:${g.id}`, label: String(t('keyboard.mergeIntoGroup', { number })) },
            ]
          }),
        ...ui.workspaceWindows
          .filter((w) => w.id !== tabContextWindowId.value)
          .map((w) => ({
            id: `replace:${w.id}`,
            label: String(t('keyboard.replaceTab', { title: windowTitle(w) })),
          })),
      ],
    },
    {
      id: 'tab-close-actions',
      items: [
        {
          id: 'close-current',
          label: String(t('header.windowTabs.closeCurrent')),
          icon: RiCloseLine,
          disabled: !validTarget,
        },
        {
          id: 'close-others',
          label: String(t('header.windowTabs.closeOthers')),
          icon: RiCloseLine,
          disabled: !validTarget || count <= 1,
        },
        {
          id: 'close-left',
          label: String(t('header.windowTabs.closeLeft')),
          icon: RiCloseLine,
          disabled: !validTarget || index <= 0,
        },
        {
          id: 'close-right',
          label: String(t('header.windowTabs.closeRight')),
          icon: RiCloseLine,
          disabled: !validTarget || index >= count - 1,
        },
        {
          id: 'close-all',
          label: String(t('header.windowTabs.closeAll')),
          icon: RiCloseLine,
          disabled: count === 0,
        },
      ],
    },
  ]
})

function registerPaneHost(element: unknown) {
  if (element instanceof HTMLElement) paneHosts?.set(props.groupId, element)
  else paneHosts?.delete(props.groupId)
}

function resolvePaneFocusWindowId(): string {
  const target = group.value
  if (!target) return ''
  const preferredId = String(target.activeWindowId || '').trim()
  if (preferredId && target.tabIds.includes(preferredId)) return preferredId
  return String(target.tabIds[0] || '').trim()
}

function focusPaneWindow(windowId?: string) {
  const requestedWindowId = String(windowId || '').trim()
  const hasRequestedWindow = requestedWindowId
    ? groupTabs.value.some((item) => String(item.id || '').trim() === requestedWindowId)
    : false

  const targetWindowId = hasRequestedWindow ? requestedWindowId : resolvePaneFocusWindowId()
  if (!targetWindowId) return
  if (String(ui.focusedWorkspaceWindowId || '').trim() === targetWindowId) return
  ui.setFocusedWorkspaceWindow(targetWindowId)
}

function handlePanePointerDown(event: PointerEvent) {
  // Tab controls own activation. Opening their context menu or focusing an
  // action button must not select the tab before the user chooses an action.
  if (event.button !== 0 || event.ctrlKey) return
  if (event.target instanceof Element && event.target.closest('[data-workspace-tab-id]')) return
  focusPaneWindow()
}

function handlePaneFocusIn(event: FocusEvent) {
  const target = event.target
  const tab = target instanceof HTMLElement ? target.closest<HTMLElement>('[data-workspace-tab-id]') : null
  if (tab) return
  focusPaneWindow()
}

function handleTabPointerDown(event: PointerEvent) {
  if (event.button !== 0 || event.ctrlKey) event.preventDefault()
}

function tabLabel(tab: MainTabId): string {
  return String(t(TAB_LABEL_KEYS[tab]))
}

function windowTitle(windowTab: WorkspaceWindowTab): string {
  const customTitle = String(windowTab.title || '').trim()
  if (customTitle) return customTitle

  const base = tabLabel(windowTab.mainTab)
  const siblings = groupTabs.value.filter((item) => item.mainTab === windowTab.mainTab)
  if (siblings.length <= 1) return base
  const idx = siblings.findIndex((item) => item.id === windowTab.id)
  if (idx < 0) return base
  return `${base} ${idx + 1}`
}

async function navigateToWindow(windowId: string, replace = true) {
  await workspaceNavigation.navigateToWorkspaceWindow(windowId, replace)
}

function getWorkspaceWindowGroupId(windowId: string): string {
  const targetId = String(windowId || '').trim()
  if (!targetId) return ''
  for (const item of ui.workspaceGroups) {
    if (item.tabIds.includes(targetId)) return item.id
  }
  return ''
}

function matchKeysForTab(tab: MainTabId): string[] {
  if (tab === 'chat') return ['sessionId']
  if (tab === 'files') return ['filePath']
  return []
}

function normalizeTemplateSource(source: WorkspaceWindowTemplateDragData): WorkspaceWindowTemplateDragData | null {
  const tab = source.tab
  if (!tab) return null

  const windowId = String(source.windowId || '').trim()
  const queryRaw = source.query && typeof source.query === 'object' ? source.query : {}
  const query: Record<string, string> = {}
  for (const [key, value] of Object.entries(queryRaw || {})) {
    const k = String(key || '').trim()
    const v = String(value || '').trim()
    if (!k || !v) continue
    query[k] = v
  }

  const title = String(source.title || '').trim()
  const matchKeysRaw = Array.isArray(source.matchKeys) ? source.matchKeys : []
  const matchKeys =
    matchKeysRaw
      .map((item) => String(item || '').trim())
      .filter(Boolean)
      .filter((item, idx, list) => list.indexOf(item) === idx) || []

  return {
    tab,
    ...(windowId ? { windowId } : {}),
    ...(Object.keys(query).length ? { query } : {}),
    ...(title ? { title } : {}),
    matchKeys: matchKeys.length ? matchKeys : matchKeysForTab(tab),
  }
}

function readDraggedWorkspaceWindowId(event: DragEvent, opts?: { hasTemplate?: boolean }): string {
  const fromTransfer = readWorkspaceWindowDragIdFromDataTransfer(event.dataTransfer)
  if (fromTransfer) return fromTransfer
  if (opts?.hasTemplate) return ''
  return String(ui.workspaceWindowDragId || '').trim()
}

function hasWorkspaceDragDataType(event: DragEvent): boolean {
  return hasWorkspaceWindowDragDataTransfer(event.dataTransfer)
}

type GroupDropSource =
  | {
      kind: 'window'
      windowId: string
    }
  | {
      kind: 'template'
      data: WorkspaceWindowTemplateDragData
    }

function resolveGroupDropSource(event: DragEvent): GroupDropSource | null {
  const rawTemplate = readWorkspaceWindowTemplateFromDataTransfer(event.dataTransfer)
  const normalizedTemplate = rawTemplate ? normalizeTemplateSource(rawTemplate) : null
  const templateWindowId = String(normalizedTemplate?.windowId || '').trim()
  if (templateWindowId && ui.getWorkspaceWindowById(templateWindowId)) {
    return {
      kind: 'window',
      windowId: templateWindowId,
    }
  }

  const sourceWindowId = readDraggedWorkspaceWindowId(event, { hasTemplate: Boolean(normalizedTemplate) })
  if (sourceWindowId && ui.getWorkspaceWindowById(sourceWindowId)) {
    return {
      kind: 'window',
      windowId: sourceWindowId,
    }
  }

  if (!normalizedTemplate) return null

  return {
    kind: 'template',
    data: normalizedTemplate,
  }
}

function isWindowActive(windowId: string): boolean {
  return String(windowId || '').trim() === activeWindowId.value
}

function isWindowFocused(windowId: string): boolean {
  return String(windowId || '').trim() === String(ui.focusedWorkspaceWindowId || '').trim()
}

function tabClass(windowId: string): string {
  const active = isWindowActive(windowId)
  const focused = isWindowFocused(windowId)
  return cn(
    'group flex min-w-[140px] max-w-[240px] select-none items-center rounded-md border transition-colors cursor-default active:cursor-grabbing',
    active
      ? focused
        ? 'border-primary/50 bg-background text-foreground shadow-[0_1px_0_rgba(0,0,0,0.1)]'
        : 'border-border/70 bg-secondary/40 text-foreground/85'
      : 'border-transparent bg-transparent text-muted-foreground hover:border-border/60 hover:bg-secondary/60 hover:text-foreground',
  )
}

async function activateTab(windowId: string) {
  const targetId = String(windowId || '').trim()
  const groupId = String(props.groupId || '').trim()
  if (!targetId || !groupId) return

  ui.selectWorkspaceGroup(groupId)
  ui.selectWorkspaceWindow(targetId)
  await navigateToWindow(targetId, true)
}

async function closeTab(windowId: string) {
  const targetId = String(windowId || '').trim()
  if (!targetId) return

  const restoreFocus = Boolean(tabStripEl.value?.contains(document.activeElement)) || tabContextMenuOpen.value
  const wasRouteActive = String(ui.activeWorkspaceWindowId || '').trim() === targetId
  ui.closeWorkspaceWindow(targetId)

  if (wasRouteActive) {
    const fallback = ui.activeWorkspaceWindow || ui.workspaceWindows[0] || null
    if (fallback) await navigateToWindow(fallback.id, true)
    else await workspaceNavigation.navigateToEmptyWorkspace(true)
  }
  if (restoreFocus) await focusActiveTab()
}

async function focusActiveTab() {
  await nextTick()
  const buttons = Array.from(tabStripEl.value?.querySelectorAll<HTMLElement>('[data-workspace-tab-primary]') || [])
  const target = buttons.find((button) => button.dataset.workspaceTabPrimary === activeWindowId.value) || buttons[0]
  if (target) target.focus()
  else {
    const region = document.querySelector<HTMLElement>('[data-keyboard-region]')
    if (region) focusKeyboardRegion(region)
  }
}

function onTabStripKeydown(event: KeyboardEvent) {
  if (event.defaultPrevented || isImeComposing(event) || event.altKey || event.ctrlKey || event.metaKey) return
  const target = event.target
  if (!(target instanceof HTMLElement) || !target.hasAttribute('data-workspace-tab-primary')) return
  const windowId = target.dataset.workspaceTabPrimary || ''
  if ((event.shiftKey && event.key === 'F10') || event.key === 'ContextMenu') {
    event.preventDefault()
    event.stopPropagation()
    tabContextWindowId.value = windowId
    tabContextReturnId.value = ''
    tabContextMenuAnchorEl.value = target
    tabContextMenuOpen.value = true
    return
  }
  if (!isPlainNavigationKey(event) || event.shiftKey) return
  if (event.key === 'Delete') {
    event.preventDefault()
    event.stopPropagation()
    void closeTab(windowId)
    return
  }
  if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return
  const tabs = groupTabs.value
  const index = navigationIndex(
    event.key,
    tabs.findIndex((tab) => tab.id === windowId),
    tabs.length,
  )
  if (index === null) return
  event.preventDefault()
  event.stopPropagation()
  void activateTab(tabs[index]!.id).then(focusActiveTab)
}

function openTabContextMenu(event: MouseEvent, windowId: string) {
  const targetId = String(windowId || '').trim()
  const anchor = event.currentTarget
  if (!targetId || !(anchor instanceof HTMLElement)) return
  tabContextWindowId.value = targetId
  tabContextReturnId.value = ''
  tabContextMenuAnchorEl.value = anchor
  tabContextMenuOpen.value = true
}

async function closeTabIds(windowIds: string[], fallbackWindowId = '') {
  const ids = [...new Set(windowIds.map((id) => String(id || '').trim()).filter(Boolean))].filter((id) =>
    Boolean(ui.getWorkspaceWindowById(id)),
  )
  if (!ids.length) return

  const routeActiveId = String(ui.activeWorkspaceWindowId || '').trim()
  const removesRouteActive = Boolean(routeActiveId && ids.includes(routeActiveId))

  for (const id of ids) ui.closeWorkspaceWindow(id)
  if (removesRouteActive) {
    const preferredFallbackId = String(fallbackWindowId || '').trim()
    const fallback =
      (preferredFallbackId ? ui.getWorkspaceWindowById(preferredFallbackId) : null) ||
      ui.activeWorkspaceWindow ||
      ui.workspaceWindows[0] ||
      null
    if (fallback) await navigateToWindow(fallback.id, true)
    else await workspaceNavigation.navigateToEmptyWorkspace(true)
  }
  await focusActiveTab()
}

async function handleTabContextMenuSelect(item: OptionMenuItem) {
  const targetId = String(tabContextWindowId.value || '').trim()
  const tabIds = groupTabs.value.map((tab) => tab.id)
  const index = tabIds.indexOf(targetId)
  if (!targetId || index < 0) return

  let destinationId = targetId
  let handled = true
  if (item.id === 'duplicate') destinationId = ui.duplicateWorkspaceWindow(targetId)
  else if (item.id === 'split') {
    ui.splitWorkspaceWindowToRight(targetId)
    destinationId = ui.activeWorkspaceWindowId
  } else if (item.id === 'detach') ui.ungroupWorkspaceWindow(targetId)
  else if (item.id === 'tab-left' || item.id === 'tab-right')
    ui.reorderWorkspaceTab(targetId, item.id === 'tab-left' ? -1 : 1)
  else if (item.id === 'group-left' || item.id === 'group-right') {
    const groupIndex = ui.workspaceGroups.findIndex((g) => g.id === props.groupId)
    ui.moveWorkspaceGroupToIndex(props.groupId, groupIndex + (item.id === 'group-left' ? -1 : 2))
  } else if (item.id.startsWith('move-group:')) {
    ui.moveWorkspaceWindowToGroup(targetId, item.id.slice('move-group:'.length), { activateTargetGroup: true })
  } else if (item.id.startsWith('merge-group:')) {
    const groupId = item.id.slice('merge-group:'.length)
    for (const id of tabIds) ui.moveWorkspaceWindowToGroup(id, groupId, { activateTargetGroup: true })
    ui.selectWorkspaceWindow(targetId)
  } else if (item.id.startsWith('replace:')) {
    destinationId = item.id.slice('replace:'.length)
    ui.replaceWorkspaceWindowContent(destinationId, targetId)
  } else handled = false
  if (handled) {
    tabContextReturnId.value = destinationId
    await navigateToWindow(destinationId, true)
    return
  }

  if (item.id === 'close-current') {
    await closeTab(targetId)
    return
  }
  if (item.id === 'close-others') {
    await closeTabIds(
      tabIds.filter((id) => id !== targetId),
      targetId,
    )
    return
  }
  if (item.id === 'close-left') {
    await closeTabIds(tabIds.slice(0, index), targetId)
    return
  }
  if (item.id === 'close-right') {
    await closeTabIds(tabIds.slice(index + 1), targetId)
    return
  }
  if (item.id === 'close-all') await closeTabIds(tabIds)
}

function clearTabStripDropState() {
  isTabStripDropActive.value = false
}

function handleTabDragStart(event: DragEvent, windowId: string) {
  if (!canDragTabs.value) {
    event.preventDefault()
    return
  }

  const id = String(windowId || '').trim()
  if (!id) return

  ui.beginWorkspaceWindowDrag(id)

  const transfer = event.dataTransfer
  if (!transfer) return
  transfer.effectAllowed = 'move'

  const source = ui.getWorkspaceWindowById(id)
  const wroteTemplate = source
    ? writeWorkspaceWindowTemplateToDataTransfer(transfer, {
        tab: source.mainTab,
        query: source.routeQuery,
        title: String(source.title || '').trim() || undefined,
        matchKeys: matchKeysForTab(source.mainTab),
        windowId: id,
      })
    : false

  try {
    transfer.setData(WORKSPACE_WINDOW_DRAG_MIME, id)
  } catch {
    // Ignore runtime restrictions on custom MIME writes.
  }

  if (!wroteTemplate) {
    try {
      transfer.setData('text/plain', id)
    } catch {
      // Ignore plain-text write failures when custom MIME is available.
    }
  }
}

function handleTabDragEnd(_event: DragEvent) {
  ui.endWorkspaceWindowDrag()
  clearTabStripDropState()
}

function handleTabStripDragOver(event: DragEvent) {
  const source = resolveGroupDropSource(event)
  if (!source && !hasWorkspaceDragDataType(event)) return

  event.preventDefault()
  if (event.dataTransfer) {
    event.dataTransfer.dropEffect = source?.kind === 'window' ? 'move' : 'copy'
  }
  isTabStripDropActive.value = true
}

function handleTabStripDragLeave(event: DragEvent) {
  if (!isTabStripDropActive.value) return
  const host = event.currentTarget as HTMLElement | null
  const related = event.relatedTarget as Node | null
  if (host && related && host.contains(related)) return
  clearTabStripDropState()
}

async function handleTabStripDrop(event: DragEvent) {
  const targetGroupId = String(props.groupId || '').trim()
  const source = resolveGroupDropSource(event)

  clearTabStripDropState()
  ui.endWorkspaceWindowDrag()

  if (!targetGroupId || !source) return

  event.preventDefault()

  let sourceWindowId = ''
  if (source.kind === 'window') {
    sourceWindowId = source.windowId
  } else {
    sourceWindowId = ui.openWorkspaceWindow(source.data.tab, {
      activate: false,
      query: source.data.query,
      title: source.data.title,
      matchKeys: source.data.matchKeys,
      reuseExisting: false,
    })
  }

  if (!sourceWindowId) return

  const sourceGroupId = getWorkspaceWindowGroupId(sourceWindowId)
  if (sourceGroupId !== targetGroupId) {
    const moved = ui.moveWorkspaceWindowToGroup(sourceWindowId, targetGroupId, { activateTargetGroup: true })
    if (!moved) return
  } else {
    ui.selectWorkspaceGroup(targetGroupId)
    ui.selectWorkspaceWindow(sourceWindowId)
  }

  await navigateToWindow(sourceWindowId, true)
}
</script>

<template>
  <section
    class="app-region-no-drag flex h-full min-h-0 flex-col border-l border-border/60 bg-background"
    :data-workspace-pane-group="groupId"
    @pointerdown.capture="handlePanePointerDown"
    @focusin="handlePaneFocusIn"
  >
    <div
      class="relative min-h-0 border-b border-border/60 bg-secondary/20 px-1.5 py-1"
      :class="isTabStripDropActive ? 'bg-primary/10 ring-1 ring-inset ring-primary/50' : ''"
      @dragover="handleTabStripDragOver"
      @dragleave="handleTabStripDragLeave"
      @drop="void handleTabStripDrop($event)"
    >
      <div
        ref="tabStripEl"
        role="tablist"
        :aria-label="String(t('keyboard.workspaceTabs'))"
        data-keyboard-region
        tabindex="-1"
        class="flex min-w-0 items-center gap-1 overflow-x-auto"
        @keydown="onTabStripKeydown"
      >
        <div
          v-for="windowTab in groupTabs"
          :key="`${groupId}:${windowTab.id}`"
          :data-workspace-tab-id="windowTab.id"
          :data-workspace-tab-main="windowTab.mainTab"
          :class="tabClass(windowTab.id)"
          :draggable="canDragTabs"
          @pointerdown="handleTabPointerDown"
          @contextmenu.prevent.stop="openTabContextMenu($event, windowTab.id)"
          @dragstart="handleTabDragStart($event, windowTab.id)"
          @dragend="handleTabDragEnd($event)"
        >
          <button
            type="button"
            :data-workspace-tab-primary="windowTab.id"
            :id="`workspace-tab-${windowTab.id}`"
            role="tab"
            :aria-selected="isWindowActive(windowTab.id)"
            :aria-controls="`workspace-panel-${windowTab.id}`"
            :tabindex="isWindowActive(windowTab.id) ? 0 : -1"
            class="flex min-w-0 flex-1 select-none items-center gap-2 rounded-md px-2 py-1.5 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset"
            :title="windowTitle(windowTab)"
            @click="void activateTab(windowTab.id)"
          >
            <component :is="TAB_ICONS[windowTab.mainTab]" class="h-3.5 w-3.5 flex-shrink-0" />
            <span class="truncate text-[12px] font-medium">{{ windowTitle(windowTab) }}</span>
          </button>

          <button
            type="button"
            class="inline-flex h-6 w-6 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-secondary/80 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            :aria-label="String(t('keyboard.tabActions', { title: windowTitle(windowTab) }))"
            @click.stop="openTabContextMenu($event, windowTab.id)"
          >
            <RiMore2Line class="h-3.5 w-3.5" />
          </button>

          <button
            type="button"
            class="mr-1 inline-flex h-6 w-6 items-center justify-center rounded-md text-muted-foreground transition-opacity hover:bg-secondary/80 hover:text-foreground"
            :class="
              isWindowActive(windowTab.id)
                ? 'opacity-100'
                : 'opacity-0 group-hover:opacity-100 group-focus-within:opacity-100'
            "
            :title="String(t('header.windowTabs.closeCurrent'))"
            :aria-label="String(t('header.windowTabs.closeCurrent'))"
            @click.stop="void closeTab(windowTab.id)"
          >
            <RiCloseLine class="h-3.5 w-3.5" />
          </button>
        </div>

        <div v-if="!groupTabs.length" class="px-2 py-1 text-[11px] text-muted-foreground">
          {{ t('header.windowTabs.splitNoContent') }}
        </div>
      </div>

      <OptionMenu
        v-model:open="tabContextMenuOpen"
        :return-focus-target="tabContextReturnTarget"
        :groups="tabContextMenuGroups"
        :title="tabContextMenuTitle"
        :searchable="false"
        :is-compact-touch="false"
        :desktop-anchor-el="tabContextMenuAnchorEl"
        desktop-placement="bottom-start"
        desktop-class="w-56"
        @select="void handleTabContextMenuSelect($event)"
      />

      <div
        v-if="isTabStripDropActive"
        class="workspace-tab-strip-drop-overlay pointer-events-none absolute inset-x-1.5 inset-y-1 flex items-center justify-center rounded-md border border-dashed border-primary/50 bg-primary/10"
        aria-hidden="true"
      >
        <span class="px-2 text-[10px] font-medium text-primary">
          {{ t('header.windowTabs.dropIntoTabs') }}
        </span>
      </div>
    </div>

    <div :ref="registerPaneHost" class="relative min-h-0 flex-1 overflow-hidden bg-background">
      <div v-if="!activeWindowId" class="flex h-full items-center justify-center px-4 text-xs text-muted-foreground">
        {{ t('header.windowTabs.splitNoContent') }}
      </div>
    </div>
  </section>
</template>
