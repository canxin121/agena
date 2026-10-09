<script setup lang="ts">
import { createFailedAttachmentDraftSlot } from './chat/failedAttachmentDrafts'
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch, type Component } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { RiScissorsLine } from '@remixicon/vue'

import ChatPageView from './chat/ChatPageView.vue'
import type { ChatPageViewContext } from './chat/chatPageViewContext'

import { copyTextToClipboard } from '@/lib/clipboard'
import { apiJson } from '@/lib/api'
import { promptForText } from '@/lib/appTextPrompt'
import { readSessionIdFromFullPath, readSessionIdFromQuery } from '@/app/navigation/sessionQuery'
import { useChatStore } from '@/stores/chat'
import { useBtwStore } from '@/stores/btw'
import * as chatApi from '@/stores/chat/api'
import { useDirectoryStore } from '@/stores/directory'
import { useDirectorySessionStore } from '@/stores/directorySessionStore'
import { useSessionActivityStore } from '@/stores/sessionActivity'
import { useSettingsStore } from '@/stores/settings'
import { useUiStore } from '@/stores/ui'
import { useToastsStore } from '@/stores/toasts'

import { useMessageStreaming } from '@/composables/chat/useMessageStreaming'
import { useChatAttachments, type AttachedFile } from './chat/useChatAttachments'
import { getComposerInput, type ComposerExpose, type ComposerInput, type ComposerSegment } from './chat/composerInput'
import { useChatScrollNav } from './chat/useChatScrollNav'
import { useChatComposerLayout } from './chat/useChatComposerLayout'
import { useChatModelSelection } from './chat/useChatModelSelection'
import { useChatCommands, matchSlashCommand } from './chat/useChatCommands'
import type { BuiltInCommand, Command } from './chat/useChatCommands'
import { commandUsage } from './chat/chatCommandsCatalog'
import { useChatSessionActions } from './chat/useChatSessionActions'
import { useChatRunUi } from './chat/useChatRunUi'
import { useChatTranscriptVim } from './chat/useChatTranscriptVim'
import { resolveTranscriptVimEnabled } from './chat/transcriptVimPreference'
import {
  composerLineEnd,
  composerLineStart,
  composerWordRangeAfter,
  composerWordRangeBefore,
  nextComposerGraphemeBoundary,
  nextComposerWordBoundary,
  previousComposerGraphemeBoundary,
  previousComposerWordBoundary,
} from './chat/composerWordNavigation'
import { useComposerPromptHistory } from './chat/composerPromptHistory'
import { useSessionPlan } from './chat/useSessionPlan'
import { conditionalJsonObserved } from '@/lib/conditionalJson'
import { invalidateResources } from '@/lib/resourceSync'
import { buildPlanToolInvocationRequest } from './chat/planViewerRequest'
import { openComposerInputMenu } from './chat/composerInputMenus'
import { formatTimeHM } from '@/i18n/intl'
import { useChatRenderBlocks } from './chat/useChatRenderBlocks'
import { useChatMessageActions } from './chat/useChatMessageActions'
import { activityInitiallyExpandedForPart as resolveActivityInitiallyExpanded } from './chat/transcriptExpansion'
import { isAssistantMessageStreaming } from '@/lib/chatRunState'
import { deriveSendRunConfig } from './chat/modelSendDefaults'
import { useWorkspacePaneContext } from '@/app/workspace/workspacePaneContext'
import type { OptionMenuGroup, OptionMenuItem } from '@/components/ui/optionMenu.types'
import type { TranscriptDisplayPart } from '@/components/chat/messageList.types'
import type { MessageFold } from '@/types/chat'
import type { JsonObject, JsonValue } from '@/types/json'
import type { PluginCommandResult } from '@/lib/pluginOperations'
import { useTranscriptPreferencesStore } from '@/stores/transcriptPreferences'

