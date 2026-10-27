export type AdditionalSystemPromptMode = 'inherit' | 'custom' | 'disabled'

export type AdditionalSystemPromptDraft = {
  mode: AdditionalSystemPromptMode
  text: string
}

/** Missing/null inherits; an exact empty string explicitly disables the addition. */
export function additionalSystemPromptDraft(value: unknown): AdditionalSystemPromptDraft {
  if (typeof value !== 'string') return { mode: 'inherit', text: '' }
  return value === '' ? { mode: 'disabled', text: '' } : { mode: 'custom', text: value }
}

export function additionalSystemPromptValue(draft: AdditionalSystemPromptDraft): string | null {
  if (draft.mode === 'inherit') return null
  return draft.mode === 'disabled' ? '' : draft.text
}

export function additionalSystemPromptIsValid(draft: AdditionalSystemPromptDraft): boolean {
  return draft.mode !== 'custom' || draft.text.length > 0
}

export function additionalSystemPromptIsDirty(draft: AdditionalSystemPromptDraft, saved: string | null): boolean {
  const baseline = additionalSystemPromptDraft(saved)
  return draft.mode !== baseline.mode || (draft.mode === 'custom' && draft.text !== baseline.text)
}
