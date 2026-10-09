import { i18n } from '@/i18n'
import { apiJson } from '@/lib/api'
import { conditionalJson } from '@/lib/conditionalJson'

import { normalizePreviewProxyBasePath } from '../model/previewUrl'

export type WorkspacePreviewSession = {
  id: string
  state: string
  directory: string
  runDirectory: string
  agenaSessionId?: string
  proxyBasePath: string
  targetUrl: string
  command: string
  args: string[]
  logsPath: string
  pid?: number
}

type PreviewSessionsResponse = {
  sessions: unknown
}

export function normalizeWorkspacePreviewSession(value: unknown): WorkspacePreviewSession | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null

  const record = value as Record<string, unknown>
  const id = typeof record.id === 'string' ? record.id.trim() : ''
  if (!id) return null

  const state = typeof record.state === 'string' && record.state.trim() ? record.state.trim() : 'unknown'
  const directory = typeof record.directory === 'string' ? record.directory.trim() : ''
  if (!directory) return null

  const runDirectory =
    typeof record.runDirectory === 'string' && record.runDirectory.trim() ? record.runDirectory.trim() : ''
  if (!runDirectory) return null

  const proxyBasePath = normalizePreviewProxyBasePath(
    typeof record.proxyBasePath === 'string' ? record.proxyBasePath : '',
  )
  const targetUrl = typeof record.targetUrl === 'string' && record.targetUrl.trim() ? record.targetUrl.trim() : ''
  if (!targetUrl) return null

  const command = typeof record.command === 'string' && record.command.trim() ? record.command.trim() : ''
  if (!command) return null

  const args = Array.isArray(record.args) ? record.args.map((v) => String(v || '').trim()).filter(Boolean) : []
  if (args.length === 0) return null

  const logsPath = typeof record.logsPath === 'string' && record.logsPath.trim() ? record.logsPath.trim() : ''
  if (!logsPath) return null

  const pidRaw = typeof record.pid === 'number' ? record.pid : Number(record.pid)
  const pid = Number.isFinite(pidRaw) && pidRaw > 0 ? Math.floor(pidRaw) : undefined

  const agenaSessionId =
    typeof record.agenaSessionId === 'string' && record.agenaSessionId.trim() ? record.agenaSessionId.trim() : undefined

  return {
    id,
    state,
    directory,
    runDirectory,
    ...(agenaSessionId ? { agenaSessionId } : {}),
    proxyBasePath,
    targetUrl,
    command,
    args,
    logsPath,
    ...(pid ? { pid } : {}),
  }
}

function normalizePreviewSessions(payload: PreviewSessionsResponse): WorkspacePreviewSession[] {
  const source = Array.isArray(payload?.sessions) ? payload.sessions : []

  const sessions: WorkspacePreviewSession[] = []
  for (const item of source) {
    const session = normalizeWorkspacePreviewSession(item)
    if (session) sessions.push(session)
  }
  return sessions
}

export async function listWorkspacePreviewSessions(
  signal?: AbortSignal,
  force = false,
): Promise<WorkspacePreviewSession[]> {
  const payload = await conditionalJson<PreviewSessionsResponse>(
    'preview',
    '/api/v1/workbench/preview/sessions',
    { signal },
    force,
  )
  return normalizePreviewSessions(payload)
}

export type WorkspacePreviewSessionCreateInput = {
  id: string
  directory: string
  runDirectory: string
  command: string
  args: string[]
  logsPath: string
  targetUrl: string
  agenaSessionId?: string
}

