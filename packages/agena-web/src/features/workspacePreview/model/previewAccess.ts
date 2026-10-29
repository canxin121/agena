export type PreviewAccessOrigins = { backendOrigin: string; workbenchOrigin: string }
export type WorkspacePreviewAccess = { url: string; expiresAt: number; generation: string }

export function previewAccessOrigins(backendUrl: string, workbenchUrl: string): PreviewAccessOrigins {
  const workbench = new URL(workbenchUrl)
  const backend = new URL(backendUrl, workbench)
  if (
    ![workbench, backend].every((url) => ['http:', 'https:'].includes(url.protocol) && !url.username && !url.password)
  ) {
    throw new Error('Preview access requires HTTP(S) backend and workbench origins.')
  }
  return { backendOrigin: backend.origin, workbenchOrigin: workbench.origin }
}

/** Never turn a malformed response or legacy proxy path into a workbench iframe. */
export function normalizePreviewAccess(
  value: unknown,
  origins: PreviewAccessOrigins,
  now = Date.now(),
): WorkspacePreviewAccess {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('Invalid preview access response.')
  const record = value as Record<string, unknown>
  if (typeof record.url !== 'string' || typeof record.generation !== 'string' || !record.generation.trim()) {
    throw new Error('Invalid preview access response.')
  }
  let url: URL
  try {
    url = new URL(record.url)
  } catch {
    throw new Error('Preview access must return an absolute isolated URL.')
  }
  if (
    !['http:', 'https:'].includes(url.protocol) ||
    url.username ||
    url.password ||
    url.pathname !== '/' ||
    url.search ||
    url.hash ||
    !/^p-[a-z0-9-]+\./.test(url.hostname) ||
    url.origin === origins.backendOrigin ||
    url.origin === origins.workbenchOrigin
  ) {
    throw new Error('Preview access must use an isolated preview origin.')
  }
  if (typeof record.expiresAt !== 'number' || !Number.isFinite(record.expiresAt) || record.expiresAt <= now) {
    throw new Error('Preview access has expired. Refresh the preview to authorize it again.')
  }
  return { url: url.href, expiresAt: record.expiresAt, generation: record.generation }
}

export function buildIsolatedPreviewFrameSrc(access: WorkspacePreviewAccess, refreshToken: number): string {
  const url = new URL(access.url)
  url.searchParams.set('__oc_preview_refresh', String(Math.max(0, Math.floor(refreshToken))))
  return url.href
}

/** Match the launch properties that revoke the server's run-bound capability. */
export function previewRunIdentity(
  session: {
    id: string
    instanceId?: string
    directory: string
    runDirectory: string
    agenaSessionId?: string
    targetUrl: string
    command: string
    args: string[]
    pid?: number
    startedAt?: number
    state: string
  } | null,
): string {
  return session
    ? JSON.stringify([
        session.id,
        session.instanceId,
        session.directory,
        session.runDirectory,
        session.agenaSessionId,
        session.targetUrl,
        session.command,
        session.args,
        session.pid,
        session.startedAt,
        session.state,
      ])
    : ''
}
