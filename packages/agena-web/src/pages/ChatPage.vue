<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch, type Component } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { RiScissorsLine } from '@remixicon/vue'

import ChatPageView from './chat/ChatPageView.vue'
import type { ChatPageViewContext } from './chat/chatPageViewContext'

import { copyTextToClipboard } from '@/lib/clipboard'
import { readSessionIdFromFullPath, readSessionIdFromQuery } from '@/app/navigation/sessionQuery'
import { useChatStore } from '@/stores/chat'
import { useDirectoryStore } from '@/stores/directory'
import { useDirectorySessionStore } from '@/stores/directorySessionStore'
import { useSessionActivityStore } from '@/stores/sessionActivity'
import { useSettingsStore } from '@/stores/settings'
import { useUiStore } from '@/stores/ui'
import { useToastsStore } from '@/stores/toasts'

import { useMessageStreaming } from '@/composables/chat/useMessageStreaming'
import { useChatAttachments } from './chat/useChatAttachments'
import { useChatScrollNav } from './chat/useChatScrollNav'
import { useChatComposerLayout } from './chat/useChatComposerLayout'
import { useChatModelSelection } from './chat/useChatModelSelection'
import { useChatCommands } from './chat/useChatCommands'
import { useChatSessionActions } from './chat/useChatSessionActions'
import { useChatRunUi } from './chat/useChatRunUi'
import { useChatTranscriptVim } from './chat/useChatTranscriptVim'
import {
  composerWordRangeAfter,
  composerWordRangeBefore,
  nextComposerWordBoundary,
  previousComposerWordBoundary,
} from './chat/composerWordNavigation'
import PlanViewerDialog from '@/components/chat/PlanViewerDialog.vue'
import { openComposerInputMenu } from './chat/composerInputMenus'
import { formatTimeHM } from '@/i18n/intl'
import { useChatRenderBlocks } from './chat/useChatRenderBlocks'
import { useChatMessageActions } from './chat/useChatMessageActions'
import { deriveSendRunConfig } from './chat/modelSendDefaults'
import { useWorkspacePaneContext } from '@/app/workspace/workspacePaneContext'
import type { OptionMenuGroup, OptionMenuItem } from '@/components/ui/optionMenu.types'
import type { TranscriptDisplayPart } from '@/components/chat/messageList.types'
import type { MessageEntry } from '@/types/chat'
import type { JsonObject, JsonValue } from '@/types/json'
import {
  DEFAULT_CHAT_ACTIVITY_EXPAND_KEYS,
  DEFAULT_CHAT_ACTIVITY_EXPANDED_TOOL_FILTERS,
  normalizeChatActivityDefaultExpanded,
  normalizeChatToolActivityFilters,
  normalizeChatToolExpansionOverrides,
  resolveChatToolDefaultExpanded,
  type ChatActivityExpandKey,
  type ChatToolExpansionOverrides,
} from '@/lib/chatActivity'

type ComposerActionItem = { id: string; label: string; description?: string; icon?: Component; disabled?: boolean }

type ComposerExpose = {
  shellEl?: HTMLDivElement | { value: HTMLDivElement | null } | null
  textareaEl?: HTMLTextAreaElement | { value: HTMLTextAreaElement | null } | null
  openFilePicker?: () => void
}

type OptionMenuExpose = {
  containsTarget?: (target: Node | null) => boolean
  focusSearch?: () => void
}

type OutgoingMessagePart =
  | { type: 'text'; text: string }
  | {
      type: 'activity'
      activity: {
        id: string
        payload: {
          activity_type: 'resource'
          kind: 'file' | 'image' | 'audio' | 'video' | 'pdf'
          reference: { reference_type: 'workspace_path'; path: string }
          name: string
          media_type?: string
          size_bytes?: number
        }
      }
    }

const route = useRoute()
const router = useRouter()
const chat = useChatStore()
const workspacePane = useWorkspacePaneContext()
const isFocusedWorkspacePane = computed(() => !workspacePane || workspacePane.isFocused.value)
const directoryStore = useDirectoryStore()
const directorySessions = useDirectorySessionStore()
const activity = useSessionActivityStore()
const settings = useSettingsStore()
const ui = useUiStore()
const toasts = useToastsStore()
const { t } = useI18n()

const orphanDraft = ref('')
const draft = computed<string>({
  get() {
    const sid = chat.selectedSessionId
    if (!sid) return orphanDraft.value
    return chat.getComposerDraft(sid)
  },
  set(value) {
    const sid = chat.selectedSessionId
    if (!sid) {
      orphanDraft.value = value
      return
    }
    chat.setComposerDraft(sid, value)
  },
})
const sending = ref(false)

const composerRef = ref<ComposerExpose | null>(null)
const attachments = useChatAttachments({ toasts, composerRef })
const {
  attachedFiles,
  attachmentsBusy,
  attachProjectDialogOpen,
  attachProjectPath,
  formatBytes,
  handleDrop,
  handlePaste,
  handleFileInputChange,
  removeAttachment,
  clearAttachments,
  openFilePicker,
  openProjectAttachDialog,
  addProjectAttachment,
} = attachments

const editorFullscreen = ref(false)
const editorClosing = ref(false)

const sessionActionsMenuRef = ref<OptionMenuExpose | null>(null)

const composerActionMenuOpen = ref(false)
const composerActionMenuQuery = ref('')
const composerActionMenuAnchorRef = ref<HTMLElement | null>(null)
const attachmentsPanelOpen = ref(false)

const composerActionItems = computed<ComposerActionItem[]>(() => [
  {
    id: 'compact',
    label: String(t('chat.composer.actions.compact.label')),
    description: String(t('chat.composer.actions.compact.description')),
    icon: RiScissorsLine,
    disabled: !chat.selectedSessionId || compactBusy.value,
  },
])

const filteredComposerActionItems = computed<ComposerActionItem[]>(() => {
  const q = composerActionMenuQuery.value.trim().toLowerCase()
  const list = composerActionItems.value
  if (!q) return list
  return list.filter((item) => {
    const label = item.label.toLowerCase()
    const desc = String(item.description || '').toLowerCase()
    return label.includes(q) || desc.includes(q) || item.id.includes(q)
  })
})

const composerActionMenuGroups = computed<OptionMenuGroup[]>(() => [
  {
    id: 'composer-actions',
    items: filteredComposerActionItems.value as OptionMenuItem[],
  },
])

const sessionDirectory = computed(() => chat.selectedSessionDirectory || directoryStore.currentDirectory || '')
const composerFullscreenActive = computed(() => editorFullscreen.value || editorClosing.value)
const sessionTitle = computed(() => {
  const s = asRecord(chat.selectedSession)
  const title = typeof s?.title === 'string' ? s.title.trim() : ''
  const slug = typeof s?.slug === 'string' ? s.slug.trim() : ''
  return title || slug
})

const workspaceChatTabTitle = computed(() => {
  const title = String(sessionTitle.value || '').trim()
  if (title) return title
  return String(t('nav.chat'))
})

