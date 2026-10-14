export type ComposerInputMenuKind = 'attachments' | 'picker'

type ComposerInputMenuClosers = {
  closeAttachments: () => void
  closePicker: () => void
}

export function closeAllComposerInputMenus(closers: ComposerInputMenuClosers) {
  closers.closeAttachments()
  closers.closePicker()
}

export function openComposerInputMenu(kind: ComposerInputMenuKind, closers: ComposerInputMenuClosers) {
  if (kind !== 'attachments') closers.closeAttachments()
  if (kind !== 'picker') closers.closePicker()
}
