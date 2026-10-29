import { apiJson, apiFetch } from '@/lib/api'
import type { WorkspacePreviewSession } from '@/features/workspacePreview/api/workspacePreviewApi'
import { serviceRequestUrl, type AppManifest } from './model'

export type AppSnapshot = {
  owner: string
  directory: string
  manifest: AppManifest
  widgetState: unknown
  services: Record<string, WorkspacePreviewSession>
}
export const appApiPath = (owner: string, id: string) =>
  `/api/v1/workbench/apps/${encodeURIComponent(owner)}/${encodeURIComponent(id)}`
export const saveApp = (owner: string, directory: string, manifest: AppManifest, signal?: AbortSignal) =>
  apiJson<AppSnapshot>(appApiPath(owner, manifest.id), {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ directory, manifest }),
    signal,
  })

export async function bridgeFetch(
  snapshot: AppSnapshot | null,
  manifest: AppManifest,
  input: unknown,
  signal: AbortSignal,
) {
  if (!input || typeof input !== 'object') throw new Error('Invalid service request')
  const request = input as Record<string, unknown>
  const serviceName = String(request.service || '')
  if (!manifest.services?.some((s) => s.name === serviceName)) throw new Error('Service was not declared by this app')
  const session = snapshot?.services[serviceName]
  if (!session || !session.pid || !['ready', 'running'].includes(session.state))
    throw new Error('Run the app services first')
  const url = serviceRequestUrl(session.proxyBasePath, String(request.path || ''))
  const method = String(request.method || 'GET').toUpperCase()
  if (!['GET', 'HEAD', 'POST', 'PUT', 'PATCH', 'DELETE', 'OPTIONS'].includes(method))
    throw new Error('Unsupported request method')
  const headers = new Headers()
  if (request.headers && typeof request.headers === 'object') {
    for (const [name, value] of Object.entries(request.headers)) {
      if (!['accept', 'content-type'].includes(name.toLowerCase()) || typeof value !== 'string' || value.length > 200)
        throw new Error('Only Accept and Content-Type headers are supported')
      headers.set(name, value)
    }
  }
  if (
    request.body !== undefined &&
    request.body !== null &&
    (typeof request.body !== 'string' || new TextEncoder().encode(request.body).byteLength > 1024 * 1024)
  )
    throw new Error('Request body must be a string of at most 1 MiB')
  const response = await apiFetch(url, {
    method,
    headers,
    body: method === 'GET' || method === 'HEAD' ? undefined : (request.body as string | undefined),
    signal,
    redirect: 'error',
  })
  let size = 0
  const chunks: Uint8Array[] = []
  const reader = response.body?.getReader()
  if (reader) {
    try {
      for (;;) {
        const { value, done } = await reader.read()
        if (done) break
        size += value.length
        if (size > 1024 * 1024) throw new Error('App response exceeds 1 MiB')
        chunks.push(value)
      }
    } finally {
      await reader.cancel()
    }
  }
  const bytes = new Uint8Array(size)
  let offset = 0
  for (const chunk of chunks) {
    bytes.set(chunk, offset)
    offset += chunk.length
  }
  return {
    status: response.status,
    headers: { 'content-type': response.headers.get('content-type') || 'text/plain' },
    body: new TextDecoder().decode(bytes),
  }
}