watch(
  () => [route.path, route.query, workspaceChatTabTitle.value] as const,
  ([path, _query, title]) => {
    if (!String(path || '').startsWith('/chat')) return
    ui.setWorkspaceWindowTitleFromRoute(route.query, title)
  },
  { immediate: true, deep: true },
)
const composerControlsRef = ref<HTMLDivElement | null>(null)
const composerPickerRef = ref<OptionMenuExpose | null>(null)
const composerPickerOpen = ref<null | 'model' | 'thinking' | 'speed'>(null)
const modelPickerQuery = ref('')
const thinkingPickerQuery = ref('')
const speedPickerQuery = ref('')

const pageRef = ref<HTMLElement | null>(null)
const composerBarRef = ref<HTMLElement | null>(null)
const transcriptSearchInputRef = ref<HTMLInputElement | null>(null)
const planViewerOpen = ref(false)

const modelTriggerRef = ref<HTMLElement | null>(null)
const thinkingTriggerRef = ref<HTMLElement | null>(null)
const speedTriggerRef = ref<HTMLElement | null>(null)

const composerPickerStyle = ref<Record<string, string>>({ left: '8px' })

// Composer sizing + fullscreen layout.
const COMPOSER_DIVIDER_HIT_PX = 12
const composerShellHeight = ref(0)

let modelSelection: ReturnType<typeof useChatModelSelection>

type ModelSlugPickerOption = {
  value?: string
  label?: string
  providerId?: string
  adapterId?: string
  modelId?: string
  description?: string
}

function getComposerTextareaEl(composer: ComposerExpose | null): HTMLTextAreaElement | null {
  const textarea = composer?.textareaEl
  if (!textarea) return null
  return textarea instanceof HTMLTextAreaElement ? textarea : textarea.value
}

function asRecord(value: JsonValue): JsonObject {
  return typeof value === 'object' && value !== null ? (value as JsonObject) : {}
}

function getRecord(value: JsonValue, key: string): JsonObject {
  const root = asRecord(value)
  const nested = root[key]
  return typeof nested === 'object' && nested !== null ? (nested as JsonObject) : {}
}

function getSelectedSessionRevertId(): string {
  const session = asRecord(chat.selectedSession)
  const revert = getRecord(session, 'revert')
  return typeof revert?.messageID === 'string' ? revert.messageID.trim() : ''
}

const chatCommands = useChatCommands({
  draft,
  composerRef,
  composerPickerOpen,
  onSend: send,
})

const {
  commandOpen,
  commandQuery,
  commandIndex,
  loadCommands,
  insertCommand,
  handleDraftInput: handleDraftInputBase,
  handleDraftKeydown: handleDraftKeydownInner,
} = chatCommands

function handleDraftInput() {
  handleDraftInputBase()
  ui.setGlobalSelection('chat-input', chat.selectedSessionId || 'composer', {
    meta: { source: 'chat-composer-input' },
  })
}

modelSelection = useChatModelSelection({
  chat,
  composerControlsRef,
  composerPickerOpen,
  composerPickerStyle,
  modelTriggerRef,
  thinkingTriggerRef,
  speedTriggerRef,
  modelPickerQuery,
  onOpenComposerPicker: () => {
    openComposerInputMenu('picker', {
      closeAttachments: closeAttachmentsPanel,
      closeActions: closeComposerActionMenu,
      closePicker: closeComposerPickerMenu,
    })
  },
  commandOpen,
  commandQuery,
  commandIndex,
})

const composerPickerTitle = computed(() => {
  if (composerPickerOpen.value === 'model') return String(t('chat.composer.picker.modelTitle'))
  if (composerPickerOpen.value === 'thinking') return String(t('chat.composer.picker.thinkingTitle'))
  if (composerPickerOpen.value === 'speed') return String(t('chat.composer.picker.speedTitle'))
  return String(t('chat.composer.picker.optionsTitle'))
})

const composerPickerSearchable = computed(() => {
  return Boolean(composerPickerOpen.value)
})

const composerPickerSearchPlaceholder = computed(() => {
  if (composerPickerOpen.value === 'model') return String(t('chat.composer.picker.searchModels'))
  if (composerPickerOpen.value === 'thinking') return String(t('chat.composer.picker.searchThinkingModes'))
  if (composerPickerOpen.value === 'speed') return String(t('chat.composer.picker.searchSpeedModes'))
  return String(t('chat.composer.picker.searchOptions'))
})

const composerPickerQuery = computed(() => {
  if (composerPickerOpen.value === 'model') return modelPickerQuery.value
  if (composerPickerOpen.value === 'thinking') return thinkingPickerQuery.value
  if (composerPickerOpen.value === 'speed') return speedPickerQuery.value
  return ''
})

function setComposerPickerQuery(value: string) {
  const next = String(value || '')
  if (composerPickerOpen.value === 'model') {
    modelPickerQuery.value = next
    return
  }
  if (composerPickerOpen.value === 'thinking') {
    thinkingPickerQuery.value = next
    return
  }
  if (composerPickerOpen.value === 'speed') {
    speedPickerQuery.value = next
  }
}

const composerPickerHelperText = computed(() => '')

const composerPickerEmptyText = computed(() => {
  if (composerPickerOpen.value === 'model') return String(t('chat.composer.picker.emptyModels'))
  if (composerPickerOpen.value === 'thinking') return String(t('chat.composer.picker.emptyThinkingModes'))
  if (composerPickerOpen.value === 'speed') return String(t('chat.composer.picker.emptySpeedModes'))
  return String(t('chat.composer.picker.emptyOptions'))
})

