import { computed, ref, type ComputedRef } from 'vue'

import type { MessageLike, MessagePartLike, RenderBlock } from '@/components/chat/messageList.types'
import { durablePartKind, projectTranscriptBlocks, transcriptPartText } from './transcriptProjection'

type ChatLike = { messages: MessageLike[] }
type SettingsLike = { data?: unknown }

export function useChatRenderBlocks(opts: {
  chat: ChatLike
  settings: SettingsLike
  showThinking: ComputedRef<boolean>
  formatTime: (ms?: number) => string
}) {
  const { chat, showThinking } = opts
  // Deliberately reference settings/formatTime so the composable's public
  // contract remains stable while presentation filtering moves to TUI parity.
  void opts.settings
  void opts.formatTime

  const renderBlocks = computed<RenderBlock[]>(() =>
    projectTranscriptBlocks(chat.messages || [], {
      showReasoning: showThinking.value,
    }),
  )

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
    activityExpandedByBlockKey.value = {}
    activityCollapseSignal.value += 1
  }

  function isActivityExpanded(partKey: string): boolean {
    return Boolean(activityExpandedByBlockKey.value[partKey])
  }

  function setActivityExpanded(partKey: string, expanded: boolean) {
    activityExpandedByBlockKey.value = { ...activityExpandedByBlockKey.value, [partKey]: expanded }
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
    setActivityExpanded,
  }
}

export type { RenderBlock }
