/**
 * Placeholder that marks an attachment's position inside the submitted text.
 *
 * The composer keeps every attachment at a text position (an inline chip whose
 * label is the accepted filename or the pasted-text preview). The submitted
 * message keeps those positions as plain bracketed labels, so the text the user
 * typed survives the send, the model can still tell what belonged where, and
 * the attachment itself is submitted as its own part instead of splitting the
 * surrounding text. The brackets match the TUI composer placeholder
 * (`attachment-placeholder = [{$kind} {$filename}]`).
 */
export const ATTACHMENT_PLACEHOLDER_MAX_CHARS = 24

/**
 * `[<label>]`, truncated by code point so a long pasted-text preview cannot
 * push the rest of the message around. An empty label and fallback produce an
 * empty string: an unlabeled attachment must not leave a bare `[]` in the
 * submitted text.
 */
export function attachmentPlaceholder(label: string, fallback = ''): string {
  const value = String(label || '').trim() || String(fallback || '').trim()
  if (!value) return ''
  const characters = Array.from(value)
  const compact =
    characters.length > ATTACHMENT_PLACEHOLDER_MAX_CHARS
      ? `${characters.slice(0, ATTACHMENT_PLACEHOLDER_MAX_CHARS).join('')}…`
      : value
  return `[${compact}]`
}
