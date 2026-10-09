/** Local staging only. This module never contacts a model provider. */
import { i18n } from '@/i18n'
export type AttachmentDelivery = 'reference' | 'model_input'
/**
 * Preparation state of one composer attachment. It is the only difference
 * between a chip that is still being prepared and one that is ready to
 * send; no consumer relies on which list an item happens to live in.
 */
export type AttachmentState = 'preparing' | 'ready'
/** One composer attachment, shaped like the `file_ref` payload it becomes. */
export type ComposerAttachment = {
  id: string
  filename: string
  size: number
  mime: string
  url?: string
  blob?: Blob
  serverPath?: string
  delivery?: AttachmentDelivery
  state: AttachmentState
  pastePreview?: string
}
/** @deprecated use {@link ComposerAttachment}; kept for older call sites. */
export type StagedAttachment = ComposerAttachment
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
  onError: (error: { kind: 'size' | 'total' | 'count' | 'read' | 'memory'; file: File }) => void
  onBusy: (count: number) => void
  release?: (url: string) => void
  remainingBytes?: () => number
}
type StageOptions = {
  idForFile?: (file: File) => string
  shouldSkip?: (file: File) => boolean
}
export function createAttachmentIngestor(options: Options) {
  let generation = 0
  let pending = 0
  let queue: Promise<void> = Promise.resolve()
  const active = new Set<AbortController>()
  const activeById = new Map<string, AbortController>()
  async function process(files: File[], epoch: number, stageOptions: StageOptions): Promise<File[]> {
    const accepted: File[] = []
    for (const file of files) {
      if (epoch !== generation) break
      if (stageOptions.shouldSkip?.(file)) continue
      const current = options.get()
      if (current.length >= MAX_ATTACHMENTS) {
        options.onError({ kind: 'count', file })
        break
      }
      if (!Number.isFinite(file.size) || file.size <= 0 || file.size > MAX_ATTACHMENT_BYTES) {
        options.onError({ kind: 'size', file })
        continue
      }
      if (current.reduce((n, f) => n + Math.max(0, f.size || 0), 0) + file.size > MAX_ATTACHMENT_TOTAL_BYTES) {
        options.onError({ kind: 'total', file })
        continue
      }
      if (options.remainingBytes && file.size > options.remainingBytes()) {
        options.onError({ kind: 'memory', file })
        continue
      }
      const abort = new AbortController()
      active.add(abort)
      const stagedId = stageOptions.idForFile?.(file)
      if (stagedId) activeById.set(stagedId, abort)
      let url: string
      try {
        url = await options.read(file, abort.signal)
      } catch {
        if (epoch === generation && !stageOptions.shouldSkip?.(file)) options.onError({ kind: 'read', file })
        continue
      } finally {
        active.delete(abort)
        if (stagedId) activeById.delete(stagedId)
      }
      if (epoch !== generation || abort.signal.aborted) {
        options.release?.(url)
        break
      }
      if (stageOptions.shouldSkip?.(file)) {
        options.release?.(url)
        continue
      }
      if (!url.startsWith('blob:') && (!url.startsWith('data:') || !url.includes(';base64,'))) {
        options.release?.(url)
        options.onError({ kind: 'read', file })
        continue
      }
      // Actual contents decide duplicates. Same-name/same-size screenshots
      // may differ and must never disappear on a filename-only comparison.
      let duplicate = false
      for (const item of options.get()) {
        if (item.filename !== (file.name || 'file')) continue
        if (
          item.url === url ||
          item.blob === file ||
          (item.blob && item.size === file.size && (await equalBlobs(item.blob, file)))
        ) {
          // Hashing can yield while the user removes the original chip. Only
          // suppress the new file if that duplicate still belongs to the draft.
          duplicate = options
            .get()
            .some((latest) => latest.id === item.id && latest.url === item.url && latest.blob === item.blob)
          if (duplicate) break
        }
      }
      if (epoch !== generation || abort.signal.aborted) {
        options.release?.(url)
        break
      }
      if (stageOptions.shouldSkip?.(file)) {
        options.release?.(url)
        continue
      }
      if (duplicate) {
        options.release?.(url)
        accepted.push(file)
        continue
      }
      const latest = options.get()
      if (latest.length >= MAX_ATTACHMENTS) {
        options.release?.(url)
        options.onError({ kind: 'count', file })
        break
      }
      if (latest.reduce((n, f) => n + Math.max(0, f.size || 0), 0) + file.size > MAX_ATTACHMENT_TOTAL_BYTES) {
        options.release?.(url)
        options.onError({ kind: 'total', file })
        continue
      }
      options.set([
        ...latest,
        {
          id: stagedId || `file-${Date.now()}-${Math.random().toString(36).slice(2)}`,
          filename: file.name || 'file',
          size: file.size,
          mime: file.type || 'application/octet-stream',
          url,
          ...(url.startsWith('blob:') ? { blob: file } : {}),
          delivery: 'model_input',
          state: 'ready',
        },
      ])
      accepted.push(file)
    }
    return accepted
  }
  function stage(files: FileList | File[], stageOptions: StageOptions = {}): Promise<File[]> {
    const epoch = generation
    const snapshot = Array.from(files)
    pending++
    options.onBusy(pending)
    const result = queue.then(() => process(snapshot, epoch, stageOptions))
    queue = result.then(
      () => undefined,
      () => undefined,
    )
    return result.finally(() => {
      if (epoch === generation) {
        pending--
        options.onBusy(pending)
      }
    })
  }
  function clear() {
    generation++
    for (const controller of active) controller.abort()
    active.clear()
    activeById.clear()
    // A cancelled reader must not keep a later draft behind its old promise.
    queue = Promise.resolve()
    pending = 0
    options.onBusy(0)
    options.set([])
  }
  function cancel(id: string) {
    activeById.get(id)?.abort()
  }
  return { stage, clear, cancel, generation: () => generation }
}

