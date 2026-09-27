export type ComposerSegment = { type: 'text'; text: string } | { type: 'attachment'; id: string }

// The editor exposes the small textarea API used by composer shortcuts. Its
// backing element is contenteditable so attachments can occupy a text position.
export type ComposerInput = {
  readonly element: HTMLDivElement | null
  readonly value: string
  readonly selectionStart: number
  readonly selectionEnd: number
  focus: () => void
  blur: () => void
  contains: (node: Node | null) => boolean
  setSelectionRange: (start: number, end: number) => void
  setRangeText: (
    replacement: string,
    start: number,
    end: number,
    selectionMode?: 'select' | 'start' | 'end' | 'preserve',
  ) => void
}

export type ComposerExpose = {
  shellEl?: HTMLDivElement | { value: HTMLDivElement | null } | null
  textareaEl?: ComposerInput | { value: ComposerInput | null } | null
  openFilePicker?: () => void
  getSegments?: () => ComposerSegment[]
  replaceAttachmentWithText?: (id: string, text: string) => void
  removeAttachmentNodes?: (ids: string[]) => void
  restoreSegments?: (segments: ComposerSegment[]) => void
  insertText?: (text: string) => void
}

export function getComposerInput(composer: ComposerExpose | null): ComposerInput | null {
  const input = composer?.textareaEl
  if (!input) return null
  return 'focus' in input ? input : input.value
}
