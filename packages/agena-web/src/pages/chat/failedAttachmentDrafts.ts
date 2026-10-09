import { i18n } from '@/i18n'
import type { StagedAttachment } from './attachmentIngestion'
import type { ComposerSegment } from './composerInput'
export type FailedAttachmentDraft = {
  sessionId: string
  text: string
  files: StagedAttachment[]
  segments?: ComposerSegment[]
}
/** Bounded recovery slot. It is local UI state, never an automatic retry. */
export function createFailedAttachmentDraftSlot() {
  let draft: FailedAttachmentDraft | null = null
  return {
    save(value: FailedAttachmentDraft) {
      // Only one send can be in flight. The UI disables another send while
      // this recovery slot is occupied, avoiding an unbounded binary queue.
      if (draft) throw new Error(i18n.global.t('chat.errors.failedDraftMustBeResolved'))
      draft = {
        ...value,
        files: value.files.map((file) => ({ ...file })),
        segments: value.segments?.map((part) => ({ ...part })),
      }
    },
    peek: () => draft,
    take(sessionId: string, currentText: string, currentFiles: StagedAttachment[]) {
      if (!draft || draft.sessionId !== sessionId || currentText || currentFiles.length) return null
      const result = draft
      draft = null
      return result
    },
    discard() {
      draft = null
    },
  }
}
