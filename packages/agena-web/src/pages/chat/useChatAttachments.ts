import { computed, ref, type Ref } from 'vue'
import { i18n } from '@/i18n'

type ToastKind = 'info' | 'success' | 'error'
type Toasts = { push: (kind: ToastKind, message: string, timeoutMs?: number) => void }

type ComposerExpose = {
  openFilePicker?: () => void
}

export type AttachedFile = {
  id: string
  filename: string
  size: number
  mime: string
  url?: string // data: URL (optional for server-side attachments)
  serverPath?: string
}

// Attachment handling (local uploads + project file references).
export function useChatAttachments(opts: { toasts: Toasts; composerRef: Ref<ComposerExpose | null> }) {
  const { toasts, composerRef } = opts

  const attachedFiles = ref<AttachedFile[]>([])

  const attachBusyCount = ref(0)
  const attachmentsBusy = computed(() => attachBusyCount.value > 0)

  // Used to ignore late file reads after the user clears attachments.
  let attachEpoch = 0

  // Match the server/TUI upload contract: 50 MiB per file, 200 MiB per composer batch.
  const MAX_LOCAL_ATTACHMENT_BYTES = 50 * 1024 * 1024
  const MAX_LOCAL_ATTACHMENT_TOTAL_BYTES = 200 * 1024 * 1024
  const MAX_RESOURCE_ATTACHMENTS = 8
  const LONG_PASTE_TEXT_CHARS = 1_000

  const attachProjectDialogOpen = ref(false)
  const attachProjectPath = ref('')

  function formatBytes(bytes: number): string {
    if (!Number.isFinite(bytes) || bytes <= 0) return '0 B'
    if (bytes < 1024) return `${bytes} B`
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
  }

  async function readFileAsDataUrl(file: File): Promise<string> {
    return await new Promise<string>((resolve, reject) => {
      const reader = new FileReader()
      reader.onload = () => resolve(String(reader.result || ''))
      reader.onerror = () =>
        reject(reader.error || new Error(i18n.global.t('chat.attachments.errors.failedToReadUnknown')))
      reader.readAsDataURL(file)
    })
  }

  async function attachLocalFiles(files: FileList | File[]) {
    const epoch = attachEpoch
    attachBusyCount.value += 1
    try {
      const list = Array.from(files)
      let localTotal = attachedFiles.value
        .filter((f) => !f.serverPath)
        .reduce((acc, f) => acc + (Number.isFinite(f.size) ? f.size : 0), 0)

      for (const file of list) {
        if (epoch !== attachEpoch) break
        if (!(file instanceof File)) continue
        if (attachedFiles.value.length >= MAX_RESOURCE_ATTACHMENTS) {
          toasts.push(
            'error',
            i18n.global.t('chat.attachments.errors.tooMany', { count: MAX_RESOURCE_ATTACHMENTS }),
          )
          break
        }

        if (file.size > MAX_LOCAL_ATTACHMENT_BYTES) {
          toasts.push(
            'error',
            i18n.global.t('chat.attachments.errors.fileTooLarge', { name: file.name, size: formatBytes(file.size) }),
          )
          continue
        }
        if (localTotal + file.size > MAX_LOCAL_ATTACHMENT_TOTAL_BYTES) {
          toasts.push(
            'error',
            i18n.global.t('chat.attachments.errors.totalTooLarge', {
              size: formatBytes(MAX_LOCAL_ATTACHMENT_TOTAL_BYTES),
            }),
          )
          continue
        }

        const filename = (file.name || 'file').trim()
        const size = Number(file.size || 0)
        const mime = (file.type || 'application/octet-stream').trim()

        // Basic duplicate check.
        if (attachedFiles.value.some((f) => f.filename === filename && f.size === size)) continue

        let url = ''
        try {
          url = await readFileAsDataUrl(file)
        } catch (err) {
          toasts.push('error', i18n.global.t('chat.attachments.errors.failedToReadFile', { name: filename }))
          continue
        }
        if (epoch !== attachEpoch) break
        if (!url.startsWith('data:')) {
          toasts.push('error', i18n.global.t('chat.attachments.errors.unsupportedFile', { name: filename }))
          continue
        }
        // Another async paste/drop may have filled the remaining slots while this
        // file was being read. Re-check immediately before committing the item.
        if (attachedFiles.value.length >= MAX_RESOURCE_ATTACHMENTS) {
          toasts.push(
            'error',
            i18n.global.t('chat.attachments.errors.tooMany', { count: MAX_RESOURCE_ATTACHMENTS }),
          )
          break
        }

        attachedFiles.value = [
          ...attachedFiles.value,
          {
            id: `file-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
            filename,
            size,
            mime,
            url,
          },
        ]

        localTotal += file.size
      }
    } finally {
      attachBusyCount.value = Math.max(0, attachBusyCount.value - 1)
    }
  }

  async function handleDrop(e: DragEvent) {
    const files = e.dataTransfer?.files
    if (files && files.length) {
      await attachLocalFiles(files)
    }
  }

  function clipboardFiles(data: DataTransfer | null): File[] {
    if (!data) return []

    const files: File[] = []
    const seen = new Set<string>()
    const add = (file: File | null) => {
      if (!file) return
      const key = [file.name, file.size, file.type, file.lastModified].join('\u0000')
      if (seen.has(key)) return
      seen.add(key)
      files.push(file)
    }

    for (const item of Array.from(data.items || [])) {
      if (item.kind === 'file') add(item.getAsFile())
    }
    for (const file of Array.from(data.files || [])) add(file)
    return files
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
    const files = clipboardFiles(data)
    const text = data?.getData('text/plain') || ''
    const longText = Array.from(text).length >= LONG_PASTE_TEXT_CHARS

    if (longText) {
      // Long clipboard text becomes a real file attachment so the submitted
      // message carries only a workspace ref. Suppress the browser's normal
      // textarea insertion; short text still uses native paste unchanged.
      e.preventDefault()
      await attachLocalFiles([longPasteTextFile(text), ...files])
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
    attachEpoch += 1
    attachedFiles.value = []
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
