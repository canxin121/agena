import { computed, ref, type ComputedRef } from 'vue'

import type {
  MessageLike,
  MessagePartLike,
  RenderBlock,
  TranscriptDisplayPart,
} from '@/components/chat/messageList.types'
import {
  durablePartKind,
  createTranscriptProjector,
  transcriptPartText,
  type TranscriptProjectionLabels,
} from './transcriptProjection'
import { useWorkspacePaneContext } from '@/app/workspace/workspacePaneContext'

type ChatLike = { messages: MessageLike[] }
type SettingsLike = { data?: unknown }

export function useChatRenderBlocks(opts: {
  chat: ChatLike
  settings: SettingsLike
  showThinking: ComputedRef<boolean>
  formatTime: (ms?: number) => string
  labels?: () => TranscriptProjectionLabels
}) {
  const { chat, showThinking } = opts
  // Deliberately reference settings/formatTime so the composable's public
  // contract remains stable while presentation filtering moves to TUI parity.
  void opts.settings
  void opts.formatTime

  const pane = useWorkspacePaneContext()
  const project = createTranscriptProjector(() => ({
    showReasoning: showThinking.value,
    ...(opts.labels ? { labels: opts.labels() } : {}),
  }))
  const renderBlocks = computed<RenderBlock[]>(() => {
    if (pane && !pane.isVisible.value) {
      project([])
      return []
    }
    return project(chat.messages || [])
  })

  function getTextParts(parts: MessagePartLike[]): MessagePartLike[] {
    return (parts || []).filter((part) => {
      const kind = durablePartKind(part)
      if (kind !== 'text' && kind !== 'paste_ref') return false
      if (part.ignored) return false
      return Boolean(transcriptPartText(part).trim())
    })
  }

  function isReasoningPart(part: MessagePartLike): boolean {
    return durablePartKind(part) === 'think'
  }

  function isMetaPart(part: MessagePartLike): boolean {
    return ['notice', 'hook', 'compaction', 'system_notification'].includes(durablePartKind(part))
  }

  const activityExpandedByBlockKey = ref<Record<string, boolean>>({})
  const activityCollapseSignal = ref(0)

  function collapseAllActivities() {
    // This map contains only explicit user choices. Finishing a run may
    // restore automatic list folding, but must not undo those choices.
    activityCollapseSignal.value += 1
  }

  function isActivityExpanded(partKey: string): boolean {
    return Boolean(activityExpandedByBlockKey.value[partKey])
  }

  function transcriptPartExpanded(part: TranscriptDisplayPart, initiallyExpanded = false): boolean {
    // Read the key even before it exists so Vue observes the first explicit
    // choice. Checking hasOwnProperty alone does not subscribe to an absent
    // key, leaving default-open parts visibly open after the first collapse.
    return activityExpandedByBlockKey.value[part.key] ?? (part.defaultExpanded || initiallyExpanded)
  }

  function setActivityExpanded(partKey: string, expanded: boolean) {
    activityExpandedByBlockKey.value[partKey] = expanded
  }

  return {
    renderBlocks,
    getTextParts,
    isReasoningPart,
    isMetaPart,
    activityExpandedByBlockKey,
    activityCollapseSignal,
    collapseAllActivities,
    isActivityExpanded,
    transcriptPartExpanded,
    setActivityExpanded,
  }
}

export type { RenderBlock }
