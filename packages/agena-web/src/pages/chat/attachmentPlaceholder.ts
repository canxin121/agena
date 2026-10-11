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
 * One label for one attachment everywhere: the text placeholder and the
 * transcript's attachment row must name the same attachment the same way, or a
 * message with several files all called `image.png` cannot be read.
 *
 * The ordinal is the attachment's 1-based position in the message. It leads the
 * label because the label itself is clipped by the chip's ellipsis and by the
 * placeholder limit: a trailing `#2` would disappear for long pasted-text
 * previews, and the number is the part that must never be lost.
 */
export function attachmentPlaceholder(label: string, options: { fallback?: string; ordinal?: number } = {}): string {
  const value = String(label || '').trim() || String(options.fallback || '').trim()
  const ordinal = Number.isFinite(options.ordinal) ? Math.floor(Number(options.ordinal)) : 0
  const badge = ordinal > 0 ? `#${ordinal} ` : ''
  if (!value) return badge ? `[${badge.trim()}]` : ''
  const characters = Array.from(value)
  const compact =
    characters.length > ATTACHMENT_PLACEHOLDER_MAX_CHARS
      ? `${characters.slice(0, ATTACHMENT_PLACEHOLDER_MAX_CHARS).join('')}…`
      : value
  return `[${badge}${compact}]`
}