const composerPickerGroups = computed<OptionMenuGroup[]>(() => {
  if (composerPickerOpen.value === 'model') {
    const groups: OptionMenuGroup[] = [
      {
        id: 'model-default',
        title: String(t('common.default')),
        collapsible: false,
        items: [
          {
            id: 'model:default',
            label: String(t('chat.composer.model.autoDefault')),
            description: String(t('chat.composer.model.autoDefaultDescription')),
            checked: modelSelection.modelSource.value === 'default' || modelSelection.modelSource.value === 'auto',
            keywords: 'auto default model',
          },
        ],
      },
    ]

    const byProvider = new Map<string, OptionMenuItem[]>()
    for (const opt of modelSelection.filteredModelSlugOptions.value as ModelSlugPickerOption[]) {
      const providerId = String(opt?.providerId || '').trim() || String(t('common.other'))
      const adapterId = String(opt?.adapterId || '').trim()
      const modelId = String(opt?.modelId || '').trim() || String(opt?.value || '').trim()
      const label = String(opt?.label || '').trim() || modelId
      const value = String(opt?.value || '').trim()
      if (!value) continue
      const list = byProvider.get(providerId) || []
      list.push({
        id: `model:${value}`,
        label: label || value,
        description: adapterId ? `${adapterId} / ${modelId}` : modelId,
        checked: value === modelSelection.selectedModelSlug.value,
        keywords: `${value} ${providerId} ${adapterId} ${modelId} ${label}`,
        monospace: true,
      })
      byProvider.set(providerId, list)
    }

    for (const providerId of Array.from(byProvider.keys()).sort((a, b) => a.localeCompare(b))) {
      groups.push({
        id: `provider:${providerId}`,
        title: providerId,
        subtitle: `${byProvider.get(providerId)?.length || 0} model(s)`,
        collapsible: true,
        items: byProvider.get(providerId) || [],
      })
    }

    return groups
  }

  if (composerPickerOpen.value === 'thinking') {
    const query = thinkingPickerQuery.value.trim().toLowerCase()
    const thinkingItems = modelSelection.thinkingModeOptions.value
      .filter((option) => {
        if (!query) return true
        return `${option.label} ${option.value} ${option.description}`.toLowerCase().includes(query)
      })
      .map((option) => ({
        id: `thinking:${option.value}`,
        label: option.label,
        description: option.description,
        checked: option.value === modelSelection.selectedThinkingMode.value,
        keywords: `${option.label} ${option.value} ${option.description}`,
      }))

    return [
      {
        id: 'thinking-default',
        title: String(t('common.default')),
        items: [
          {
            id: 'thinking:default',
            label: String(t('common.default')),
            description: String(t('chat.composer.model.defaultThinkingDescription')),
            checked: modelSelection.thinkingModeSource.value !== 'manual',
            keywords: 'default thinking mode',
          },
        ],
      },
      {
        id: 'thinking-modes',
        title: String(t('chat.composer.picker.thinkingTitle')),
        subtitle: String(t('chat.composer.picker.availableCount', { count: thinkingItems.length })),
        items: thinkingItems,
      },
    ]
  }

  if (composerPickerOpen.value === 'speed') {
    const query = speedPickerQuery.value.trim().toLowerCase()
    const speedItems = modelSelection.speedModeOptions.value
      .filter((option) => {
        if (!query) return true
        return `${option.label} ${option.value} ${option.description}`.toLowerCase().includes(query)
      })
      .map((option) => ({
        id: `speed:${option.value}`,
        label: option.label,
        description: option.description,
        checked: option.value === modelSelection.selectedSpeedMode.value,
        keywords: `${option.label} ${option.value} ${option.description}`,
      }))

    return [
      {
        id: 'speed-default',
        title: String(t('common.default')),
        items: [
          {
            id: 'speed:default',
            label: String(t('common.default')),
            description: String(t('chat.composer.model.defaultSpeedDescription')),
            checked: modelSelection.speedModeSource.value !== 'manual',
            keywords: 'default speed mode',
          },
        ],
      },
      {
        id: 'speed-modes',
        title: String(t('chat.composer.picker.speedTitle')),
        subtitle: String(t('chat.composer.picker.availableCount', { count: speedItems.length })),
        items: speedItems,
      },
    ]
  }

  return []
})

const composerPickerLoading = computed(() => {
  return composerPickerOpen.value === 'model' && modelSelection.catalogLoading.value
})

const composerPickerRefreshable = computed(() => composerPickerOpen.value === 'model')

function refreshComposerPickerOptions() {
  if (!composerPickerRefreshable.value) return
  void modelSelection.loadProvidersAndModels()
}

function closeComposerPickerMenu() {
  composerPickerOpen.value = null
  modelPickerQuery.value = ''
  thinkingPickerQuery.value = ''
  speedPickerQuery.value = ''
}

function closeAttachmentsPanel() {
  attachmentsPanelOpen.value = false
}

async function setAttachmentsPanelOpen(next: boolean) {
  if (!next) {
    closeAttachmentsPanel()
    return
  }
  openComposerInputMenu('attachments', {
    closeAttachments: closeAttachmentsPanel,
    closeActions: closeComposerActionMenu,
    closePicker: closeComposerPickerMenu,
  })
  await nextTick()
  attachmentsPanelOpen.value = true
}

function toggleAttachmentsPanel() {
  void setAttachmentsPanelOpen(!attachmentsPanelOpen.value)
}

function setComposerPickerOpen(next: boolean) {
  if (!next) closeComposerPickerMenu()
}

function handleComposerPickerSelect(item: OptionMenuItem) {
  const id = String(item.id || '')
  if (id === 'model:default') {
    void modelSelection.chooseModelDefault()
    return
  }
  if (id === 'thinking:default') {
    void modelSelection.chooseThinkingModeDefault()
    return
  }
  if (id === 'speed:default') {
    void modelSelection.chooseSpeedModeDefault()
    return
  }
  if (id.startsWith('model:')) {
    void modelSelection.chooseModelSlug(id.slice('model:'.length))
    return
  }
  if (id.startsWith('thinking:')) {
    void modelSelection.chooseThinkingMode(id.slice('thinking:'.length))
    return
  }
  if (id.startsWith('speed:')) {
    void modelSelection.chooseSpeedMode(id.slice('speed:'.length))
  }
}

const scrollNav = useChatScrollNav({
  chat,
  ui,
  getRevertId: getSelectedSessionRevertId,
  composerFullscreenActive,
  composerShellHeight,
  composerDividerHitPx: COMPOSER_DIVIDER_HIT_PX,
})

const {
  loadingOlder,
  scrollEl,
  contentEl,
  bottomEl,
  isAtBottom,
  pendingInitialScrollSessionId,
  requestInitialScroll,
  scrollToBottom,
  scheduleScrollToBottom,
  scrollToBottomOnceAfterLoad,
  ensureInitialHistoryScrollable,
  handleScroll,
  navigableMessageIds,
  navIndex,
  navBottomOffset,
  navTotalLabel,
  navPrev,
  navNext,
} = scrollNav

const composerLayout = useChatComposerLayout({
  ui,
  editorFullscreen,
  editorClosing,
  composerFullscreenActive,
  composerShellHeight,
  pageRef,
  composerBarRef,
  scrollEl,
  composerRef,
  commandOpen,
  composerPickerOpen,
  modelPickerQuery,
  scrollToBottom,
})

const {
  composerTargetHeight,
  composerSplitTopCollapsed,
  handleComposerResize,
  toggleEditorFullscreen,
  closeEditorFullscreen,
  applyComposerUserHeight,
  resetComposerHeight,
} = composerLayout

function composerTextarea(): HTMLTextAreaElement | null {
  return getComposerTextareaEl(composerRef.value)
}

function setComposerCaret(position: number) {
  void nextTick(() => {
    const textarea = composerTextarea()
    if (!textarea) return
    textarea.focus()
    textarea.setSelectionRange(position, position)
  })
}

function applyComposerEdit(start: number, end: number, replacement: string, cursor: number) {
  const value = draft.value
  if (start < 0 || end < start || end > value.length) return
  draft.value = `${value.slice(0, start)}${replacement}${value.slice(end)}`
  handleDraftInput()
  setComposerCaret(cursor)
}

