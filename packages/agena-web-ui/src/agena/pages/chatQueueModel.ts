import type { ComposerAttachmentDraft } from './chatAttachmentModel'
import type { ComposerSkillDraft } from './chatSkillModel'

export type ComposerQueueItem = {
  id: string
  text: string
  attachments: ComposerAttachmentDraft[]
  skills: ComposerSkillDraft[]
  createdAt: number
}

export function createComposerQueueItem(
  text: string,
  attachments: ComposerAttachmentDraft[],
  skills: ComposerSkillDraft[] = [],
): ComposerQueueItem {
  return {
    id:
      typeof crypto !== 'undefined' && crypto.randomUUID ? crypto.randomUUID() : `${Date.now()}-${attachments.length}`,
    text,
    attachments: [...attachments],
    skills: [...skills],
    createdAt: Date.now(),
  }
}

export function composerQueuePreview(item: ComposerQueueItem, maxLength = 80): string {
  const text = item.text.replace(/\s+/g, ' ').trim()
  const fallback = item.attachments.length
    ? `${item.attachments.length} attachment(s)`
    : item.skills.length
      ? `${item.skills.length} Skill reference(s)`
      : 'empty draft'
  const value = text || fallback
  return value.length > maxLength ? `${value.slice(0, Math.max(1, maxLength - 1))}…` : value
}