const blobHashes = new WeakMap<Blob, Promise<string>>()
function blobHash(blob: Blob) {
  let hash = blobHashes.get(blob)
  if (!hash) {
    hash = blob
      .arrayBuffer()
      .then((bytes) => crypto.subtle.digest('SHA-256', bytes))
      .then((digest) => Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, '0')).join(''))
    blobHashes.set(blob, hash)
  }
  return hash
}
async function equalBlobs(left: Blob, right: Blob) {
  // Hash only same-name/same-size candidates. New files normally require no
  // body read at all, and WebCrypto performs the hash outside JavaScript.
  try {
    const hashes = await Promise.all([blobHash(left), blobHash(right)])
    return hashes[0] === hashes[1]
  } catch {
    return false
  }
}

export function filesFromClipboard(data: Pick<DataTransfer, 'items' | 'files'> | null): File[] {
  if (!data) return []
  const fromItems = Array.from(data.items || [])
    .filter((item) => item.kind === 'file')
    .map((item) => item.getAsFile())
    .filter((file): file is File => !!file)
  // Browsers expose the same files in both collections. Use items when
  // present; do not collapse distinct files on weak filename/size metadata.
  return fromItems.length ? fromItems : Array.from(data.files || [])
}

export function readLocalDataUrl(file: File, signal: AbortSignal): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    const cleanup = () => signal.removeEventListener('abort', abort)
    const abort = () => {
      reader.abort()
      cleanup()
      reject(new DOMException('Attachment read cancelled', 'AbortError'))
    }
    reader.onload = () => {
      cleanup()
      resolve(String(reader.result || ''))
    }
    reader.onerror = () => {
      cleanup()
      reject(reader.error || new Error(i18n.global.t('chat.errors.attachmentReadFailed')))
    }
    reader.onabort = () => {
      cleanup()
      reject(new DOMException('Attachment read cancelled', 'AbortError'))
    }
    if (signal.aborted) {
      abort()
      return
    }
    signal.addEventListener('abort', abort, { once: true })
    reader.readAsDataURL(file)
  })
}