type ComposerActionItem = { id: string; label: string; description?: string; icon?: Component; disabled?: boolean }

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
        provenance?: { content_hash: string }
        payload: {
          activity_type: 'resource'
          delivery?: 'reference' | 'model_input'
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
const btw = useBtwStore()
const workspacePane = useWorkspacePaneContext()
const isVisibleWorkspacePane = computed(() => !workspacePane || workspacePane.isVisible.value)
watch(
  () => [chat.selectedSessionId, isVisibleWorkspacePane.value] as const,
  ([sid, visible], _previous, onCleanup) => {
    if (sid && visible) onCleanup(chat.retainSession(sid))
  },
  { immediate: true },
)

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
const failedDraftSlot = createFailedAttachmentDraftSlot()
const failedDraftVersion = ref(0)
const failedAttachmentDraft = computed(() => {
  void failedDraftVersion.value
  return failedDraftSlot.peek()
})
function restoreFailedAttachmentDraft() {
  const sid = chat.selectedSessionId
  if (!sid) return
  const restored = failedDraftSlot.take(sid, draft.value, attachedFiles.value)
  if (!restored) {
    toasts.push('info', String(t('chat.attachments.recoveryNeedsEmptyDraft')))
    return
  }
  clearAttachments()
  draft.value = restored.text
  attachedFiles.value = restored.files.map((file) => ({ ...file, state: 'ready' as const }))
  failedDraftVersion.value++
  if (restored.segments) void nextTick(() => composerRef.value?.restoreSegments?.(restored.segments!))
}
function discardFailedAttachmentDraft() {
  failedDraftSlot.discard()
  failedDraftVersion.value++
}

const composerRef = ref<ComposerExpose | null>(null)
const submittingAttachmentUrls = new Set<string>()
const attachments = useChatAttachments({
  toasts,
  composerRef,
  restoreText: (text) => {
    draft.value += text
  },
  retainedUrls: () => [
    ...submittingAttachmentUrls,
    ...[...attachmentDraftsBySession.values()].flatMap((files) =>
      files.flatMap((file) => (file.url ? [file.url] : [])),
    ),
    ...(failedDraftSlot.peek()?.files.flatMap((file) => (file.url ? [file.url] : [])) ?? []),
    ...(optimisticUser.value?.files.flatMap((file) => (file.url ? [file.url] : [])) ?? []),
    ...(ui.isImageViewerOpen ? ui.imageViewerItems.map((item) => item.src) : []),
  ],
})
const {
  attachedFiles,
  pendingAttachments,
  attachmentsBusy,
  waitForAttachments,
  preparationErrorCount,
  attachmentGeneration,
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
const attachmentDraftsBySession = new Map<string, AttachedFile[]>()
watch(
  () => chat.selectedSessionId,
  (sid, previousSid) => {
    const previousKey = String(previousSid || '')
    if (attachedFiles.value.length) {
      attachmentDraftsBySession.set(
        previousKey,
        attachedFiles.value.map((file) => ({ ...file })),
      )
    } else {
      attachmentDraftsBySession.delete(previousKey)
    }
    clearAttachments({ preserve: true })
    attachedFiles.value = (attachmentDraftsBySession.get(String(sid || '')) || []).map((file) => ({ ...file }))
    attachmentDraftsBySession.delete(String(sid || ''))
  },
  { flush: 'sync' },
)

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
const sideBusy = ref(false)
const sideParentId = computed(() => {
  const context = asRecord(chat.getSessionExecution(chat.selectedSessionId)?.conversation)
  const parent = Number(context.parent_session_id)
  return context.mode === 'side' && Number.isSafeInteger(parent) && parent > 0 ? String(parent) : null
})
function openBtw(question = '') {
  const sessionId = chat.selectedSessionId
  if (!sessionId) return
  btw.open(sessionId, question)
  void nextTick(() => {
    if (chat.selectedSessionId === sessionId) scrollToBottom('smooth')
  })
}
async function openSide(question = '') {
  const parent = chat.selectedSessionId
  if (!parent || sideBusy.value) return
  sideBusy.value = true
  try {
    const created = await chat.forkSession(parent, { conversation_mode: 'side' })
    if (!created?.id) throw new Error('The server did not return a side conversation')
    // A slow fork must not pull the user out of a session they navigated to.
    if (chat.selectedSessionId === parent) await chat.selectSession(created.id)
    if (question.trim()) await chat.sendMessage(created.id, { text: question.trim() })
  } catch (error) {
    toasts.push('error', error instanceof Error ? error.message : String(error))
  } finally {
    sideBusy.value = false
  }
}
function returnFromSide() {
  if (sideParentId.value) void chat.selectSession(sideParentId.value)
}

const modelTriggerRef = ref<HTMLElement | null>(null)
const thinkingTriggerRef = ref<HTMLElement | null>(null)
const speedTriggerRef = ref<HTMLElement | null>(null)

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

function asRecord(value: JsonValue): JsonObject {
  return typeof value === 'object' && value !== null ? (value as JsonObject) : {}
}

function getRecord(value: JsonValue, key: string): JsonObject {
  const root = asRecord(value)
  const nested = root[key]
  return typeof nested === 'object' && nested !== null ? (nested as JsonObject) : {}
}

const chatCommands = useChatCommands({
  draft,
  composerRef,
  composerPickerOpen,
  onSend: send,
  onCommandSelected: handleCommandSelected,
})

const {
  commands,
  filteredCommands,
  commandOpen,
  commandQuery,
  commandIndex,
  commandFocusSearch,
  loadCommands,
  openCommandPalette: openCommandPaletteBase,
  setCommandQuery: setCommandQueryBase,
  closeCommandPalette,
  selectCommand,
  handleCommandPaletteKeydown,
  handleDraftInput: handleDraftInputBase,
  handleDraftKeydown: handleDraftKeydownInner,
} = chatCommands

const promptHistory = useComposerPromptHistory()
const {
  filteredEntries: filteredPromptHistoryEntries,
  open: promptHistoryOpen,
  query: promptHistoryQuery,
  activeIndex: promptHistoryIndex,
  focus: promptHistoryFocus,
  openHistory: openPromptHistory,
  closeHistory: closePromptHistory,
  updateQuery: updatePromptHistoryQuery,
  focusInput: focusPromptHistoryInput,
  focusResults: focusPromptHistoryResults,
  moveOlder: movePromptHistoryOlder,
  moveNewer: movePromptHistoryNewer,
  accept: acceptPromptHistory,
  record: recordPromptHistory,
} = promptHistory

function openCommandPalette(query = '', options: { focusSearch?: boolean } = {}) {
  closePromptHistory()
  openCommandPaletteBase(query, options)
}

function setCommandQuery(value: string) {
  setCommandQueryBase(value)
}

function handleDraftInput() {
  handleDraftInputBase()
  ui.setGlobalSelection('chat-input', chat.selectedSessionId || 'composer', {
    meta: { source: 'chat-composer-input' },
  })
}

modelSelection = useChatModelSelection({
  chat,
  composerPickerOpen,
  modelPickerQuery,
  onOpenComposerPicker: () => {
    closePromptHistory()
    openComposerInputMenu('picker', {
      closeAttachments: closeAttachmentsPanel,
      closeActions: closeComposerActionMenu,
      closePicker: closeComposerPickerMenu,
    })
  },
  commandOpen,
  commandFocusSearch,
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
            checked: modelSelection.modelSource.value === 'default',
            keywords: 'default runtime model',
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

function applyPromptHistoryText(text: string | null) {
  if (!text) return
  // TUI restores a history entry as a fresh plain-text composer document and
  // places the cursor at its end. Do not run slash-command input handling here:
  // recalling a prompt must not unexpectedly open another palette.
  draft.value = text
  ui.setGlobalSelection('chat-input', chat.selectedSessionId || 'composer', {
    meta: { source: 'chat-prompt-history-recall' },
  })
  setComposerCaret(text.length)
}

function selectPromptHistoryEntry(entry?: string) {
  if (entry !== undefined) {
    const index = filteredPromptHistoryEntries.value.indexOf(entry)
    if (index >= 0) promptHistoryIndex.value = index
  }
  applyPromptHistoryText(acceptPromptHistory())
}

function tryOpenPromptHistory(): boolean {
  if (attachedFiles.value.length > 0) {
    toasts.push('info', String(t('chat.composer.promptHistory.itemsHint')))
    return true
  }
  if (!openPromptHistory()) {
    toasts.push('info', String(t('chat.composer.promptHistory.empty')))
    return true
  }
  closeCommandPalette()
  closeComposerPickerMenu()
  closeComposerActionMenu()
  closeAttachmentsPanel()
  return true
}

function handlePromptHistoryKeydown(event: KeyboardEvent): boolean {
  if (!promptHistoryOpen.value) return false

  const plain = !event.ctrlKey && !event.altKey && !event.metaKey && !event.shiftKey
  if (event.key === 'Escape' && plain) {
    event.preventDefault()
    closePromptHistory()
    return true
  }
  if (event.key.toLowerCase() === 'c' && event.ctrlKey && !event.altKey && !event.metaKey && !event.shiftKey) {
    event.preventDefault()
    closePromptHistory()
    return true
  }
  if (event.key === 'Enter') {
    if (plain) {
      event.preventDefault()
      selectPromptHistoryEntry()
    } else {
      // The history search owns Enter while it is open; it must never submit
      // the underlying composer through Ctrl/Cmd+Enter.
      event.preventDefault()
    }
    return true
  }
  if (event.key === 'ArrowDown' && plain) {
    event.preventDefault()
    if (promptHistoryFocus.value === 'input') focusPromptHistoryResults()
    else movePromptHistoryOlder()
    return true
  }
  if (event.key === 'ArrowUp' && plain) {
    event.preventDefault()
    if (promptHistoryFocus.value === 'results') movePromptHistoryNewer()
    else focusPromptHistoryInput()
    return true
  }
  if (event.key.toLowerCase() === 'r' && event.ctrlKey && !event.altKey && !event.metaKey && !event.shiftKey) {
    event.preventDefault()
    movePromptHistoryOlder()
    return true
  }
  if (event.key.toLowerCase() === 's' && event.ctrlKey && !event.altKey && !event.metaKey && !event.shiftKey) {
    event.preventDefault()
    movePromptHistoryNewer(true)
    return true
  }

  // SearchPicker returns to its editor as soon as a non-navigation key is
  // pressed while the result list has focus. The Web search input stays the
  // DOM focus target, so only the logical focus state needs changing here.
  if (promptHistoryFocus.value === 'results') focusPromptHistoryInput()
  return true
}

async function setAttachmentsPanelOpen(next: boolean) {
  if (!next) {
    closeAttachmentsPanel()
    return
  }
  closePromptHistory()
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
  handleScroll,
  handleWheel,
  loadOlderAndPreserveViewport,
  navigableMessageIds,
  navIndex,
  navBottomOffset,
  navTotalLabel,
  navCurrentOrdinal,
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
  composerMaxHeight,
  composerSplitTopCollapsed,
  handleComposerResize,
  toggleEditorFullscreen,
  closeEditorFullscreen,
  applyComposerUserHeight,
  resetComposerHeight,
} = composerLayout

function composerTextarea(): ComposerInput | null {
  return getComposerInput(composerRef.value)
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
  const textarea = composerTextarea()
  const value = textarea?.value ?? ''
  if (start < 0 || end < start || end > value.length) return
  textarea?.setRangeText(replacement, start, end, 'end')
  setComposerCaret(cursor)
}

function handleComposerCursorKeydown(event: KeyboardEvent): boolean {
  const textarea = composerTextarea()
  if (!textarea || event.shiftKey || event.ctrlKey || event.altKey || event.metaKey) return false

  const cursor = textarea.selectionStart ?? 0
  const selectionEnd = textarea.selectionEnd ?? cursor
  let target: number | null = null

  if (event.key === 'ArrowLeft') {
    target =
      cursor === selectionEnd
        ? previousComposerGraphemeBoundary(textarea.value, cursor)
        : Math.min(cursor, selectionEnd)
  } else if (event.key === 'ArrowRight') {
    target =
      cursor === selectionEnd ? nextComposerGraphemeBoundary(textarea.value, cursor) : Math.max(cursor, selectionEnd)
  } else if (event.key === 'Home') {
    target = composerLineStart(textarea.value, cursor)
  } else if (event.key === 'End') {
    target = composerLineEnd(textarea.value, cursor)
  }

  if (target === null) return false
  event.preventDefault()
  setComposerCaret(target)
  return true
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
  if (promptHistoryOpen.value) {
    handlePromptHistoryKeydown(e)
    return
  }
  if (composerFullscreenActive.value && e.key === 'Escape' && !commandOpen.value) {
    e.preventDefault()
    closeEditorFullscreen()
    return
  }
  const textarea = composerTextarea()
  if (
    !commandOpen.value &&
    e.key === 'ArrowUp' &&
    !e.ctrlKey &&
    !e.altKey &&
    !e.metaKey &&
    !e.shiftKey &&
    !e.isComposing &&
    textarea &&
    textarea.selectionStart === 0 &&
    textarea.selectionEnd === 0
  ) {
    tryOpenPromptHistory()
    e.preventDefault()
    return
  }
  if (handleComposerWordKeydown(e)) return
  if (handleComposerCursorKeydown(e)) return
  handleDraftKeydownInner(e)
}

const settingsData = computed<JsonObject>(() => asRecord(settings.data))
const transcriptPreferences = useTranscriptPreferencesStore()

const activityAutoCollapseOnIdle = computed(() => settingsData.value.chatActivityAutoCollapseOnIdle !== false)

function activityInitiallyExpandedForPart(part: TranscriptDisplayPart): boolean {
  return resolveActivityInitiallyExpanded(part, transcriptPreferences)
}

const showThinking = computed(() => settingsData.value.showReasoningTraces !== false)
const showTimestamps = computed(() => settingsData.value.showChatTimestamps !== false)

const renderBlocksApi = useChatRenderBlocks({
  chat,
  settings,
  showThinking,
  formatTime,
  // Row vocabulary follows the UI language; a runtime presentation title on
  // the part itself still wins over this fallback.
  labels: () => ({ attachment: String(t('chat.attachments.rowTitle')) }),
})

const {
  renderBlocks,
  getTextParts,
  isReasoningPart,
  isMetaPart,
  activityExpandedByBlockKey,
  activityCollapseSignal,
  collapseAllActivities,
  isActivityExpanded,
  setActivityExpanded,
} = renderBlocksApi

function transcriptPartExpanded(part: TranscriptDisplayPart): boolean {
  return renderBlocksApi.transcriptPartExpanded(part, activityInitiallyExpandedForPart(part))
}

function setTranscriptPartExpanded(part: TranscriptDisplayPart, expanded: boolean) {
  setActivityExpanded(part.key, expanded)
}

function loadFoldedActivity(fold: MessageFold, all: boolean) {
  const sid = chat.selectedSessionId
  if (!sid) return
  void chat.loadFoldedActivity(sid, fold, all, chat.transcriptPartPageSize).catch(() => {})
}

function setTranscriptPartPageSize(size: number) {
  chat.setTranscriptPartPageSize(size)
}

const sessionActions = useChatSessionActions({
  chat,
  toasts,
  sessionTitle,
  showThinking,
  copyToClipboard,
  onSessionForked: (newId) => chat.selectSession(newId),
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
})

const {
  awaitingAssistant,
  pendingSendAt,
  showOptimisticUser,
  resetForSessionSwitch,
  beginOptimisticSend,
  markOptimisticSent,
  clearOnSendFailure,
  clearOnCancellation,
} = stream

// vue-tsc's template narrowing can be finicky around `Ref<T | null>` even when
// the runtime checks are correct. Keep this relaxed for now.
const optimisticUser = stream.optimisticUser
watch([optimisticUser, failedDraftVersion, () => ui.isImageViewerOpen], attachments.releaseUnusedAttachmentUrls, {
  flush: 'post',
})

function restoredComposerText(document: JsonValue): string {
  if (!Array.isArray(document)) return ''
  return document
    .map((node) => {
      const record = asRecord(node)
      return record.type === 'text' && typeof record.text === 'string' ? record.text : ''
    })
    .join('')
}

function restoredComposerFiles(document: JsonValue): AttachedFile[] {
  if (!Array.isArray(document)) return []
  return document.flatMap((node, index) => {
    const record = asRecord(node)
    if (record.type !== 'activity') return []
    const activity = asRecord(record.activity)
    const payload = asRecord(activity.payload)
    if (payload.activity_type !== 'resource') return []
    const reference = asRecord(payload.reference)
    const path = typeof reference.path === 'string' ? reference.path.trim() : ''
    if (!path) return []
    const filename =
      typeof payload.name === 'string' && payload.name.trim() ? payload.name.trim() : path.split('/').pop() || 'file'
    const size =
      typeof payload.size_bytes === 'number' && Number.isFinite(payload.size_bytes)
        ? Math.max(0, payload.size_bytes)
        : 0
    const id = typeof activity.id === 'string' && activity.id.trim() ? activity.id : `restored-${Date.now()}-${index}`
    return [
      {
        id,
        filename,
        size,
        mime: typeof payload.media_type === 'string' ? payload.media_type : 'application/octet-stream',
        serverPath: path,
        delivery: payload.delivery === 'model_input' ? ('model_input' as const) : ('reference' as const),
        state: 'ready' as const,
      },
    ]
  })
}

function restoreCancelledComposer(outcome: chatApi.CancellationOutcome) {
  const optimistic = optimisticUser.value
  clearOnCancellation()
  const document = outcome.restored_user_message
  if (document == null) return

  // Do not overwrite text or attachments entered while cancellation was in
  // flight. The server result remains authoritative for the transcript, but
  // the user's newer local draft takes precedence in the editor.
  if (String(draft.value || '').length > 0 || attachedFiles.value.length > 0) return

  const text = optimistic?.text ?? restoredComposerText(document)
  const files =
    optimistic?.files?.map((file, index) => ({
      id: file.id || `restored-${Date.now()}-${index}`,
      filename: file.filename,
      size: typeof file.size === 'number' ? file.size : 0,
      mime: file.mime,
      ...(file.url ? { url: file.url } : {}),
      ...(file.serverPath ? { serverPath: file.serverPath } : {}),
      delivery: file.delivery || 'reference',
      state: 'ready' as const,
    })) ?? restoredComposerFiles(document)
  draft.value = text
  attachedFiles.value = files
}

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
  closePromptHistory()
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

async function copyToClipboard(text: string | Promise<string>) {
  const ok = await copyTextToClipboard(text)
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
  message:
    | {
        info?: { role?: string; runState?: string; finish?: string; error?: unknown }
      }
    | null
    | undefined,
): boolean {
  return isAssistantMessageStreaming(message?.info)
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
  pendingAttachments,
  sending,
  awaitingAssistant,
  pendingSendAt,
  renderBlocks,
  onSend: send,
  onCancellation: restoreCancelledComposer,
  collapseAllActivities,
  activityAutoCollapseOnIdle,
})

const {
  currentPhase,
  retryStatus,
  retryCountdownLabel,
  retryNextLabel,
  sessionUsage,
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

// Vim mode is a browser preference: desktop defaults to on, mobile to off. The
// focused-pane gate stays on top so embedded panes keep one keyboard owner.
const transcriptVimEnabled = computed(() =>
  resolveTranscriptVimEnabled(settings.data?.chatTranscriptVim, ui.isMobileDevice),
)

const transcriptVim = useChatTranscriptVim({
  enabled: computed(() => transcriptVimEnabled.value && isFocusedWorkspacePane.value),
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
    openPlanViewer()
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
  isNodeSelected: isTranscriptNodeSelected,
  isNodeSearchMatch: isTranscriptNodeSearchMatch,
  setSearchQuery: setTranscriptSearchQuery,
  handleSearchKeydown: handleTranscriptSearchKeydown,
  openSearch: openTranscriptSearch,
  jumpSearch: jumpTranscriptSearch,
  closeSearch: closeTranscriptSearch,
} = transcriptVim

// A single session-owned read feeds both the inline document and progress chip.
const {
  viewer: planViewer,
  expanded: planViewerOpen,
  visible: planVisible,
} = useSessionPlan(
  () => chat.selectedSessionId,
  () => currentPhase.value !== 'idle',
  () => [chat.messages.length, currentPhase.value],
  async (sessionId, tool, input, signal, read) => {
    const body = buildPlanToolInvocationRequest(sessionId, tool, input)
    if (!body) throw new Error(String(t('chat.planViewer.requiresSession')))
    const resource = `session:${body.session_id}:plan`
    if (tool === 'get') {
      const { value, observation } = await conditionalJsonObserved<Record<string, JsonValue>>(
        resource,
        `/api/v1/sessions/${body.session_id}/plan`,
        { signal },
        read?.force,
      )
      read?.observe(observation)
      return value
    }
    const result = await apiJson<Record<string, JsonValue>>('/api/v1/plugins/tools/invoke', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(body),
      signal,
    })
    invalidateResources([resource])
    return result
  },
)

function openPlanViewer() {
  const sessionId = chat.selectedSessionId
  if (!sessionId) return
  planViewerOpen.value = true
  void nextTick(() => {
    if (chat.selectedSessionId === sessionId) scrollToBottom('smooth')
  })
}

function formatBackgroundActivitySummary(kinds: string[]): string {
  const normalized = Array.isArray(kinds)
    ? kinds.map((kind) =>
        String(kind || '')
          .trim()
          .toLowerCase(),
      )
    : []
  return ['monitor', 'cron', 'shell', 'task', 'runtime', 'browser']
    .map((kind) => {
      const count = normalized.filter((candidate) => candidate === kind).length
      return count > 0 ? `${kind} ${count}` : ''
    })
    .filter(Boolean)
    .join(' · ')
}

const composerStatusExtra = computed(() => {
  const parts: string[] = []
  if (chat.messagesLoading) parts.push(String(t('chat.composer.status.loading')))
  if (commandOpen.value) parts.push(`${String(t('chat.composer.status.slash'))} /${commandQuery.value}`)
  return parts.join('  |  ')
})

const composerBottomLeftStatus = computed(() => {
  const sid = commandSessionId()
  if (!sid) return ''
  // Prefer the server's session-scoped projection. The client-side activity
  // snapshot is only a fallback until the first status refresh lands.
  const serverKinds = chat.sessionBackgroundActivityKinds(sid)
  if (serverKinds) return formatBackgroundActivitySummary(serverKinds)
  return formatBackgroundActivitySummary(activity.snapshot[sid]?.kinds || [])
})

const composerBottomRightStatus = computed(() => planViewer.snapshot.value?.progress || '')

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
    // New sessions remain unassigned until the user selects a model.
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
  sha256?: string
  delivery?: 'reference' | 'model_input'
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
      ...(input.sha256 ? { provenance: { content_hash: input.sha256 } } : {}),
      payload: {
        activity_type: 'resource',
        delivery: input.delivery || 'reference',
        kind: attachmentResourceKind(mediaType, input.filename),
        reference: { reference_type: 'workspace_path', path: input.path },
        name: input.filename,
        ...(mediaType ? { media_type: mediaType } : {}),
        ...(typeof input.size === 'number' ? { size_bytes: input.size } : {}),
      },
    },
  }
}

function commandSessionId(): string | null {
  const sid = String(chat.selectedSessionId || '').trim()
  return sid || null
}

function commandRequestId(): string {
  const attention = chat.selectedAttention
  const payload = asRecord(attention?.payload as JsonValue)
  const properties = getRecord(payload, 'properties')
  return typeof properties.id === 'string' ? properties.id.trim() : ''
}

/**
 * A command that documents no usage takes no arguments, so anything typed
 * after it is a mistake worth reporting rather than silently dropping.
 */
function commandHasUnexpectedArguments(command: BuiltInCommand, args: string): boolean {
  return !command.usage && Boolean(args.trim())
}

function parsePullRequestArguments(raw: string): {
  title: string
  body?: string
  base?: string
  head?: string
} | null {
  const input = String(raw || '').trim()
  if (!input) return null
  const optionPattern = /\s+--(body|base|head)\s+/g
  const first = input.search(optionPattern)
  const title = (first < 0 ? input : input.slice(0, first)).trim()
  if (!title) return null

  const output: { title: string; body?: string; base?: string; head?: string } = { title }
  const options = first < 0 ? '' : input.slice(first).trim()
  const matcher = /--(body|base|head)\s+(.+?)(?=\s+--(?:body|base|head)\s+|$)/g
  let match: RegExpExecArray | null
  while ((match = matcher.exec(options))) {
    const key = match[1] as 'body' | 'base' | 'head'
    const value = String(match[2] || '').trim()
    if (!value) return null
    output[key] = value
  }
  // Reject unknown flags and malformed options instead of creating a PR with
  // silently discarded input.
  const rest = options.replace(matcher, '').trim()
  if (rest) return null
  return output
}

async function downloadWorkspaceFile(pathInput: string) {
  const sid = commandSessionId()
  if (!sid) throw new Error('Open a session before downloading a workspace file.')
  const workspace = await chat.resolveSessionWorkspace(sid)
  const path = workspaceRelativePath(pathInput, workspace.path)
  const response = await fetch(
    `/api/v1/workspaces/${encodeURIComponent(String(workspace.id))}/download?path=${encodeURIComponent(path)}`,
  )
  if (!response.ok) throw new Error(`Download failed (${response.status})`)
  const blob = await response.blob()
  const filename = path.split('/').filter(Boolean).pop() || 'workspace-file'
  const url = URL.createObjectURL(blob)
  const link = document.createElement('a')
  link.href = url
  link.download = filename
  document.body.appendChild(link)
  link.click()
  link.remove()
  URL.revokeObjectURL(url)
}

async function submitPromptFromCommand(prompt: string) {
  const value = String(prompt || '').trim()
  if (!value) return
  draft.value = value
  await send()
}

async function executeBuiltInCommand(command: BuiltInCommand, rawArgs = ''): Promise<void> {
  const args = String(rawArgs || '').trim()
  if (commandHasUnexpectedArguments(command, args)) {
    toasts.push('error', `Usage: ${commandUsage(command)}`)
    return
  }

  const sid = commandSessionId()
  // Dispatch on the action the catalog declared, not on the slash spelling:
  // the server owns the spelling and may rename it without a client release.
  switch (command.action) {
    case 'help':
      ui.toggleHelpDialog()
      return
    case 'commands':
      openCommandPalette('')
      return
    case 'new':
      await chat.createSession()
      return
    case 'sessions':
      ui.setSessionSwitcherOpen(true)
      return
    case 'rewind': {
      if (!sid) {
        toasts.push('error', 'A session is required for /rewind.')
        return
      }
      const requested =
        args ||
        (await promptForText({
          title: String(t('chat.commandPalette.rewindPromptTitle')),
          placeholder: String(t('chat.commandPalette.rewindPromptPlaceholder')),
        })) ||
        ''
      if (!requested) return
      await handleRevertFromMessage(requested)
      return
    }
    case 'rename':
      openRenameDialog()
      return
    case 'timeline':
      await router.push({ path: '/settings/diagnostics', query: { view: 'activities' } })
      return
    case 'settings':
      await router.push('/settings/interface')
      return
    case 'model':
      await modelSelection.toggleComposerPicker('model')
      return
    case 'commit': {
      if (!args) {
        toasts.push('error', `Usage: ${commandUsage(command)}`)
        return
      }
      if (!sid) {
        toasts.push('error', 'A session is required for /commit.')
        return
      }
      const workspace = await chat.resolveSessionWorkspace(sid)
      const result = await apiJson<JsonValue>('/api/v1/git/commits', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ workspace_id: workspace.id, message: args }),
      })
      const record = asRecord(result)
      const commit = typeof record.commit === 'string' ? record.commit.slice(0, 12) : ''
      toasts.push('success', commit ? `Created commit ${commit}.` : 'Commit created.')
      return
    }
    case 'pr': {
      const parsed = parsePullRequestArguments(args)
      if (!parsed) {
        toasts.push('error', `Usage: ${commandUsage(command)}`)
        return
      }
      if (!sid) {
        toasts.push('error', 'A session is required for /pr.')
        return
      }
      const workspace = await chat.resolveSessionWorkspace(sid)
      const result = await apiJson<JsonValue>('/api/v1/git/pull-requests', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ workspace_id: workspace.id, ...parsed }),
      })
      const url = String(asRecord(result).url || '').trim()
      if (url) window.open(url, '_blank', 'noopener,noreferrer')
      toasts.push('success', url ? `Pull request created: ${url}` : 'Pull request created.')
      return
    }
    case 'export':
      await exportTranscript()
      return
    case 'pager':
      toasts.push('info', 'The web transcript already supports paging through the scroll view.')
      return
    case 'continue':
      if (!sid) {
        toasts.push('error', 'A session is required for /continue.')
        return
      }
      await chat.continueSession(sid)
      toasts.push('success', 'Session continuation started.')
      return
    case 'compact':
      await handleCompactSession()
      return
    case 'user-input':
      if (chat.selectedAttention?.kind === 'question') scrollToBottom('smooth')
      else toasts.push('info', 'There is no pending user-input request.')
      return
    case 'allow':
    case 'allow-always':
    case 'deny':
    case 'deny-always': {
      if (!sid || chat.selectedAttention?.kind !== 'permission') {
        toasts.push('info', 'There is no pending permission request.')
        return
      }
      const requestId = commandRequestId()
      if (!requestId) throw new Error('The pending permission request has no id.')
      const reply =
        command.action === 'allow'
          ? 'once'
          : command.action === 'allow-always'
            ? 'always'
            : command.action === 'deny-always'
              ? 'reject_always'
              : 'reject'
      await chat.replyPermission(sid, requestId, reply)
      return
    }
    case 'attach':
    case 'image':
      openFilePicker()
      return
    case 'paste':
      toasts.push('info', 'Use your system paste shortcut to insert clipboard text.')
      return
    case 'favorite':
    case 'hub':
    case 'lineage':
      toasts.push('info', `/${command.name} is available in the terminal client.`)
      return
    case 'download':
      if (!args) {
        toasts.push('error', `Usage: ${commandUsage(command)}`)
        return
      }
      await downloadWorkspaceFile(args)
      return
    case 'edit':
      toggleEditorFullscreen()
      return
    case 'copy':
      await copyTranscript()
      return
    case 'copy-message': {
      const lastAssistant = [...chat.messages].reverse().find((message) => message.info?.role === 'assistant')
      if (lastAssistant) handleCopyMessage(lastAssistant)
      else toasts.push('info', 'No assistant message is loaded.')
      return
    }
    case 'copy-visible': {
      const visible = String(contentEl.value?.innerText || '').trim()
      if (!visible) toasts.push('info', 'No transcript content is visible.')
      else await copyToClipboard(visible)
      return
    }
    case 'fork':
      await handleForkSession()
      return
    case 'side':
      await openSide(args)
      return
    case 'btw':
      openBtw(args)
      return
    case 'children': {
      if (!sid) {
        toasts.push('error', 'A session is required for /children.')
        return
      }
      const parentId = Number(sid)
      const page = await chatApi.listSessions({ parentId, limit: 100, excludeSubagents: true })
      if (page.sessions.length === 1) await chat.selectSession(page.sessions[0]!.id)
      else if (page.sessions.length > 1) {
        ui.setSessionSwitcherOpen(true)
        toasts.push('info', `${page.sessions.length} child sessions are available in the session switcher.`)
      } else toasts.push('info', 'This session has no child sessions.')
      return
    }
    case 'parent': {
      if (!sid) {
        toasts.push('error', 'A session is required for /parent.')
        return
      }
      const parent = Number(asRecord(chat.selectedSession).parent_id)
      if (!Number.isSafeInteger(parent) || parent <= 0) {
        toasts.push('info', 'This session has no parent session.')
        return
      }
      await chat.selectSession(String(parent))
      return
    }
    case 'diagnostics':
      toasts.push('info', `Session ${sid || 'none'} state: ${chat.selectedSessionState.kind}.`)
      return
    case 'status': {
      const usage = sessionUsage.value
      const tokens = usage ? ` · ${usage.percentUsed !== null ? `${usage.percentUsed}%` : usage.tokensLabel}` : ''
      toasts.push('info', `Session ${sid || 'none'} · ${chat.selectedSessionState.kind}${tokens}`)
      return
    }
    case 'usage':
      await router.push({ path: '/settings/diagnostics', query: { view: 'usage' } })
      return
    case 'activities':
    case 'background':
      await router.push({ path: '/settings/diagnostics', query: { view: 'activities' } })
      return
    case 'plan':
      if (!sid) toasts.push('info', String(t('chat.planViewer.requiresSession')))
      else openPlanViewer()
      return
  }
}

