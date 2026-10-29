import { apiJson, apiUrl } from '@/lib/api'
import { normalizePreviewAccess, previewAccessOrigins, type WorkspacePreviewAccess } from '../model/previewAccess'

/** The bearer-authenticated control API issues a capability; it never authenticates the app itself. */
export async function requestWorkspacePreviewAccess(
  sessionId: string,
  signal?: AbortSignal,
): Promise<WorkspacePreviewAccess> {
  const id = sessionId.trim()
  if (!id) throw new Error('Preview session ID is required.')
  const origins = previewAccessOrigins(apiUrl('/'), window.location.href)
  const response = await apiJson<unknown>(`/api/v1/workbench/preview/sessions/${encodeURIComponent(id)}/access`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(origins),
    signal,
  })
  return normalizePreviewAccess(response, origins)
}
