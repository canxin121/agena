import type { TranscriptDisplayPart } from '../../components/chat/messageList.types'
import { chatActivityKindIdForTranscriptPart, normalizeChatToolActivityId } from '../../lib/chatActivity'
import { partIsDelegatedTask } from './transcriptProjection'

export type TranscriptExpansionPreferences = {
  toolExpanded: (tool: unknown) => boolean | undefined
  toolCategoryOverrides: Readonly<Record<string, boolean>>
  kindExpanded: (kind: string) => boolean
}

function operationToolId(part: TranscriptDisplayPart): string {
  const raw = typeof part.source.tool === 'string' ? part.source.tool.trim() : ''
  const content = part.source.agenaContent
  const plugin =
    content && typeof content === 'object' && !Array.isArray(content) && typeof content.plugin === 'string'
      ? content.plugin.trim()
      : ''
  if (!plugin || !raw || raw.toLowerCase().startsWith(`${plugin.toLowerCase()}.`)) return raw
  return `${plugin}.${raw}`
}

export function activityInitiallyExpandedForPart(
  part: TranscriptDisplayPart,
  preferences: TranscriptExpansionPreferences,
): boolean {
  const activityKind = chatActivityKindIdForTranscriptPart(part.kind, part.source.agenaKind)
  if (!activityKind) return false
  if (activityKind !== 'operation') return preferences.kindExpanded(activityKind)

  const toolId = operationToolId(part)
  const exact = preferences.toolExpanded(toolId)
  if (exact !== undefined) return exact
  const category = normalizeChatToolActivityId(toolId)
  if (Object.prototype.hasOwnProperty.call(preferences.toolCategoryOverrides, category)) {
    return preferences.toolCategoryOverrides[category] === true
  }
  if (!category && Object.prototype.hasOwnProperty.call(preferences.toolCategoryOverrides, 'unknown')) {
    return preferences.toolCategoryOverrides.unknown === true
  }
  if (partIsDelegatedTask(part.source)) return true
  return preferences.kindExpanded('operation')
}
