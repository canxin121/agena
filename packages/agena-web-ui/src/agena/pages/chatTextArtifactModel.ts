export const TEXT_ARTIFACT_PASTE_THRESHOLD = 1000
export const MAX_COMPOSER_TEXT_ARTIFACTS = 8

export type ComposerTextArtifactDraft = {
  id: string
  text: string
  label?: string
  createdAt: number
}

export function createComposerTextArtifactDraft(text: string, label?: string): ComposerTextArtifactDraft {
  const trimmedLabel = label?.trim()
  return {
    id: typeof crypto !== 'undefined' && crypto.randomUUID ? crypto.randomUUID() : `${Date.now()}-text-artifact`,
    text,
    ...(trimmedLabel ? { label: trimmedLabel } : {}),
    createdAt: Date.now(),
  }
}

export function composerTextArtifactPreview(draft: ComposerTextArtifactDraft, maxLength = 80): string {
  const value = (draft.label || draft.text).replace(/\s+/g, ' ').trim()
  return value.length > maxLength ? `${value.slice(0, Math.max(1, maxLength - 1))}…` : value
}

export function textArtifactPlaceholder(index: number): string {
  return `[已粘贴文本 #${index}]`
}
