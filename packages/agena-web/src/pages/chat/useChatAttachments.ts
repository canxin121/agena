import { computed, ref, onScopeDispose, type Ref } from 'vue'
import { i18n } from '@/i18n'
import { createAttachmentIngestor, filesFromClipboard, pasteTextWithinBudget, readLocalDataUrl, MAX_ATTACHMENT_TOTAL_BYTES, MAX_ATTACHMENTS, type StagedAttachment } from './attachmentIngestion'

type ToastKind = 'info' | 'success' | 'error'
type Toasts = { push: (kind: ToastKind, message: string, timeoutMs?: number) => void }

type ComposerExpose = {
  openFilePicker?: () => void
}

export type AttachedFile = StagedAttachment

// Attachment handling (local uploads + project file references).
export function useChatAttachments(opts: { toasts: Toasts; composerRef: Ref<ComposerExpose | null>; restoreText?: (text: string) => void }) {
  const { toasts, composerRef } = opts

  const attachedFiles = ref<AttachedFile[]>([])

  const attachBusyCount = ref(0)
  const attachmentsBusy = computed(() => attachBusyCount.value > 0)

  const MAX_RESOURCE_ATTACHMENTS = MAX_ATTACHMENTS
  const LONG_PASTE_TEXT_CHARS = 1_000
  const ingestion = createAttachmentIngestor({
    get: () => attachedFiles.value,
    set: (files) => { attachedFiles.value = files },
    read: readLocalDataUrl,
    onBusy: (count) => { attachBusyCount.value = count },
    onError: ({ kind, file }) => {
      const message = kind === 'count' ? i18n.global.t('chat.attachments.errors.tooMany', { count: MAX_ATTACHMENTS })
        : kind === 'total' ? i18n.global.t('chat.attachments.errors.totalTooLarge', { size: formatBytes(MAX_ATTACHMENT_TOTAL_BYTES) })
        : kind === 'size' ? i18n.global.t('chat.attachments.errors.fileTooLarge', { name: file.name, size: formatBytes(file.size) })
        : i18n.global.t('chat.attachments.errors.failedToReadFile', { name: file.name })
      toasts.push('error', message)
    },
  })
  onScopeDispose(() => ingestion.clear())

  const attachProjectDialogOpen = ref(false)
  const attachProjectPath = ref('')

  function formatBytes(bytes: number): string {
    if (!Number.isFinite(bytes) || bytes <= 0) return '0 B'
    if (bytes < 1024) return `${bytes} B`
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
  }

  async function attachLocalFiles(files: FileList | File[]) {
    return await ingestion.stage(files)
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
    return new File([text], `clipboard-paste-${timestamp}.txt`, {
      type: 'text/plain;charset=utf-8',
      lastModified: Date.now(),
    })
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
    const longText = Array.from(text).length >= LONG_PASTE_TEXT_CHARS

    if (longText) {
      // Long text is an explicit model-input file, not a lazy path that the
      // model may never read. Preserve the text in the editor if staging fails.
      e.preventDefault()
      const epoch = ingestion.generation()
      const textFile = longPasteTextFile(text)
      const accepted = await attachLocalFiles([textFile, ...files])
      if (epoch === ingestion.generation() && !accepted.includes(textFile)) opts.restoreText?.(text)
      return
    }

    if (files.length) {
      // Do not preventDefault: short clipboard text should still paste normally
      // while image/file items are staged as attachments alongside it.
      await attachLocalFiles(files)
    }
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
    attachedFiles.value = attachedFiles.value.filter((f) => f.id !== id)
  }

  function clearAttachments() {
    ingestion.clear()
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
      toasts.push(
        'error',
        i18n.global.t('chat.attachments.errors.tooMany', { count: MAX_RESOURCE_ATTACHMENTS }),
      )
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
    attachmentsBusy,
    attachProjectDialogOpen,
    attachProjectPath,
    formatBytes,
    handleDrop,
    handlePaste,
    handleFileInputChange,
    removeAttachment,
    clearAttachments,
    openFilePicker,
    openProjectAttachDialog,
    addProjectAttachment,
  }
}
