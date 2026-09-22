import type { StagedAttachment } from './attachmentIngestion'
export type FailedAttachmentDraft = { sessionId: string; text: string; files: StagedAttachment[] }
/** Bounded recovery slot. It is local UI state, never an automatic retry. */
export function createFailedAttachmentDraftSlot() {
  let draft: FailedAttachmentDraft | null = null
  return {
    save(value: FailedAttachmentDraft) {
      // Only one send can be in flight. The UI disables another send while
      // this recovery slot is occupied, avoiding an unbounded binary queue.
      if (draft) throw new Error('An earlier failed draft must be restored or discarded first')
      draft = { ...value, files: value.files.map((file) => ({ ...file })) }
    },
    peek: () => draft,
    take(sessionId: string, currentText: string, currentFiles: StagedAttachment[]) {
      if (!draft || draft.sessionId !== sessionId || currentText || currentFiles.length) return null
      const result = draft; draft = null; return result
    },
    discard() { draft = null },
  }
}