function handleComposerWordKeydown(event: KeyboardEvent): boolean {
  const textarea = composerTextarea()
  if (!textarea) return false
  if (!event.altKey && !event.ctrlKey) return false
  if (event.shiftKey) return false

  const cursor = textarea.selectionStart ?? 0
  const selectionEnd = textarea.selectionEnd ?? cursor
  const wordLeft = event.key === 'ArrowLeft' || (event.altKey && event.code === 'KeyB')
  const wordRight = event.key === 'ArrowRight' || (event.altKey && event.code === 'KeyF')

  if (wordLeft || wordRight) {
    event.preventDefault()
    const base = wordLeft ? Math.min(cursor, selectionEnd) : Math.max(cursor, selectionEnd)
    const target = wordLeft
      ? previousComposerWordBoundary(textarea.value, base)
      : nextComposerWordBoundary(textarea.value, base)
    setComposerCaret(target)
    return true
  }

  const deleteBackward = event.key === 'Backspace'
  const deleteForward = event.key === 'Delete'
  if (!deleteBackward && !deleteForward) return false

  event.preventDefault()
  if (cursor !== selectionEnd) {
    applyComposerEdit(cursor, selectionEnd, '', cursor)
    return true
  }
  const range = deleteBackward
    ? composerWordRangeBefore(textarea.value, cursor)
    : composerWordRangeAfter(textarea.value, cursor)
  if (range.start === range.end) return true
  applyComposerEdit(range.start, range.end, '', range.start)
  return true
}

function handleDraftKeydown(e: KeyboardEvent) {
  if (composerFullscreenActive.value && e.key === 'Escape' && !commandOpen.value) {
    e.preventDefault()
    closeEditorFullscreen()
    return
  }
  if (handleComposerWordKeydown(e)) return
  handleDraftKeydownInner(e)
}

type RevertDiffFile = { filename: string; additions: number; deletions: number }
type RevertState = { messageID: string; diff: string; diffFiles: RevertDiffFile[]; revertedUserCount: number }

