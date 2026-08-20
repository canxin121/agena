import { computed, ref, type ComputedRef } from 'vue'

import type { MessageLike, MessagePartLike, RenderBlock, RevertLike } from '@/components/chat/messageList.types'
import { durablePartKind, projectTranscriptBlocks, transcriptPartText } from './transcriptProjection'

export type RevertStateLike = {
  messageID: string
  revertedUserCount?: number
  diffFiles?: Array<{ filename?: string; additions?: number; deletions?: number }>
}

type ChatLike = { messages: MessageLike[] }
type SettingsLike = { data?: unknown }

function normalizeRevertState(input: RevertStateLike | null): RevertLike | null {
  if (!input) return null
  const messageID = String(input.messageID || '').trim()
  if (!messageID) return null
  const revertedUserCount = Number.isFinite(input.revertedUserCount)
    ? Math.max(0, Math.floor(Number(input.revertedUserCount)))
    : 0
  const diffFiles = Array.isArray(input.diffFiles)
    ? input.diffFiles
        .map((item) => {
          const filename = String(item?.filename || '').trim()
          if (!filename) return null
          return {
            filename,
            additions: Number.isFinite(item?.additions) ? Math.max(0, Math.floor(Number(item?.additions))) : 0,
            deletions: Number.isFinite(item?.deletions) ? Math.max(0, Math.floor(Number(item?.deletions))) : 0,
          }
        })
        .filter((item): item is { filename: string; additions: number; deletions: number } => Boolean(item))
    : []
  return { messageID, revertedUserCount, diffFiles }
}

export function useChatRenderBlocks(opts: {
  chat: ChatLike
  settings: SettingsLike
  showThinking: ComputedRef<boolean>
  showJustification: ComputedRef<boolean>
  revertState: ComputedRef<RevertStateLike | null>
  formatTime: (ms?: number) => string
}) {
  const { chat, showThinking, showJustification, revertState } = opts
  // Deliberately reference settings/formatTime so the composable's public
  // contract remains stable while presentation filtering moves to TUI parity.
  void opts.settings
  void opts.formatTime

  const renderBlocks = computed<RenderBlock[]>(() =>
    projectTranscriptBlocks(chat.messages || [], {
      showReasoning: showThinking.value,
      showJustification: showJustification.value,
      revert: normalizeRevertState(revertState.value),
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

  function isJustificationPart(part: MessagePartLike): boolean {
    return String(part?.type || '')
      .trim()
      .toLowerCase()
      .includes('justification')
  }

  function isMetaPart(part: MessagePartLike): boolean {
    return ['notice', 'hook', 'compaction', 'system_notification'].includes(durablePartKind(part))
  }

  const activityExpandedByBlockKey = ref<Record<string, boolean>>({})
  const activityCollapseSignal = ref(0)
  const activityExpandAllSignal = ref(0)
  const MAX_VISIBLE_ACTIVITY_COLLAPSED = Number.POSITIVE_INFINITY

  function collapseAllActivities() {
    activityExpandedByBlockKey.value = {}
    activityCollapseSignal.value += 1
  }

  /** Expand every currently loaded fold and every loaded part detail. */
  function expandAllActivities() {
    activityExpandAllSignal.value += 1
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
    isJustificationPart,
    isMetaPart,
    MAX_VISIBLE_ACTIVITY_COLLAPSED,
    activityExpandedByBlockKey,
    activityCollapseSignal,
    activityExpandAllSignal,
    collapseAllActivities,
    expandAllActivities,
    isActivityExpanded,
    setActivityExpanded,
  }
}

export type { RenderBlock }