export async function createWorkspacePreviewSession(
  input: WorkspacePreviewSessionCreateInput,
): Promise<WorkspacePreviewSession> {
  const trimmedId = String(input?.id || '').trim()
  if (!trimmedId) throw new Error(i18n.global.t('errors.preview.sessionIdRequired'))

  const trimmedDirectory = String(input?.directory || '').trim()
  if (!trimmedDirectory) throw new Error(i18n.global.t('errors.preview.directoryRequired'))

  const trimmedRunDirectory = String(input?.runDirectory || '').trim()
  if (!trimmedRunDirectory) throw new Error(i18n.global.t('errors.preview.runDirectoryRequired'))

  const trimmedCommand = String(input?.command || '').trim()
  if (!trimmedCommand) throw new Error(i18n.global.t('errors.preview.commandRequired'))

  const trimmedLogsPath = String(input?.logsPath || '').trim()
  if (!trimmedLogsPath) throw new Error(i18n.global.t('errors.preview.logsPathRequired'))

  const trimmedSessionId = String(input?.agenaSessionId || '').trim()

  const args = Array.isArray(input?.args) ? input.args.map((v) => String(v || '').trim()).filter(Boolean) : []
  if (args.length === 0) throw new Error(i18n.global.t('errors.preview.argsRequired'))

  const targetUrl = String(input?.targetUrl || '').trim()
  if (!targetUrl) throw new Error(i18n.global.t('errors.preview.targetUrlRequired'))

  const payload = await apiJson<unknown>('/api/v1/workbench/preview/sessions', {
    method: 'POST',
    headers: {
      'content-type': 'application/json',
    },
    body: JSON.stringify({
      id: trimmedId,
      directory: trimmedDirectory,
      runDirectory: trimmedRunDirectory,
      command: trimmedCommand,
      args,
      logsPath: trimmedLogsPath,
      ...(trimmedSessionId ? { agenaSessionId: trimmedSessionId } : {}),
      targetUrl,
    }),
  })

  const session = normalizeWorkspacePreviewSession(payload)
  if (!session) throw new Error(i18n.global.t('errors.preview.invalidSessionResponse'))
  return session
}

export async function discoverWorkspacePreviewSession(
  input: Omit<WorkspacePreviewSessionCreateInput, 'targetUrl'>,
): Promise<WorkspacePreviewSession> {
  const trimmedId = String(input?.id || '').trim()
  if (!trimmedId) throw new Error(i18n.global.t('errors.preview.sessionIdRequired'))

  const trimmedDirectory = String(input?.directory || '').trim()
  if (!trimmedDirectory) throw new Error(i18n.global.t('errors.preview.directoryRequired'))

  const trimmedRunDirectory = String(input?.runDirectory || '').trim()
  if (!trimmedRunDirectory) throw new Error(i18n.global.t('errors.preview.runDirectoryRequired'))

  const trimmedCommand = String(input?.command || '').trim()
  if (!trimmedCommand) throw new Error(i18n.global.t('errors.preview.commandRequired'))

  const trimmedLogsPath = String(input?.logsPath || '').trim()
  if (!trimmedLogsPath) throw new Error(i18n.global.t('errors.preview.logsPathRequired'))

  const trimmedSessionId = String(input?.agenaSessionId || '').trim()
  const args = Array.isArray(input?.args) ? input.args.map((v) => String(v || '').trim()).filter(Boolean) : []
  if (args.length === 0) throw new Error(i18n.global.t('errors.preview.argsRequired'))

  const payload = await apiJson<unknown>('/api/v1/workbench/preview/sessions/discover', {
    method: 'POST',
    headers: {
      'content-type': 'application/json',
    },
    body: JSON.stringify({
      id: trimmedId,
      directory: trimmedDirectory,
      runDirectory: trimmedRunDirectory,
      command: trimmedCommand,
      args,
      logsPath: trimmedLogsPath,
      ...(trimmedSessionId ? { agenaSessionId: trimmedSessionId } : {}),
    }),
  })

  const session = normalizeWorkspacePreviewSession(payload)
  if (!session) throw new Error(i18n.global.t('errors.preview.invalidSessionResponse'))
  return session
}

