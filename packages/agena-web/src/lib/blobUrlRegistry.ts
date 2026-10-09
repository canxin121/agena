import { i18n } from '@/i18n'
export const MAX_LOCAL_ATTACHMENT_MEMORY_BYTES = 128 * 1024 * 1024
let liveBytes = 0

/** The budget is shared across panes and saved composer drafts. It rejects
 * additional staging instead of discarding a user's unsent attachments.
 */
export function createBlobUrlRegistry(
  options: {
    create?: (blob: Blob) => string
    revoke?: (url: string) => void
    maxBytes?: number
  } = {},
) {
  const urls = new Map<string, number>()
  const create = options.create ?? ((blob: Blob) => URL.createObjectURL(blob))
  const revoke = options.revoke ?? ((url: string) => URL.revokeObjectURL(url))
  const available = () => Math.max(0, (options.maxBytes ?? MAX_LOCAL_ATTACHMENT_MEMORY_BYTES) - liveBytes)
  const release = (url: string) => {
    const bytes = urls.get(url)
    if (bytes === undefined) return
    urls.delete(url)
    liveBytes -= bytes
    revoke(url)
  }
  return {
    available,
    create(blob: Blob) {
      if (blob.size > available()) throw new Error(i18n.global.t('errors.attachments.memoryBudgetExceeded'))
      const url = create(blob)
      urls.set(url, blob.size)
      liveBytes += blob.size
      return url
    },
    release,
    releaseUnreferenced(retained: ReadonlySet<string>) {
      for (const url of urls.keys()) if (!retained.has(url)) release(url)
    },
    dispose() {
      for (const url of urls.keys()) release(url)
    },
  }
}
