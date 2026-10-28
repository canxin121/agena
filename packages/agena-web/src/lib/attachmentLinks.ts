import { buildWorkspaceRawFileUrl } from './workspaceLinks'

export type AttachmentLinkInput = {
  url?: string
  path?: string
  unavailable?: boolean
  sessionId?: string | number | null
  workspaceRoot?: string
}

/** Opaque media references are resolved by the server, never by the browser.
 * Session-scoped downloads also understand managed generated images and uploads.
 */
export function attachmentPreviewUrl(input: AttachmentLinkInput): string {
  if (input.unavailable) return ''
  const url = (input.url || '').trim()
  const path = (input.path || '').trim() || (url.startsWith('file://') ? url.slice(7) : '')
  const session = String(input.sessionId ?? '').trim()
  const hasSession = /^[1-9]\d{0,18}$/.test(session)
  const managed = url.startsWith('agena:media:')
  if (managed || path) {
    const reference = managed ? url : path
    if (hasSession) {
      return `/api/v1/sessions/${encodeURIComponent(session)}/media?${new URLSearchParams({ path: reference })}`
    }
    if (!managed && input.workspaceRoot?.trim()) return buildWorkspaceRawFileUrl(input.workspaceRoot.trim(), path)
    return ''
  }
  // Unknown executable/custom schemes must not become browser navigation.
  return /^(?:https?:|data:|blob:)/i.test(url) ? url : ''
}

export function attachmentDownloadUrl(input: AttachmentLinkInput): string {
  const url = attachmentPreviewUrl(input)
  return url.startsWith('/api/v1/sessions/') ? `${url}&download=true` : url
}

/** Open in the file dock only when it really belongs to the currently active
 * workspace. Managed uploads/generated files use the authorized media URL.
 */
export function workspaceAttachmentPath(path: string, ownerRoot: string, currentRoot: string): string | null {
  const normalize = (value: string) => {
    const path = value.trim().replace(/\\/g, '/')
    return path === '/' ? path : path.replace(/\/+$/, '')
  }
  const owner = normalize(ownerRoot)
  const current = normalize(currentRoot)
  const file = normalize(path)
  if (!owner || owner !== current || !file || file.split('/').includes('..')) return null
  const absolute = file.startsWith('/') || /^[a-z]:\//i.test(file)
  if (!absolute) return file
  const prefix = owner.endsWith('/') ? owner : `${owner}/`
  return file.startsWith(prefix) ? file.slice(prefix.length).replace(/^\/+/, '') || null : null
}
