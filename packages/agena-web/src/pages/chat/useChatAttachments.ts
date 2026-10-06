import { computed, ref, onScopeDispose, watch, type Ref } from 'vue'
import { createBlobUrlRegistry, MAX_LOCAL_ATTACHMENT_MEMORY_BYTES } from '../../lib/blobUrlRegistry'
import { i18n } from '@/i18n'
import {
  createAttachmentIngestor,
  filesFromClipboard,
  pasteTextWithinBudget,
  MAX_ATTACHMENT_TOTAL_BYTES,
  MAX_ATTACHMENTS,
  type ComposerAttachment,
} from './attachmentIngestion'

type ToastKind = 'info' | 'success' | 'error'
type Toasts = { push: (kind: ToastKind, message: string, timeoutMs?: number) => void }

type ComposerExpose = {
  openFilePicker?: () => void
  insertText?: (text: string) => void
  replaceAttachmentWithText?: (id: string, text: string) => void
}

export type AttachedFile = ComposerAttachment & { state: 'ready' }
export type PendingAttachment = ComposerAttachment & { state: 'preparing' }

// Attachment handling (local uploads + project file references).
export function useChatAttachments(opts: {
  toasts: Toasts
  composerRef: Ref<ComposerExpose | null>
  restoreText?: (text: string) => void
  retainedUrls?: () => Iterable<string>
}) {
  const { toasts, composerRef } = opts

  // One attachment list. Ready and pending chips are projections of it, and
  // the `state` field is the only thing that tells them apart.
  const attachments = ref<ComposerAttachment[]>([])
  const attachedFiles = computed<AttachedFile[]>({
    get: () => attachments.value.filter((file): file is AttachedFile => file.state === 'ready'),
    set: (files) => {
      const pending = attachments.value.filter((file) => file.state === 'preparing')
      attachments.value = [...files.map((file) => ({ ...file, state: 'ready' as const })), ...pending]
    },
  })
  const pendingAttachments = computed<PendingAttachment[]>({
    get: () => attachments.value.filter((file): file is PendingAttachment => file.state === 'preparing'),
    set: (files) => {
      const ready = attachments.value.filter((file) => file.state === 'ready')
      attachments.value = [...ready, ...files.map((file) => ({ ...file, state: 'preparing' as const }))]
    },
  })
  const preparation = new Set<Promise<unknown>>()
  const cancelled = new Set<string>()
  let preparationErrors = 0

  const attachBusyCount = ref(0)
  const attachmentsBusy = computed(() => attachBusyCount.value > 0)

  const MAX_RESOURCE_ATTACHMENTS = MAX_ATTACHMENTS
  const LONG_PASTE_TEXT_CHARS = 1_000
  const objectUrls = createBlobUrlRegistry()
  const preparingUrls = new Set<string>()
  const releaseUrl = (url: string) => {
    preparingUrls.delete(url)
    objectUrls.release(url)
  }
  function releaseUnusedAttachmentUrls() {
    objectUrls.releaseUnreferenced(
      new Set([
        ...preparingUrls,
        ...(opts.retainedUrls?.() ?? []),
        ...attachments.value.flatMap((file) => (file.url ? [file.url] : [])),
      ]),
    )
  }
  watch(attachments, releaseUnusedAttachmentUrls, { flush: 'post' })
  const ingestion = createAttachmentIngestor({
    get: () => attachedFiles.value,
    set: (files) => {
      attachedFiles.value = files.map((file) => ({ ...file, state: 'ready' as const }))
      for (const file of files) if (file.url) preparingUrls.delete(file.url)
    },
    read: async (file, signal) => {
      signal.throwIfAborted()
      const url = objectUrls.create(file)
      preparingUrls.add(url)
      return url
    },
    release: releaseUrl,
    remainingBytes: objectUrls.available,
    onBusy: (count) => {
      attachBusyCount.value = count
    },
    onError: ({ kind, file }) => {
      preparationErrors += 1
      const message =
        kind === 'count'
          ? i18n.global.t('chat.attachments.errors.tooMany', { count: MAX_ATTACHMENTS })
          : kind === 'memory'
            ? i18n.global.t('chat.attachments.errors.totalTooLarge', {
                size: formatBytes(MAX_LOCAL_ATTACHMENT_MEMORY_BYTES),
              })
            : kind === 'total'
              ? i18n.global.t('chat.attachments.errors.totalTooLarge', {
                  size: formatBytes(MAX_ATTACHMENT_TOTAL_BYTES),
                })
              : kind === 'size'
                ? i18n.global.t('chat.attachments.errors.fileTooLarge', {
                    name: file.name,
                    size: formatBytes(file.size),
                  })
                : i18n.global.t('chat.attachments.errors.failedToReadFile', { name: file.name })
      toasts.push('error', message)
    },
  })
  onScopeDispose(() => {
    ingestion.clear()
    preparingUrls.clear()
    objectUrls.dispose()
  })

  const attachProjectDialogOpen = ref(false)
  const attachProjectPath = ref('')

  function formatBytes(bytes: number): string {
    if (!Number.isFinite(bytes) || bytes <= 0) return '0 B'
    if (bytes < 1024) return `${bytes} B`
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
  }

  function trackPreparation<T>(promise: Promise<T>): Promise<T> {
    preparation.add(promise)
    void promise.finally(() => preparation.delete(promise)).catch(() => undefined)
    return promise
  }

  async function waitForAttachments() {
    while (preparation.size > 0) await Promise.allSettled([...preparation])
  }

  function preparationErrorCount() {
    return preparationErrors
  }
  function attachmentGeneration() {
    return ingestion.generation()
  }

  function attachLocalFiles(files: FileList | File[], pastedText?: { file: File; preview: string; text: string }) {
    const snapshot = Array.from(files)
    const epoch = ingestion.generation()
    const pending = snapshot.map((file) => ({
      id: `pending-${Date.now()}-${Math.random().toString(36).slice(2)}`,
      filename: file.name || 'file',
      size: file.size,
      mime: file.type || 'application/octet-stream',
      state: 'preparing' as const,
      ...(file === pastedText?.file ? { pastePreview: pastedText.preview } : {}),
    }))
    const idsByFile = new Map(snapshot.map((file, index) => [file, pending[index]!.id]))
    pendingAttachments.value = [...pendingAttachments.value, ...pending]
    return trackPreparation(
      ingestion
        .stage(snapshot, {
          idForFile: (file) => idsByFile.get(file) || '',
          shouldSkip: (file) => cancelled.has(idsByFile.get(file) || ''),
        })
        .then((accepted) => {
          if (epoch !== ingestion.generation()) return accepted
          const acceptedIds = new Set(attachedFiles.value.map((file) => file.id))
          for (const item of pending) {
            if (cancelled.has(item.id) && acceptedIds.has(item.id)) {
              attachedFiles.value = attachedFiles.value.filter((file) => file.id !== item.id)
            }
          }
          const pastedId = pastedText ? idsByFile.get(pastedText.file) : undefined
          if (pastedId && attachedFiles.value.some((file) => file.id === pastedId)) {
            attachedFiles.value = attachedFiles.value.map((file) =>
              file.id === pastedId ? { ...file, pastePreview: pastedText!.preview } : file,
            )
          } else if (pastedId && !cancelled.has(pastedId)) {
            // Failed staging returns the original text at the exact paste position.
            if (composerRef.value?.replaceAttachmentWithText)
              composerRef.value.replaceAttachmentWithText(pastedId, pastedText!.text)
            else opts.restoreText?.(pastedText!.text)
          }
          return accepted
        })
        .finally(() => {
          const ids = new Set(pending.map((file) => file.id))
          pendingAttachments.value = pendingAttachments.value.filter((file) => !ids.has(file.id))
          for (const id of ids) cancelled.delete(id)
        }),
    )
  }

  async function handleDrop(e: DragEvent) {
    const files = e.dataTransfer?.files
    if (files?.length) {
      e.preventDefault()
      await attachLocalFiles(files)
    }
  }

  function longPasteTextFile(text: string): File {
    const timestamp = new Date().toISOString().replace(/[:.]/g, '-').replace(/Z$/, '')
    return new File([text], `clipboard-paste-${timestamp}-${Math.random().toString(36).slice(2, 8)}.txt`, {
      type: 'text/plain;charset=utf-8',
      lastModified: Date.now(),
    })
  }

  function pastedTextPreview(text: string): string {
    const compact = text.slice(0, 2048).replace(/\s+/g, ' ').trim()
    if (!compact) return String(i18n.global.t('chat.attachments.whitespaceOnly'))
    const characters = Array.from(compact.slice(0, 1000))
    return characters.length > 240 ? `${characters.slice(0, 240).join('')}…` : compact
  }

  async function handlePaste(e: ClipboardEvent) {
    const data = e.clipboardData
    const files = filesFromClipboard(data)
    const text = data?.getData('text/plain') || ''
    if (!pasteTextWithinBudget(text)) {
      e.preventDefault()
      toasts.push('error', String(i18n.global.t('chat.attachments.errors.textPasteTooLarge')))
      return
    }
    if (
      text.length >= LONG_PASTE_TEXT_CHARS * 2 ||
      Array.from(text.slice(0, LONG_PASTE_TEXT_CHARS * 2)).length >= LONG_PASTE_TEXT_CHARS
    ) {
      e.preventDefault()
      const textFile = longPasteTextFile(text)
      await attachLocalFiles([textFile, ...files], { file: textFile, preview: pastedTextPreview(text), text })
      return
    }
    e.preventDefault()
    if (text) {
      if (composerRef.value?.insertText) composerRef.value.insertText(text)
      else opts.restoreText?.(text)
    }
    if (files.length) await attachLocalFiles(files)
  }

  async function handleFileInputChange(e: Event | FileList) {
    const files = e instanceof FileList ? e : (e.target as HTMLInputElement | null)?.files
    if (!files) return
    await attachLocalFiles(files)

    if (!(e instanceof FileList)) {
      const input = e.target as HTMLInputElement | null
      if (input) input.value = ''
    }
  }

  function removeAttachment(id: string) {
    cancelled.add(id)
    ingestion.cancel(id)
    pendingAttachments.value = pendingAttachments.value.filter((f) => f.id !== id)
    attachedFiles.value = attachedFiles.value.filter((f) => f.id !== id)
  }

  function clearAttachments(options: { preserve?: boolean } = {}) {
    // Preserved drafts are supplied through retainedUrls by the owning page.
    void options
    ingestion.clear()
    pendingAttachments.value = []
    cancelled.clear()
  }

  function openFilePicker() {
    // New UI uses AttachmentPicker (encapsulated hidden input).
    if (composerRef.value?.openFilePicker) {
      composerRef.value.openFilePicker()
    }
  }

  function openProjectAttachDialog() {
    attachProjectPath.value = ''
    attachProjectDialogOpen.value = true
  }

  function basename(path: string): string {
    const p = (path || '').replace(/\\/g, '/').trim()
    if (!p) return 'file'
    const parts = p.split('/').filter(Boolean)
    return parts[parts.length - 1] || p
  }

  function guessMimeFromName(name: string): string {
    const n = (name || '').toLowerCase()
    if (n.endsWith('.png')) return 'image/png'
    if (n.endsWith('.jpg') || n.endsWith('.jpeg')) return 'image/jpeg'
    if (n.endsWith('.gif')) return 'image/gif'
    if (n.endsWith('.webp')) return 'image/webp'
    if (n.endsWith('.svg')) return 'image/svg+xml'
    if (n.endsWith('.pdf')) return 'application/pdf'
    if (n.endsWith('.json')) return 'application/json'
    if (n.endsWith('.md')) return 'text/markdown'
    if (n.endsWith('.txt')) return 'text/plain'
    if (n.endsWith('.ts') || n.endsWith('.tsx')) return 'text/plain'
    if (n.endsWith('.js') || n.endsWith('.jsx')) return 'text/plain'
    if (n.endsWith('.css')) return 'text/plain'
    if (n.endsWith('.html')) return 'text/plain'
    return 'application/octet-stream'
  }

  async function attachProjectFile(path: string) {
    const p = (path || '').trim()
    if (!p) return

    const filename = basename(p)
    if (attachedFiles.value.some((f) => f.serverPath === p)) return
    if (attachedFiles.value.length >= MAX_RESOURCE_ATTACHMENTS) {
      toasts.push('error', i18n.global.t('chat.attachments.errors.tooMany', { count: MAX_RESOURCE_ATTACHMENTS }))
      return
    }

    // Avoid pulling workspace file contents into the browser or message. We send the
    // workspace path as a lazy reference; the model can call fs.read when needed.
    const mime = guessMimeFromName(filename)

    attachedFiles.value = [
      ...attachedFiles.value,
      {
        id: `server-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
        filename,
        size: 0,
        mime,
        url: '',
        serverPath: p,
        delivery: 'reference',
        state: 'ready',
      },
    ]
  }

  async function addProjectAttachment() {
    const p = (attachProjectPath.value || '').trim()
    if (!p) return
    await attachProjectFile(p)
    attachProjectPath.value = ''
  }

  return {
    attachedFiles,
    pendingAttachments,
    attachmentsBusy,
    waitForAttachments,
    preparationErrorCount,
    attachmentGeneration,
    attachProjectDialogOpen,
    attachProjectPath,
    formatBytes,
    handleDrop,
    handlePaste,
    handleFileInputChange,
    removeAttachment,
    clearAttachments,
    releaseUnusedAttachmentUrls,
    openFilePicker,
    openProjectAttachDialog,
    addProjectAttachment,
  }
}
