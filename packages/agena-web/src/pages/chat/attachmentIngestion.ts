/** Local staging only. This module never contacts a model provider. */
export type AttachmentDelivery = 'reference' | 'model_input'
export type StagedAttachment = {
  id: string
  filename: string
  size: number
  mime: string
  url?: string
  serverPath?: string
  delivery?: AttachmentDelivery
}
export const MAX_ATTACHMENT_BYTES = 20 * 1024 * 1024
export const MAX_ATTACHMENT_TOTAL_BYTES = 40 * 1024 * 1024
export const MAX_ATTACHMENTS = 8
export const MAX_PASTE_TEXT_BYTES = 1024 * 1024
export function pasteTextWithinBudget(text: string) {
  // Every UTF-16 code unit uses at least one encoded UTF-8 byte here. Reject
  // huge strings before allocating another full encoded copy.
  return text.length <= MAX_PASTE_TEXT_BYTES && new TextEncoder().encode(text).byteLength <= MAX_PASTE_TEXT_BYTES
}


type Options = {
  get: () => StagedAttachment[]
  set: (files: StagedAttachment[]) => void
  read: (file: File, signal: AbortSignal) => Promise<string>
  onError: (error: { kind: 'size' | 'total' | 'count' | 'read'; file: File }) => void
  onBusy: (count: number) => void
}
export function createAttachmentIngestor(options: Options) {
  let generation = 0
  let pending = 0
  let queue: Promise<void> = Promise.resolve()
  const active = new Set<AbortController>()
  async function process(files: File[], epoch: number): Promise<File[]> {
    const accepted: File[] = []
    for (const file of files) {
      if (epoch !== generation) break
      const current = options.get()
      if (current.length >= MAX_ATTACHMENTS) {
        options.onError({ kind: 'count', file }); break
      }
      if (!Number.isFinite(file.size) || file.size <= 0 || file.size > MAX_ATTACHMENT_BYTES) {
        options.onError({ kind: 'size', file }); continue
      }
      if (current.reduce((n, f) => n + Math.max(0, f.size || 0), 0) + file.size > MAX_ATTACHMENT_TOTAL_BYTES) {
        options.onError({ kind: 'total', file }); continue
      }
      const abort = new AbortController(); active.add(abort)
      let url: string
      try { url = await options.read(file, abort.signal) }
      catch { if (epoch === generation) options.onError({ kind: 'read', file }); continue }
      finally { active.delete(abort) }
      if (epoch !== generation || abort.signal.aborted) break
      if (!url.startsWith('data:') || !url.includes(';base64,')) {
        options.onError({ kind: 'read', file }); continue
      }
      // Actual contents decide duplicates. Same-name/same-size screenshots
      // may differ and must never disappear on a filename-only comparison.
      if (options.get().some((f) => f.url === url && f.filename === (file.name || 'file'))) {
        accepted.push(file); continue
      }
      const latest = options.get()
      if (latest.length >= MAX_ATTACHMENTS) { options.onError({ kind: 'count', file }); break }
      if (latest.reduce((n, f) => n + Math.max(0, f.size || 0), 0) + file.size > MAX_ATTACHMENT_TOTAL_BYTES) {
        options.onError({ kind: 'total', file }); continue
      }
      options.set([...latest, {
        id: `file-${Date.now()}-${Math.random().toString(36).slice(2)}`,
        filename: file.name || 'file', size: file.size,
        mime: file.type || 'application/octet-stream', url, delivery: 'model_input',
      }])
      accepted.push(file)
    }
    return accepted
  }
  function stage(files: FileList | File[]): Promise<File[]> {
    const epoch = generation; const snapshot = Array.from(files)
    pending++; options.onBusy(pending)
    const result = queue.then(() => process(snapshot, epoch))
    queue = result.then(() => undefined, () => undefined)
    return result.finally(() => { if (epoch === generation) { pending--; options.onBusy(pending) } })
  }
  function clear() {
    generation++
    for (const controller of active) controller.abort()
    active.clear()
    // A cancelled reader must not keep a later draft behind its old promise.
    queue = Promise.resolve(); pending = 0; options.onBusy(0); options.set([])
  }
  return { stage, clear, generation: () => generation }
}

export function filesFromClipboard(data: Pick<DataTransfer, 'items' | 'files'> | null): File[] {
  if (!data) return []
  const fromItems = Array.from(data.items || []).filter((item) => item.kind === 'file').map((item) => item.getAsFile()).filter((file): file is File => !!file)
  // Browsers expose the same files in both collections. Use items when
  // present; do not collapse distinct files on weak filename/size metadata.
  return fromItems.length ? fromItems : Array.from(data.files || [])
}

export function readLocalDataUrl(file: File, signal: AbortSignal): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    const cleanup = () => signal.removeEventListener('abort', abort)
    const abort = () => { reader.abort(); cleanup(); reject(new DOMException('Attachment read cancelled', 'AbortError')) }
    reader.onload = () => { cleanup(); resolve(String(reader.result || '')) }
    reader.onerror = () => { cleanup(); reject(reader.error || new Error('Attachment read failed')) }
    reader.onabort = () => { cleanup(); reject(new DOMException('Attachment read cancelled', 'AbortError')) }
    if (signal.aborted) { abort(); return }
    signal.addEventListener('abort', abort, { once: true })
    reader.readAsDataURL(file)
  })
}
