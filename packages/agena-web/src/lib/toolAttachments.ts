type RecordValue = Record<string, unknown>

function record(value: unknown): RecordValue {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as RecordValue) : {}
}

/** The transcript keeps attachment descriptors even when the large payload is
 * deferred. In particular, payload:null does not mean there is no attachment. */
export function toolOutputAttachments(output: unknown): RecordValue[] {
  const value = record(output)
  const attachments = value.attachments ?? record(value.raw_output).attachments
  return Array.isArray(attachments) ? attachments.map(record).filter((item) => Object.keys(item).length > 0) : []
}

export function hasToolAttachmentOutput(content: unknown, presentation: unknown): boolean {
  if (toolOutputAttachments(record(content).output).length > 0) return true
  const blocks = record(presentation).blocks
  return (
    Array.isArray(blocks) &&
    blocks.some((block) =>
      ['media', 'image', 'audio', 'video', 'file'].includes(String(record(block).type ?? record(block).kind ?? '')),
    )
  )
}

/** A list snapshot carries only attachment descriptors. The output section
 * remains deferred until a section response/revision marks it as loaded. */
export function isDeferredAttachmentOutput(output: unknown): boolean {
  const value = record(output)
  return value.payload === null && toolOutputAttachments(value).length > 0
}