export async function updateWorkspacePreviewSession(
  sessionId: string,
  patch: {
    directory?: string
    runDirectory?: string
    agenaSessionId?: string
    command?: string
    args?: string[]
    logsPath?: string
    targetUrl?: string
  },
): Promise<WorkspacePreviewSession> {
  const trimmedId = String(sessionId || '').trim()
  if (!trimmedId) throw new Error(i18n.global.t('errors.preview.sessionIdRequired'))

  const directory = typeof patch.directory === 'string' ? patch.directory.trim() : ''
  if (typeof patch.directory === 'string' && !directory)
    throw new Error(i18n.global.t('errors.preview.directoryRequired'))

  const runDirectory = typeof patch.runDirectory === 'string' ? patch.runDirectory.trim() : ''
  if (typeof patch.runDirectory === 'string' && !runDirectory)
    throw new Error(i18n.global.t('errors.preview.runDirectoryRequired'))

  const command = typeof patch.command === 'string' ? patch.command.trim() : ''
  if (typeof patch.command === 'string' && !command) throw new Error(i18n.global.t('errors.preview.commandRequired'))

  const logsPath = typeof patch.logsPath === 'string' ? patch.logsPath.trim() : ''
  if (typeof patch.logsPath === 'string' && !logsPath) throw new Error(i18n.global.t('errors.preview.logsPathRequired'))

  const targetUrl = typeof patch.targetUrl === 'string' ? patch.targetUrl.trim() : ''
  if (typeof patch.targetUrl === 'string' && !targetUrl)
    throw new Error(i18n.global.t('errors.preview.targetUrlRequired'))

  const args = Array.isArray(patch.args) ? patch.args.map((v) => String(v || '').trim()).filter(Boolean) : []
  if (Array.isArray(patch.args) && args.length === 0) throw new Error(i18n.global.t('errors.preview.argsRequired'))

  const payload = await apiJson<unknown>(`/api/v1/workbench/preview/sessions/${encodeURIComponent(trimmedId)}`, {
    method: 'PUT',
    headers: {
      'content-type': 'application/json',
    },
    body: JSON.stringify({
      ...(typeof patch.directory === 'string' ? { directory } : {}),
      ...(typeof patch.runDirectory === 'string' ? { runDirectory } : {}),
      ...(typeof patch.agenaSessionId === 'string' ? { agenaSessionId: patch.agenaSessionId.trim() } : {}),
      ...(typeof patch.command === 'string' ? { command } : {}),
      ...(Array.isArray(patch.args) ? { args } : {}),
      ...(typeof patch.logsPath === 'string' ? { logsPath } : {}),
      ...(typeof patch.targetUrl === 'string' ? { targetUrl } : {}),
    }),
  })

  const session = normalizeWorkspacePreviewSession(payload)
  if (!session) throw new Error(i18n.global.t('errors.preview.invalidSessionResponse'))
  return session
}

export async function deleteWorkspacePreviewSession(sessionId: string): Promise<void> {
  const trimmedId = String(sessionId || '').trim()
  if (!trimmedId) throw new Error(i18n.global.t('errors.preview.sessionIdRequired'))

  await apiJson<{ ok: boolean }>(`/api/v1/workbench/preview/sessions/${encodeURIComponent(trimmedId)}`, {
    method: 'DELETE',
  })
}

export async function renameWorkspacePreviewSession(
  sessionId: string,
  newId: string,
): Promise<WorkspacePreviewSession> {
  const trimmedId = String(sessionId || '').trim()
  const trimmedNew = String(newId || '').trim()
  if (!trimmedId) throw new Error(i18n.global.t('errors.preview.sessionIdRequired'))
  if (!trimmedNew) throw new Error(i18n.global.t('errors.preview.newSessionIdRequired'))

  const payload = await apiJson<unknown>(`/api/v1/workbench/preview/sessions/${encodeURIComponent(trimmedId)}/rename`, {
    method: 'POST',
    headers: {
      'content-type': 'application/json',
    },
    body: JSON.stringify({ id: trimmedNew }),
  })

  const session = normalizeWorkspacePreviewSession(payload)
  if (!session) throw new Error(i18n.global.t('errors.preview.invalidSessionResponse'))
  return session
}

export async function startWorkspacePreviewSession(sessionId: string): Promise<WorkspacePreviewSession> {
  const trimmedId = String(sessionId || '').trim()
  if (!trimmedId) throw new Error(i18n.global.t('errors.preview.sessionIdRequired'))

  const payload = await apiJson<unknown>(`/api/v1/workbench/preview/sessions/${encodeURIComponent(trimmedId)}/start`, {
    method: 'POST',
  })

  const session = normalizeWorkspacePreviewSession(payload)
  if (!session) throw new Error(i18n.global.t('errors.preview.invalidSessionResponse'))
  return session
}

export async function stopWorkspacePreviewSession(sessionId: string): Promise<WorkspacePreviewSession> {
  const trimmedId = String(sessionId || '').trim()
  if (!trimmedId) throw new Error(i18n.global.t('errors.preview.sessionIdRequired'))

  const payload = await apiJson<unknown>(`/api/v1/workbench/preview/sessions/${encodeURIComponent(trimmedId)}/stop`, {
    method: 'POST',
  })

  const session = normalizeWorkspacePreviewSession(payload)
  if (!session) throw new Error(i18n.global.t('errors.preview.invalidSessionResponse'))
  return session
}
