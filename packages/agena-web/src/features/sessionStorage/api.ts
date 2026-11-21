import { apiJson, apiUrl } from '@/lib/api'

export type StorageScope = 'project' | 'bot' | 'deleted'
export type StorageProtection = 'running' | 'shared' | 'referenced' | 'unmanaged' | 'reading'
export type SessionFileKind = 'upload' | 'artifact' | 'cache' | 'plugin'

export interface StorageBytes {
  conversation: number
  recordings: number
  media: number
  files: number
  cache: number
  plugins: number
}

export interface SessionStorageSummary {
  session_id: number
  title: string
  workspace_id: number | null
  workspace_name: string | null
  bot_id: string | null
  bot_name: string | null
  surface: string | null
  deleted: boolean
  busy: boolean
  busy_reason: StorageProtection | null
  modified_at_ms: number
  bytes: StorageBytes
}

export interface DatabaseStorage {
  file_bytes: number
  wal_bytes: number
  reusable_bytes: number
  can_reclaim: boolean
}

export interface StorageBot {
  id: string
  name: string
  session_count: number
  session_bytes: number
  profile_bytes: number
  channel_bytes: number
  channel_files: number
}

export interface SessionStorageList {
  items: SessionStorageSummary[]
  total: number
  measured_at_ms: number
  bytes: number
  bots: StorageBot[]
  database: DatabaseStorage
  shared: {
    workspace_files: number
    web: number
    memory: number
    logs: number
    plugins: number
  }
}

export interface StorageFile {
  id: string
  name: string
  kind: SessionFileKind
  bytes: number
  protection: StorageProtection | null
}

export interface StorageOperation {
  id: string
  state: 'running' | 'completed' | 'failed'
  purge: boolean
  files_removed: number
  files_skipped: number
  file_bytes_removed: number
  database_bytes_removed: number
  keeps_shared_history: boolean
  detail: string | null
}

export interface SessionStorageDetail extends SessionStorageSummary {
  shared_history: boolean
  files: StorageFile[]
  file_count: number
  removable_recording_bytes: number
  operation: StorageOperation | null
}

export interface StorageSelection {
  purge?: boolean
  cache?: boolean
  recordings?: boolean
  files?: string[]
}

export interface StoragePreview {
  token: string
  file_bytes: number
  database_bytes: number
  file_count: number
  keeps_shared_history: boolean
  protection: StorageProtection | null
}

function sessionUrl(id: string, action = '') {
  return apiUrl(`/api/v1/sessions/${encodeURIComponent(id)}/storage${action}`)
}

function post(body: object, signal?: AbortSignal): RequestInit {
  return { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body), signal }
}

export function listSessionStorage(
  query: { scope?: StorageScope; bot?: string; search?: string; offset: number; limit: number; refresh?: boolean },
  signal?: AbortSignal,
) {
  const params = new URLSearchParams()
  for (const [key, value] of Object.entries(query)) {
    if (value !== undefined && value !== '') params.set(key, String(value))
  }
  return apiJson<SessionStorageList>(apiUrl(`/api/v1/session-storage?${params}`), { signal })
}

export function readSessionStorage(id: string, fileOffset: number, fileLimit: number, signal?: AbortSignal) {
  const query = new URLSearchParams({ file_offset: String(fileOffset), file_limit: String(fileLimit) })
  return apiJson<SessionStorageDetail>(sessionUrl(id, `?${query}`), { signal })
}

export function readStorageOperation(id: string, signal?: AbortSignal) {
  return apiJson<StorageOperation | null>(sessionUrl(id, '/operation'), { signal })
}

export function previewSessionStorage(id: string, selection: StorageSelection, signal?: AbortSignal) {
  return apiJson<StoragePreview>(sessionUrl(id, '/preview'), post(selection, signal))
}

export function confirmSessionStorage(id: string, selection: StorageSelection, token: string) {
  return apiJson<StorageOperation>(sessionUrl(id, '/cleanup'), post({ selection, token }))
}

export function reclaimDatabaseStorage() {
  return apiJson<{ database: DatabaseStorage; released_bytes: number }>(
    apiUrl('/api/v1/session-storage/reclaim-database'),
    { method: 'POST' },
  )
}

export function totalStorage(bytes: StorageBytes) {
  return bytes.conversation + bytes.recordings + bytes.media + bytes.files + bytes.cache + bytes.plugins
}