function parseDiffFiles(diffText: string): RevertDiffFile[] {
  const t = (diffText || '').trim()
  if (!t) return []

  const lines = t.replace(/\r\n/g, '\n').replace(/\r/g, '\n').split('\n')
  const byFile = new Map<string, { additions: number; deletions: number }>()
  let current: string | null = null

  function normalizeFilename(raw: string): string {
    const v = (raw || '').trim()
    if (!v || v === '/dev/null') return ''
    return v.replace(/^[ab]\//, '')
  }

  function ensure(name: string) {
    const filename = normalizeFilename(name)
    if (!filename) return ''
    if (!byFile.has(filename)) byFile.set(filename, { additions: 0, deletions: 0 })
    return filename
  }

  for (const line of lines) {
    if (line.startsWith('diff --git ')) {
      const parts = line.split(' ')
      const a = parts[2] || ''
      const b = parts[3] || ''
      current = ensure(b || a) || null
      continue
    }
    if (line.startsWith('+++ ')) {
      const name = line.replace(/^\+\+\+\s+/, '')
      current = ensure(name) || current
      continue
    }

    if (!current) continue
    const record = byFile.get(current)
    if (!record) continue

    if (line.startsWith('+') && !line.startsWith('+++')) {
      record.additions += 1
    } else if (line.startsWith('-') && !line.startsWith('---')) {
      record.deletions += 1
    }
  }

  const out: RevertDiffFile[] = []
  for (const [filename, counts] of byFile.entries()) {
    out.push({ filename, additions: counts.additions, deletions: counts.deletions })
  }
  out.sort((a, b) => a.filename.localeCompare(b.filename))
  return out
}

const revertState = computed<RevertState | null>(() => {
  const s = asRecord(chat.selectedSession)
  const rev = getRecord(s, 'revert')
  const messageID = typeof rev?.messageID === 'string' ? rev.messageID.trim() : ''
  if (!messageID) return null

  const diff = typeof rev?.diff === 'string' ? rev.diff : ''
  const diffFiles = parseDiffFiles(diff).slice(0, 12)
  const revertedUserCount = chat.messages.filter((m: MessageEntry) => {
    const id = typeof m?.info?.id === 'string' ? m.info.id : ''
    const role = String(m?.info?.role || '')
    return role === 'user' && id && id >= messageID
  }).length

  return { messageID, diff, diffFiles, revertedUserCount }
})

const revertMarkerBusy = ref(false)

// Agena rewind is destructive (later parts are dropped server-side) and has no
// "redo"/"unrevert" counterpart; the marker handlers are kept as no-ops for
// template compatibility (revertState is always null so they never fire).
async function handleRedoFromRevertMarker() {
  // no-op
}

async function handleUnrevertFromRevertMarker() {
  // no-op
}

const settingsData = computed<JsonObject>(() => asRecord(settings.data))

const activityAutoCollapseOnIdle = computed(() => settingsData.value.chatActivityAutoCollapseOnIdle !== false)

const activityDefaultExpandedKeys = computed<ChatActivityExpandKey[]>(() => {
  const s = settingsData.value
  if (s && Object.prototype.hasOwnProperty.call(s, 'chatActivityDefaultExpanded')) {
    return normalizeChatActivityDefaultExpanded(s.chatActivityDefaultExpanded)
  }
  return DEFAULT_CHAT_ACTIVITY_EXPAND_KEYS.slice()
})

const activityDefaultExpandedToolSet = computed<Set<string>>(() => {
  const s = settingsData.value
  if (s && Object.prototype.hasOwnProperty.call(s, 'chatActivityDefaultExpandedToolFilters')) {
    return new Set(normalizeChatToolActivityFilters(s.chatActivityDefaultExpandedToolFilters))
  }
  return new Set(DEFAULT_CHAT_ACTIVITY_EXPANDED_TOOL_FILTERS)
})

const activityDefaultExpandedToolOverrides = computed<ChatToolExpansionOverrides>(() =>
  normalizeChatToolExpansionOverrides(settings.data?.chatToolActivityDefaultExpandedOverrides),
)

function activityExpandKeyForPart(part: JsonObject): ChatActivityExpandKey | '' {
  const t = String(part?.type || '')
    .trim()
    .toLowerCase()
  if (t === 'tool' || (!t && typeof part?.tool === 'string')) return 'tool'
  if (t === 'reasoning' || t === 'thinking' || t === 'reasoning_content' || t === 'reasoning_details') return 'thinking'
  if (t.includes('justification')) return 'justification'
  return (t as ChatActivityExpandKey) || ''
}

function activityInitiallyExpandedForPart(part: JsonObject): boolean {
  const key = activityExpandKeyForPart(part)
  if (!key) return false
  if (key === 'tool') {
    return resolveChatToolDefaultExpanded(
      part?.tool,
      activityDefaultExpandedToolOverrides.value,
      activityDefaultExpandedToolSet.value,
    )
  }
  return activityDefaultExpandedKeys.value.includes(key)
}

const showThinking = computed(() => settingsData.value.showReasoningTraces !== false)
const showJustification = computed(() => settingsData.value.showTextJustificationActivity !== false)
const showTimestamps = computed(() => settingsData.value.showChatTimestamps !== false)

const renderBlocksApi = useChatRenderBlocks({
  chat,
  settings,
  showThinking,
  showJustification,
  revertState,
  formatTime,
})

const {
  renderBlocks,
  getTextParts,
  isReasoningPart,
  isJustificationPart,
  isMetaPart,
  MAX_VISIBLE_ACTIVITY_COLLAPSED,
  activityExpandedByBlockKey,
  activityCollapseSignal,
  collapseAllActivities,
  isActivityExpanded,
  setActivityExpanded,
} = renderBlocksApi

function transcriptPartExpanded(part: TranscriptDisplayPart): boolean {
  if (Object.prototype.hasOwnProperty.call(activityExpandedByBlockKey.value, part.key)) {
    return Boolean(activityExpandedByBlockKey.value[part.key])
  }
  if (part.defaultExpanded) return true
  return activityInitiallyExpandedForPart(part.source as JsonObject)
}

function setTranscriptPartExpanded(part: TranscriptDisplayPart, expanded: boolean) {
  setActivityExpanded(part.key, expanded)
}

const sessionActions = useChatSessionActions({
  chat,
  toasts,
  sessionTitle,
  showThinking,
  showJustification,
  copyToClipboard,
  onSessionForked: (newId) => {
    void router.push('/chat')
    void chat.selectSession(newId).catch(() => {})
  },
})

const {
  renameDialogOpen,
  renameDraft,
  renameBusy,
  compactBusy,
  openRenameDialog,
  saveRename,
  copyTranscript,
  exportTranscript,
  handleForkSession,
  handleCompactSession,
} = sessionActions

const stream = useMessageStreaming({
  selectedSessionId: computed(() => chat.selectedSessionId || null),
  messages: computed(() => chat.messages),
  revertBoundaryId: computed(() => (revertState.value?.messageID ? String(revertState.value.messageID) : null)),
})

const {
  awaitingAssistant,
  pendingSendAt,
  showOptimisticUser,
  resetForSessionSwitch,
  beginOptimisticSend,
  markOptimisticQueued,
  markOptimisticSent,
  clearOnSendFailure,
} = stream

// vue-tsc's template narrowing can be finicky around `Ref<T | null>` even when
// the runtime checks are correct. Keep this relaxed for now.
const optimisticUser = stream.optimisticUser

function closeComposerActionMenu() {
  composerActionMenuOpen.value = false
  composerActionMenuAnchorRef.value = null
  composerActionMenuQuery.value = ''
}

async function toggleComposerActionMenu(event?: MouseEvent | PointerEvent) {
  if (composerActionMenuOpen.value) {
    closeComposerActionMenu()
    return
  }
  openComposerInputMenu('actions', {
    closeAttachments: closeAttachmentsPanel,
    closeActions: closeComposerActionMenu,
    closePicker: closeComposerPickerMenu,
  })
  await nextTick()
  composerActionMenuOpen.value = true
  composerActionMenuAnchorRef.value = event?.currentTarget instanceof HTMLElement ? event.currentTarget : null
  composerActionMenuQuery.value = ''
  commandOpen.value = false
  commandQuery.value = ''
  commandIndex.value = 0
  // Desktop: focus search for quick filtering. Mobile: don't auto-focus (avoid IME popup).
  if (!ui.isTouchPointer) {
    void nextTick(() => sessionActionsMenuRef.value?.focusSearch?.())
  }
}

function runComposerActionMenu(item: ComposerActionItem | OptionMenuItem) {
  if (item.disabled) return
  closeComposerActionMenu()
  handleSessionActionRequest(item.id)
}

async function copyToClipboard(text: string) {
  const ok = await copyTextToClipboard(String(text || ''))
  if (!ok) throw new Error(t('common.copyFailed'))
}

function stringifyForClipboard(value: JsonValue): string {
  if (typeof value === 'string') return value
  try {
    return JSON.stringify(value, null, 2)
  } catch {
    return String(value ?? '')
  }
}

async function handleCopySessionError() {
  const sid = String(chat.selectedSessionId || '').trim()
  const selectedError = chat.selectedSessionError
  if (!selectedError) {
    toasts.push('error', t('chat.toasts.noSessionErrorToCopy'))
    return
  }

  const detail = selectedError.error
  const lines: string[] = []
  if (sid) lines.push(`sessionID: ${sid}`)

  const at = Number(selectedError.at || 0)
  if (Number.isFinite(at) && at > 0) {
    lines.push(`time: ${new Date(at).toISOString()}`)
  }

  const classification = String(detail.classification || '').trim()
  if (classification) lines.push(`classification: ${classification}`)

  const code = String(detail.code || '').trim()
  if (code) lines.push(`code: ${code}`)

  const name = String(detail.name || '').trim()
  if (name) lines.push(`name: ${name}`)

  const message = String(detail.message || '').trim()
  if (message) lines.push(`message: ${message}`)

  const rendered = String(detail.rendered || '').trim()
  if (rendered && rendered !== message) lines.push(`rendered: ${rendered}`)

  lines.push('raw:')
  lines.push(stringifyForClipboard(detail.raw as JsonValue))

  try {
    await copyToClipboard(lines.join('\n'))
    toasts.push('success', t('chat.toasts.copiedErrorDetails'))
  } catch {
    toasts.push('error', t('chat.toasts.failedToCopyErrorDetails'))
  }
}

const messageActions = useChatMessageActions({
  chat,
  toasts,
  route,
  router,
  sessionDirectory,
  draft,
  attachedFiles,
  clearAttachments,
  composerRef,
  getTextParts,
  copyToClipboard,
  scrollToBottom,
})

const { copiedMessageId, revertBusyMessageId, handleCopyMessage, handleForkFromMessage, handleRevertFromMessage } =
  messageActions

function isStreamingAssistantMessage(
  message: { info?: { role?: string; finish?: string; error?: unknown } } | null | undefined,
): boolean {
  if (!message?.info) return false
  const role = String(message.info.role || '')
  if (role !== 'assistant') return false
  if (message.info.error) return false
  const finish = typeof message.info.finish === 'string' ? message.info.finish.trim() : ''
  return !finish
}

let commandPointerHandler: ((event: MouseEvent | TouchEvent) => void) | null = null
let chatFocusInHandler: ((event: FocusEvent) => void) | null = null
let chatPointerUpHandler: ((event: PointerEvent) => void) | null = null

function resolveMessageIdFromTarget(target: EventTarget | null): string {
  if (!(target instanceof Element)) return ''
  const anchor = target.closest<HTMLElement>('[id^="msg-"]')
  if (!anchor) return ''
  return String(anchor.id || '')
    .replace(/^msg-/, '')
    .trim()
}

function hasTextSelection(): boolean {
  const selection = window.getSelection()
  const text = selection ? String(selection.toString() || '').trim() : ''
  return text.length > 0
}

function formatTime(ms?: number): string {
  return formatTimeHM(ms)
}

const runUi = useChatRunUi({
  chat,
  activity,
  directorySessions,
  toasts,
  modelSelection,
  draft,
  attachedFiles,
  sending,
  awaitingAssistant,
  pendingSendAt,
  renderBlocks,
  getRevertId: () => (revertState.value?.messageID ? String(revertState.value.messageID) : ''),
  onSend: send,
  collapseAllActivities,
  activityAutoCollapseOnIdle,
})

const {
  currentPhase,
  retryStatus,
  retryCountdownLabel,
  retryNextLabel,
  sessionUsage,
  formatCompactNumber,
  showAssistantPlaceholder,
  sessionEnded,
  aborting,
  canAbort,
  abortRun,
  showComposerStopAction,
  composerStopDisabled,
  composerPrimaryDisabled,
  handleComposerPrimaryAction,
  handleComposerStopAction,
} = runUi

const transcriptVim = useChatTranscriptVim({
  enabled: isFocusedWorkspacePane,
  pageRef,
  scrollEl,
  composerRef,
  searchInputRef: transcriptSearchInputRef,
  selectedSessionId: computed(() => chat.selectedSessionId),
  renderBlocks,
  draft,
  clearComposer: () => {
    draft.value = ''
    clearAttachments()
  },
  canAbort,
  abortRun,
  toggleHelp: () => ui.toggleHelpDialog(),
  openPlan: () => {
    if (!chat.selectedSessionId) {
      toasts.push('info', String(t('chat.planViewer.requiresSession')))
      return
    }
    planViewerOpen.value = true
  },
  togglePart: setTranscriptPartExpanded,
  isPartExpanded: transcriptPartExpanded,
  toasts,
})

const {
  modeLabel: transcriptVimModeLabel,
  commandLabel: transcriptVimCommandLabel,
  searchOpen: transcriptSearchOpen,
  searchQuery: transcriptSearchQuery,
  searchSummary: transcriptSearchSummary,
  selectNode: selectTranscriptNode,
  isNodeActive: isTranscriptNodeActive,
  isNodeSelected: isTranscriptNodeSelected,
  isNodeSearchMatch: isTranscriptNodeSearchMatch,
  setSearchQuery: setTranscriptSearchQuery,
  handleSearchKeydown: handleTranscriptSearchKeydown,
  closeSearch: closeTranscriptSearch,
} = transcriptVim

function handleSessionActionRequest(actionId: string) {
  switch (actionId) {
    case 'rename':
      openRenameDialog()
      break
    case 'fork':
      void handleForkSession()
      break
    case 'copy-transcript':
      void copyTranscript()
      break
    case 'export-transcript':
      void exportTranscript()
      break
    case 'compact':
      void handleCompactSession()
      break
    case 'attach-local':
      openFilePicker()
      break
    case 'attach-project':
      openProjectAttachDialog()
      break
    default:
      break
  }
}

watch(
  () => chat.selectedSessionId,
  (sid) => {
    const selectedSid = String(sid || '').trim()
    if (selectedSid) {
      ui.setGlobalSelection('chat-session', selectedSid, {
        meta: { source: 'chat-page-session-watch' },
      })
    }
    requestInitialScroll(chat.selectedSessionId)

    // Existing sessions restore their model and run modes from the server execution context.
    // New sessions fall back to the Agena runtime defaults.
    modelSelection.resetSelectionForSessionSwitch()
    modelSelection.applySessionSelection()
    activityExpandedByBlockKey.value = {}
    activityCollapseSignal.value += 1
    navIndex.value = Math.max(0, navigableMessageIds.value.length - 1)
    resetForSessionSwitch()
    revertBusyMessageId.value = ''
    editorFullscreen.value = false
    editorClosing.value = false
    applyComposerUserHeight()

    // Ensure chips reflect the session's resolved run config even when messages are cached
    // and no reactive length change occurs.
    modelSelection.applySessionSelection()

    void ensureInitialHistoryScrollable(sid)
  },
)

// Single entry/session-switch bottom landing: keep it post-flush to avoid racing DOM.
watch(
  () => [
    pendingInitialScrollSessionId.value,
    scrollEl.value,
    contentEl.value,
    bottomEl.value,
    chat.messagesLoading,
    chat.messages.length,
  ],
  () => {
    const sid = pendingInitialScrollSessionId.value
    if (!sid) return
    void scrollToBottomOnceAfterLoad(sid)
  },
  { flush: 'post' },
)

watch(
  () => chat.messages.length,
  () => {
    if (isAtBottom.value) {
      navIndex.value = Math.max(0, navigableMessageIds.value.length - 1)
    }
  },
)

function attachmentBase64(dataUrl: string): string {
  const match = /^data:[^,]*;base64,([a-z0-9+/=\s]+)$/i.exec(String(dataUrl || '').trim())
  const data = match?.[1]?.replace(/\s+/g, '') || ''
  if (!data) throw new Error('Attachment data is not a valid base64 data URL')
  return data
}

function workspaceRelativePath(inputPath: string, workspaceRoot: string): string {
  const path = String(inputPath || '')
    .trim()
    .replace(/\\/g, '/')
  const root = String(workspaceRoot || '')
    .trim()
    .replace(/\\/g, '/')
    .replace(/\/+$/, '')
  if (!path) throw new Error('Attachment path is empty')

  if (root && (path === root || path.startsWith(`${root}/`))) {
    const relative = path.slice(root.length).replace(/^\/+/, '')
    if (!relative) throw new Error('The workspace root cannot be attached as a file')
    return relative
  }
  if (path.startsWith('/') || /^[a-z]:\//i.test(path)) {
    throw new Error('The attachment must be inside the active workspace')
  }
  return path.replace(/^\.\//, '')
}

function attachmentResourceKind(mime: string, filename: string): 'file' | 'image' | 'audio' | 'video' | 'pdf' {
  const mediaType = String(mime || '')
    .trim()
    .toLowerCase()
  const name = String(filename || '')
    .trim()
    .toLowerCase()
  if (mediaType.startsWith('image/')) return 'image'
  if (mediaType.startsWith('audio/')) return 'audio'
  if (mediaType.startsWith('video/')) return 'video'
  if (mediaType === 'application/pdf' || name.endsWith('.pdf')) return 'pdf'
  return 'file'
}

function resourceComposerNode(input: {
  path: string
  filename: string
  mime: string
  size?: number
}): OutgoingMessagePart {
  const mediaType = String(input.mime || '').trim()
  return {
    type: 'activity',
    activity: {
      id: globalThis.crypto.randomUUID(),
      payload: {
        activity_type: 'resource',
        kind: attachmentResourceKind(mediaType, input.filename),
        reference: { reference_type: 'workspace_path', path: input.path },
        name: input.filename,
        ...(mediaType ? { media_type: mediaType } : {}),
        ...(typeof input.size === 'number' ? { size_bytes: input.size } : {}),
      },
    },
  }
}

async function send() {
  const sid = chat.selectedSessionId
  let text = draft.value.trim()
  const filesSnapshot = attachedFiles.value.slice()
  const draftSnapshot = draft.value
  if (!sid || (!text && filesSnapshot.length === 0)) return

  if (text && filesSnapshot.length === 0) {
    sending.value = true
    try {
      const effect = await chatCommands.runPluginSlashCommand(text, sid)
      if (effect) {
        if (effect.kind === 'submit_prompt') {
          text = effect.prompt.trim()
          if (!text) {
            draft.value = ''
            return
          }
        } else {
          draft.value = ''
          commandOpen.value = false
          commandQuery.value = ''
          if (effect.kind === 'message' && effect.text.trim()) {
            toasts.push('info', effect.text.trim())
          } else if (effect.kind === 'open_plugin_workbench') {
            await router.push({
              path: '/settings/plugins',
              query: {
                ...route.query,
                plugin: effect.pluginId,
                ...(effect.tab ? { pluginTab: effect.tab } : {}),
              },
            })
          } else if (effect.kind === 'open_url') {
            const url = new URL(effect.url, window.location.href)
            if (url.protocol !== 'http:' && url.protocol !== 'https:') {
              throw new Error('Plugin URLs must use HTTP or HTTPS.')
            }
            window.open(url.toString(), '_blank', 'noopener,noreferrer')
          }
          return
        }
      }
    } catch (err) {
      toasts.push('error', err instanceof Error ? err.message : String(err))
      return
    } finally {
      sending.value = false
    }
  }

  // UX: if the editor is expanded, collapse it on send.
  if (editorFullscreen.value && !editorClosing.value) {
    closeEditorFullscreen()
  }

  sending.value = true
  beginOptimisticSend({
    sessionId: sid,
    text,
    files: filesSnapshot.map((f) => ({ filename: f.filename, mime: f.mime, url: f.url, serverPath: f.serverPath })),
  })

  // UX: clear the composer immediately on send.
  // If the request fails, we restore it in the catch block.
  draft.value = ''
  clearAttachments()
  commandOpen.value = false
  commandQuery.value = ''
  await nextTick()
  scrollToBottom('smooth')
  try {
    const parts: OutgoingMessagePart[] = []
    if (text) parts.push({ type: 'text', text })
    const workspace = filesSnapshot.length > 0 ? await chat.resolveSessionWorkspace(sid) : null
    for (const f of filesSnapshot) {
      let path = ''
      let size = Number.isFinite(f.size) && f.size > 0 ? Math.floor(f.size) : undefined
      const dataUrl = typeof f.url === 'string' ? f.url.trim() : ''
      if (dataUrl) {
        const dataBase64 = attachmentBase64(dataUrl)
        const uploaded = await chat.uploadWorkspaceAttachment(sid, {
          filename: f.filename,
          dataBase64,
          mime: f.mime,
        })
        path = String(uploaded.path || '').trim()
        size = Number.isFinite(uploaded.size_bytes) ? Math.max(0, Math.floor(uploaded.size_bytes)) : size
      } else if (f.serverPath && workspace) {
        path = workspaceRelativePath(f.serverPath, workspace.path)
      }
      if (!path) continue
      parts.push(resourceComposerNode({ path, filename: f.filename, mime: f.mime, size }))
    }

    const runCfg = deriveSendRunConfig({
      selectedProviderId: modelSelection.selectedProviderId.value,
      selectedAdapterId: modelSelection.selectedAdapterId.value,
      selectedModelId: modelSelection.selectedModelId.value,
      selectedThinkingMode: modelSelection.selectedThinkingMode.value,
      selectedSpeedMode: modelSelection.selectedSpeedMode.value,
      effectiveDefaults: modelSelection.effectiveDefaults.value,
    })

    const sendResult = await chat.sendMessage(sid, { ...runCfg, parts })

    // Mark the optimistic message as sent (generation may still be running).
    if (sendResult?.queued) {
      markOptimisticQueued(sid)
    } else {
      markOptimisticSent(sid)
    }
  } catch (e) {
    // Keep UI consistent if send fails.
    clearOnSendFailure()

    // Restore composer content on failure.
    draft.value = draftSnapshot
    attachedFiles.value = filesSnapshot
    throw e
  } finally {
    sending.value = false
  }
}

const lastMessageKey = computed(() => {
  const last = chat.messages[chat.messages.length - 1]
  if (!last) return ''
  const lastPart = last.parts[last.parts.length - 1]
  const part = (lastPart || {}) as { text?: string; content?: string }
  const textLen =
    typeof part.text === 'string'
      ? String(part.text).length
      : typeof part.content === 'string'
        ? String(part.content).length
        : 0
  return `${last.info.id}:${last.parts.length}:${lastPart?.id || ''}:${textLen}`
})

watch(
  () => lastMessageKey.value,
  () => {
    // Preserve user scroll position if they intentionally scrolled up.
    // `scheduleScrollToBottom()` has an additional near-bottom guard to recover
    // from stale bottom flags after background resume.
    if (pendingInitialScrollSessionId.value) return
    scheduleScrollToBottom()
  },
)

const lastHandledSessionActionSeq = ref(0)

watch(
  // Session actions can be requested from the ChatSidebar while ChatPage
  // is unmounted (mobile session switcher). Make the watcher immediate so a
  // pending request is handled on mount.
  () => ui.sessionActionSeq,
  (seq) => {
    if (!seq) return
    if (seq === lastHandledSessionActionSeq.value) return
    const actionId = ui.sessionActionId
    if (!actionId) return
    lastHandledSessionActionSeq.value = seq
    handleSessionActionRequest(actionId)
    ui.clearSessionActionRequest()
  },
  { immediate: true, flush: 'post' },
)

onMounted(async () => {
  // MainLayout already refreshes these, but keep Chat resilient on direct navigation.
  if (!chat.sessions.length) await chat.refreshSessions().catch(() => {})

  const sidFromQuery = readSessionIdFromQuery(route.query) || readSessionIdFromFullPath(route.fullPath)
  if (sidFromQuery) {
    await chat.selectSession(sidFromQuery).catch(() => {})
  }

  const sid = (chat.selectedSessionId || '').trim()
  if (sid && !pendingInitialScrollSessionId.value) {
    requestInitialScroll(sid)
  }

  await modelSelection.loadProvidersAndModels()
  modelSelection.applySessionSelection()
  await loadCommands()
  navIndex.value = Math.max(0, navigableMessageIds.value.length - 1)

  void ensureInitialHistoryScrollable(sid)

  commandPointerHandler = (event: MouseEvent | TouchEvent) => {
    const target = event.target as Node | null
    if (!target) return

    if (composerActionMenuOpen.value) {
      if (sessionActionsMenuRef.value?.containsTarget?.(target)) return
      if (composerActionMenuAnchorRef.value && composerActionMenuAnchorRef.value.contains(target)) return
      closeComposerActionMenu()
    }

    // Keep menus open when interacting within them.
    if (composerPickerRef.value?.containsTarget?.(target)) return
    if (composerControlsRef.value && composerControlsRef.value.contains(target)) return

    // Clicking anywhere else closes picker panels.
    closeComposerPickerMenu()

    // Don't dismiss command suggestions while the user is still interacting
    // with the textarea.
    const textarea = getComposerTextareaEl(composerRef.value)
    if (textarea && textarea.contains(target)) return
    commandOpen.value = false
  }
  document.addEventListener('pointerdown', commandPointerHandler, true)

  chatFocusInHandler = (event: FocusEvent) => {
    const target = event.target as Node | null
    if (!target || !pageRef.value?.contains(target)) return
    const element = target instanceof Element ? target : target.parentElement
    if (!element) return

    if (element.closest('[data-chat-input="true"]')) {
      ui.setGlobalSelection('chat-input', String(chat.selectedSessionId || 'composer').trim() || 'composer', {
        meta: { source: 'chat-composer-focus' },
      })
      return
    }

    const messageId = resolveMessageIdFromTarget(element)
    if (messageId) {
      ui.setGlobalSelection('chat-message', messageId, {
        meta: { source: 'chat-message-focus' },
      })
    }
  }

  chatPointerUpHandler = (event: PointerEvent) => {
    const target = event.target as Node | null
    if (!target || !pageRef.value?.contains(target)) return

    const messageId = resolveMessageIdFromTarget(target)
    if (messageId) {
      ui.setGlobalSelection('chat-message', messageId, {
        meta: { source: 'chat-message-pointerup' },
      })
    }

    if (hasTextSelection()) {
      const sid = String(chat.selectedSessionId || '').trim()
      const token = sid ? `${sid}:${Date.now()}` : `selection:${Date.now()}`
      ui.setGlobalSelection('chat-text', token, {
        meta: {
          source: 'chat-text-selection',
          ...(messageId ? { messageId } : {}),
        },
      })
    }
  }

  document.addEventListener('focusin', chatFocusInHandler, true)
  document.addEventListener('pointerup', chatPointerUpHandler, true)
})

watch(
  () => sessionDirectory.value,
  () => {
    void loadCommands()
    void modelSelection.loadProvidersAndModels()
  },
)

// Template is rendered by ./chat/ChatPageView.vue. Keep this file under 1000 LOC by
// passing a context bag (refs + handlers) to the view.
const viewCtx = {
  // Stores / environment.
  chat,
  ui,

  // Template refs.
  pageRef,
  scrollEl,
  contentEl,
  bottomEl,
  composerBarRef,
  composerRef,
  composerControlsRef,
  composerPickerRef,
  modelTriggerRef,
  thinkingTriggerRef,
  speedTriggerRef,
  sessionActionsMenuRef,

  // Composer + attachments.
  draft,
  attachedFiles,
  attachmentsBusy,
  attachmentsPanelOpen,
  formatBytes,
  handleDrop,
  handlePaste,
  handleDraftInput,
  handleDraftKeydown,
  handleFileInputChange,
  removeAttachment,
  clearAttachments,
  openFilePicker,
  openProjectAttachDialog,
  toggleAttachmentsPanel,
  setAttachmentsPanelOpen,
  closeAttachmentsPanel,
  composerFullscreenActive,
  composerSplitTopCollapsed,
  composerTargetHeight,
  handleComposerResize,
  resetComposerHeight,
  toggleEditorFullscreen,

  // Header.
  sessionEnded,
  canAbort,
  retryStatus,
  retryCountdownLabel,
  retryNextLabel,
  abortRun,

  // Messages.
  renderBlocks,
  pendingInitialScrollSessionId,
  loadingOlder,
  showTimestamps,
  formatTime,
  copiedMessageId,
  revertBusyMessageId,
  isStreamingAssistantMessage,
  showAssistantPlaceholder,
  revertMarkerBusy,
  currentPhase,
  awaitingAssistant,
  optimisticUser,
  showOptimisticUser,
  handleForkFromMessage,
  handleRevertFromMessage,
  handleCopyMessage,
  handleCopySessionError,
  handleRedoFromRevertMarker,
  handleUnrevertFromRevertMarker,

  // Activity rendering.
  activityInitiallyExpandedForPart,
  activityCollapseSignal,
  MAX_VISIBLE_ACTIVITY_COLLAPSED,
  isActivityExpanded,
  setActivityExpanded,
  isReasoningPart,
  isJustificationPart,
  isMetaPart,
  transcriptPartExpanded,
  setTranscriptPartExpanded,

  // TUI-parity transcript navigation and search.
  transcriptSearchInputRef,
  transcriptVimModeLabel,
  transcriptVimCommandLabel,
  transcriptSearchOpen,
  transcriptSearchQuery,
  transcriptSearchSummary,
  selectTranscriptNode,
  isTranscriptNodeActive,
  isTranscriptNodeSelected,
  isTranscriptNodeSearchMatch,
  setTranscriptSearchQuery,
  handleTranscriptSearchKeydown,
  closeTranscriptSearch,

  // Scroll + nav.
  handleScroll,
  isAtBottom,
  navigableMessageIds,
  navBottomOffset,
  navIndex,
  navTotalLabel,
  navPrev,
  navNext,
  scrollToBottom,

  // Composer action menu.
  composerActionMenuOpen,
  composerActionMenuQuery,
  composerActionMenuGroups,
  toggleComposerActionMenu,
  closeComposerActionMenu,
  runComposerActionMenu,

  // Model, thinking mode, and speed mode selection.
  composerPickerTitle,
  composerPickerSearchable,
  composerPickerSearchPlaceholder,
  composerPickerQuery,
  setComposerPickerQuery,
  composerPickerHelperText,
  composerPickerEmptyText,
  composerPickerGroups,
  composerPickerLoading,
  composerPickerRefreshable,
  refreshComposerPickerOptions,
  setComposerPickerOpen,
  handleComposerPickerSelect,
  ...modelSelection,

  // Send/stop.
  sessionUsage,
  formatCompactNumber,
  showComposerStopAction,
  composerStopDisabled,
  composerPrimaryDisabled,
  handleComposerPrimaryAction,
  handleComposerStopAction,
  aborting,
  sending,

  // Dialogs.
  renameDialogOpen,
  renameDraft,
  renameBusy,
  saveRename,
  attachProjectDialogOpen,
  attachProjectPath,
  sessionDirectory,
  sessionTitle,
  addProjectAttachment,
} satisfies ChatPageViewContext

onBeforeUnmount(() => {
  if (commandPointerHandler) {
    document.removeEventListener('pointerdown', commandPointerHandler, true)
    commandPointerHandler = null
  }
  if (chatFocusInHandler) {
    document.removeEventListener('focusin', chatFocusInHandler, true)
    chatFocusInHandler = null
  }
  if (chatPointerUpHandler) {
    document.removeEventListener('pointerup', chatPointerUpHandler, true)
    chatPointerUpHandler = null
  }
})
</script>

<template>
  <ChatPageView :ctx="viewCtx" />
  <PlanViewerDialog v-model:open="planViewerOpen" :session-id="chat.selectedSessionId" />
</template>