function showPluginCommandFeedback(result: PluginCommandResult) {
  const message = [result.title, result.summary].filter((value) => String(value || '').trim()).join(': ')
  if (result.status === 'succeeded') {
    toasts.push('success', message || 'Plugin operation completed')
  } else if (result.status === 'failed') {
    toasts.push('error', result.detail?.trim() || message || 'Plugin operation failed')
  } else {
    toasts.push('info', result.detail?.trim() || message || `Plugin operation ${result.status}`)
  }
  for (const diagnostic of result.diagnostics || []) {
    const path = diagnostic.path ? `${diagnostic.path}: ` : ''
    const detail = `${path}${diagnostic.message}`.trim()
    if (detail) toasts.push(result.status === 'failed' ? 'error' : 'info', detail)
  }
}

async function applyPluginCommandResult(
  result: PluginCommandResult,
  promptMode: 'submit' | 'return',
): Promise<string | null> {
  showPluginCommandFeedback(result)
  let returnedPrompt: string | null = null
  for (const effect of result.effects || []) {
    if (effect.kind === 'insert_prompt') {
      const prompt = effect.prompt.trim()
      if (!prompt) continue
      if (promptMode === 'return') returnedPrompt = prompt
      else await submitPromptFromCommand(prompt)
      continue
    }
    if (effect.kind === 'navigate') {
      if (!effect.path.startsWith('/')) throw new Error('Plugin navigation must use an application-relative path.')
      await router.push(effect.path)
      continue
    }
    if (effect.kind === 'open_url') {
      const url = new URL(effect.url, window.location.href)
      if (url.protocol !== 'http:' && url.protocol !== 'https:') {
        throw new Error('Plugin URLs must use HTTP or HTTPS.')
      }
      window.open(url.toString(), '_blank', 'noopener,noreferrer')
    }
  }
  return returnedPrompt
}

async function handleCommandSelected(command: Command) {
  try {
    if (command.kind === 'builtin') {
      await executeBuiltInCommand(command)
      return
    }
    const result = await chatCommands.runPluginSlashCommand(`/${command.name}`, commandSessionId() || '')
    if (result) await applyPluginCommandResult(result, 'submit')
  } catch (err) {
    toasts.push('error', err instanceof Error ? err.message : String(err))
  }
}

async function send() {
  if (failedDraftSlot.peek()) {
    toasts.push('info', String(t('chat.attachments.recoverBeforeSend')))
    return
  }
  if (sending.value) return
  const sid = chat.selectedSessionId
  const generation = attachmentGeneration()
  const errors = preparationErrorCount()
  sending.value = true
  try {
    await waitForAttachments()
    if (chat.selectedSessionId !== sid || attachmentGeneration() !== generation) return
    if (preparationErrorCount() !== errors) {
      toasts.push('info', String(t('chat.attachments.preparationFailed')))
      return
    }
    try {
      await sendReady(sid)
    } catch (error) {
      toasts.push('error', error instanceof Error ? error.message : String(error))
    }
  } finally {
    sending.value = false
  }
}

async function sendReady(sid: string | null) {
  let text = draft.value.trim()
  const filesSnapshot = attachedFiles.value.slice()
  const draftSnapshot = draft.value
  const segmentsSnapshot: ComposerSegment[] = composerRef.value?.getSegments?.() ?? [
    { type: 'text', text: draftSnapshot },
    ...filesSnapshot.map((file) => ({ type: 'attachment' as const, id: file.id })),
  ]
  let outgoingSegments = segmentsSnapshot

  // The editor document is what reaches the provider, so derive the outgoing
  // body text from that same source instead of trusting the draft string alone.
  // A send must never drop the text the user sees while still posting the
  // attachments; the two sources drifting apart is what produced a text message
  // plus a second, later attachment message in the transcript.
  const segmentText = segmentsSnapshot
    .filter((part): part is { type: 'text'; text: string } => part.type === 'text')
    .map((part) => part.text)
    .join('')
  if (!text) text = segmentText.trim()
  if ((!sid && filesSnapshot.length > 0) || (!text && filesSnapshot.length === 0)) return

  if (text && filesSnapshot.length === 0) {
    try {
      if (commands.value.length === 0) await loadCommands()
      const matchedCommand = matchSlashCommand(commands.value, text)
      if (matchedCommand?.command.kind === 'builtin') {
        draft.value = ''
        closeCommandPalette()
        await executeBuiltInCommand(matchedCommand.command, matchedCommand.args)
        return
      }
      const result = await chatCommands.runPluginSlashCommand(text, sid || '')
      if (result) {
        const prompt = await applyPluginCommandResult(result, 'return')
        if (prompt) {
          text = prompt
          outgoingSegments = [{ type: 'text', text: prompt }]
        } else {
          draft.value = ''
          closeCommandPalette()
          return
        }
      }
    } catch (err) {
      toasts.push('error', err instanceof Error ? err.message : String(err))
      return
    }
  }

  if (!sid || chat.selectedSessionId !== sid) return

  if (!modelSelection.selectedProviderId.value || !modelSelection.selectedModelId.value) {
    toasts.push('error', 'Select a model before sending')
    return
  }

  const runCfg = deriveSendRunConfig({
    selectedProviderId: modelSelection.selectedProviderId.value,
    selectedAdapterId: modelSelection.selectedAdapterId.value,
    selectedModelId: modelSelection.selectedModelId.value,
    selectedThinkingMode: modelSelection.selectedThinkingMode.value,
    selectedSpeedMode: modelSelection.selectedSpeedMode.value,
  })

  // UX: if the editor is expanded, collapse it on send.
  if (editorFullscreen.value && !editorClosing.value) {
    closeEditorFullscreen()
  }

  beginOptimisticSend({
    sessionId: sid,
    text,
    fileFallbackLabel: String(t('chat.messageItem.fileFallback')).trim(),
    attachmentTitle: String(t('chat.attachments.rowTitle')).trim(),
    files: filesSnapshot.map((f) => ({
      id: f.id,
      filename: f.filename,
      size: f.size,
      mime: f.mime,
      url: f.url,
      serverPath: f.serverPath,
      delivery: f.delivery,
    })),
  })

  // Remove only the submitted draft and files. A newer edit or attachment
  // staged while this send was preparing belongs to the next message.
  if (draft.value === draftSnapshot) draft.value = ''
  const sentFileIds = new Set(filesSnapshot.map((file) => file.id))
  for (const file of filesSnapshot) if (file.url) submittingAttachmentUrls.add(file.url)
  composerRef.value?.removeAttachmentNodes?.([...sentFileIds])
  attachedFiles.value = attachedFiles.value.filter((file) => !sentFileIds.has(file.id))
  commandOpen.value = false
  commandQuery.value = ''
  await nextTick()
  scrollToBottom('smooth')
  try {
    const parts: OutgoingMessagePart[] = []
    const workspace = filesSnapshot.length > 0 ? await chat.resolveSessionWorkspace(sid) : null
    const filesById = new Map(filesSnapshot.map((file) => [file.id, file]))
    const orderedSegments: ComposerSegment[] = outgoingSegments.map((part) => ({ ...part }))
    // Attachments below are force-included even when the editor has not
    // rendered their chip yet. Mirror that guarantee for the body text: a
    // message must never be submitted without the text the user sees.
    if (text && !orderedSegments.some((part) => part.type === 'text' && part.text.trim())) {
      orderedSegments.unshift({ type: 'text', text })
    }
    for (const file of filesSnapshot) {
      if (!orderedSegments.some((part) => part.type === 'attachment' && part.id === file.id)) {
        orderedSegments.push({ type: 'attachment', id: file.id })
      }
    }
    // Preserve text around each inline attachment. Only outer whitespace is
    // trimmed, matching the previous plain-text composer behavior.
    const first = orderedSegments[0]
    const last = orderedSegments[orderedSegments.length - 1]
    if (first?.type === 'text') first.text = first.text.trimStart()
    if (last?.type === 'text') last.text = last.text.trimEnd()
    for (const segment of orderedSegments) {
      if (segment.type === 'text') {
        if (segment.text) parts.push({ type: 'text', text: segment.text })
        continue
      }
      const f = filesById.get(segment.id)
      if (!f) continue
      let path = ''
      let sha256: string | undefined
      let size = Number.isFinite(f.size) && f.size > 0 ? Math.floor(f.size) : undefined
      const dataUrl = typeof f.url === 'string' ? f.url.trim() : ''
      if (dataUrl) {
        const dataBase64 = f.blob ? undefined : attachmentBase64(dataUrl)
        const uploaded = await chat.uploadWorkspaceAttachment(sid, {
          filename: f.filename,
          dataBase64,
          ...(f.blob ? { blob: f.blob } : {}),
          mime: f.mime,
        })
        path = String(uploaded.path || '').trim()
        sha256 = uploaded.sha256
        size = Number.isFinite(uploaded.size_bytes) ? Math.max(0, Math.floor(uploaded.size_bytes)) : size
      } else if (f.serverPath && workspace) {
        path = workspaceRelativePath(f.serverPath, workspace.path)
      }
      if (!path) throw new Error(`Attachment could not be prepared: ${f.filename}`)
      parts.push(
        resourceComposerNode({
          path,
          filename: f.filename,
          mime: f.mime,
          size,
          sha256,
          delivery: f.delivery || 'reference',
        }),
      )
    }

    const state = await chat.sendMessage(sid, { ...runCfg, parts })

    // Match the TUI: only a successfully submitted plain-text draft enters
    // global prompt history. Messages containing resources/attachments are
    // intentionally excluded because recalling them would silently lose data.
    if (filesSnapshot.length === 0) recordPromptHistory(text)

    // Mark the optimistic message as sent (generation may still be running).
    // Acknowledgement is the accepted user run id, not an id/text scan.
    markOptimisticSent(sid, chatApi.acceptedUserRunId(state), chatApi.acceptedAssistantReplyId(state))
  } catch (e) {
    // Keep UI consistent if send fails.
    clearOnSendFailure()

    // Restore composer content on failure.
    if (
      chat.selectedSessionId === sid &&
      !draft.value &&
      attachedFiles.value.length === 0 &&
      pendingAttachments.value.length === 0
    ) {
      draft.value = draftSnapshot
      attachedFiles.value = filesSnapshot
      await nextTick()
      composerRef.value?.restoreSegments?.(segmentsSnapshot)
    } else {
      failedDraftSlot.save({ sessionId: sid, text: draftSnapshot, files: filesSnapshot, segments: segmentsSnapshot })
      failedDraftVersion.value++
      toasts.push('error', String(t('chat.attachments.failedDraftSaved')))
    }
    throw e
  } finally {
    submittingAttachmentUrls.clear()
    attachments.releaseUnusedAttachmentUrls()
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
    if (pendingInitialScrollSessionId.value) return
    scheduleScrollToBottom()
  },
)

const lastHandledSessionActionSeq = ref(0)

watch(
  // Session actions can be requested from the ChatSidebar while ChatPage
  // is unmounted (mobile session switcher). Make the watcher immediate so a
  // pending request is handled on mount.
  () => [ui.sessionActionSeq, isFocusedWorkspacePane.value, chat.selectedSessionId] as const,
  ([seq, focused, sid]) => {
    if (!seq) return
    if (seq === lastHandledSessionActionSeq.value) return
    const actionId = ui.sessionActionId
    if (!actionId) return
    if (!focused || (ui.sessionActionSessionId && ui.sessionActionSessionId !== sid)) return
    if (workspacePane && ui.sessionActionWindowId && ui.sessionActionWindowId !== workspacePane.windowId.value) return
    lastHandledSessionActionSeq.value = seq
    handleSessionActionRequest(actionId)
    ui.clearSessionActionRequest()
  },
  { immediate: true, flush: 'post' },
)

onMounted(async () => {
  await transcriptPreferences.refresh()
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
    if (target instanceof Element && target.closest('[data-command-palette="true"]')) return
    if (target instanceof Element && target.closest('[data-prompt-history-palette="true"]')) return

    // Clicking anywhere else closes picker panels.
    closeComposerPickerMenu()

    // Don't dismiss command suggestions while the user is still interacting
    // with the textarea.
    const textarea = getComposerInput(composerRef.value)
    if (textarea && textarea.contains(target)) return
    closePromptHistory()
    closeCommandPalette()
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
  failedAttachmentDraft,
  restoreFailedAttachmentDraft,
  discardFailedAttachmentDraft,
  draft,
  attachedFiles,
  pendingAttachments,
  attachmentsBusy,
  attachmentsPanelOpen,
  formatBytes,
  handleDrop,
  handlePaste,
  handleDraftInput,
  handleDraftKeydown,
  handlePromptHistoryKeydown,
  selectPromptHistoryEntry,
  updatePromptHistoryQuery,
  handleCommandPaletteKeydown,
  commandOpen,
  commandQuery,
  commandIndex,
  commandFocusSearch,
  commands: filteredCommands,
  commandsLoading: chatCommands.commandsLoading,
  commandIcon: chatCommands.commandIcon,
  openCommandPalette,
  selectCommand,
  setCommandQuery,
  promptHistoryOpen,
  promptHistoryQuery,
  promptHistoryEntries: filteredPromptHistoryEntries,
  promptHistoryIndex,
  promptHistoryAutoFocus: promptHistoryOpen,
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
  composerMaxHeight,
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
  currentPhase,
  awaitingAssistant,
  optimisticUser,
  showOptimisticUser,
  handleForkFromMessage,
  handleRevertFromMessage,
  handleCopyMessage,
  handleCopySessionError,

  // Activity rendering.
  activityInitiallyExpandedForPart,
  activityCollapseSignal,
  isActivityExpanded,
  setActivityExpanded,
  isReasoningPart,
  isMetaPart,
  transcriptPartExpanded,
  setTranscriptPartExpanded,
  loadFoldedActivity,
  setTranscriptPartPageSize,

  // TUI-parity transcript navigation and search.
  transcriptSearchInputRef,
  transcriptVimEnabled,
  transcriptVimModeLabel,
  transcriptVimCommandLabel,
  transcriptSearchOpen,
  transcriptSearchQuery,
  transcriptSearchSummary,
  selectTranscriptNode,
  isTranscriptNodeSelected,
  isTranscriptNodeSearchMatch,
  setTranscriptSearchQuery,
  handleTranscriptSearchKeydown,
  openTranscriptSearch,
  jumpTranscriptSearch,
  closeTranscriptSearch,

  // Scroll + nav.
  handleScroll,
  handleWheel,
  loadOlderHistory: loadOlderAndPreserveViewport,
  preparePartReveal: () => {
    isAtBottom.value = false
  },
  isAtBottom,
  navigableMessageIds,
  navBottomOffset,
  navIndex,
  navTotalLabel,
  navCurrentOrdinal,
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
  composerStatusExtra,
  composerBottomLeftStatus,
  composerBottomRightStatus,
  returnFromSide,
  sideParentId,
  openPlanViewer,
  planViewer,
  planViewerOpen,
  planVisible,

  // Send/stop.
  sessionUsage,
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
  // Transcript data is owned by the application-level chat store, not by a
  // single pane.  A workspace split can unmount/recreate one ChatPage while
  // the other pane is still alive; clearing the shared cache here would make
  // the next focus change fetch the same transcript again and discard data
  // the user already loaded.  The store is intentionally kept until the app
  // lifecycle ends (or an explicit cache reset is requested).
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
</template>
