import { apiJson } from './api'

export type SessionWorkingDirectoryAccess = 'inherit' | 'readonly' | 'readwrite'

export type SessionWorkingDirectory = {
  path: string
  access: SessionWorkingDirectoryAccess
}

export type SessionWorkingDirectoriesView = {
  primary: string | null
  directories: string[]
  entries: SessionWorkingDirectory[]
}

export type SessionWorkingDirectoryRegistration = {
  added: boolean
  updated: boolean
  directory: string
  access: SessionWorkingDirectoryAccess
  directories: string[]
  entries: SessionWorkingDirectory[]
}

export type SessionWorkingDirectoryRemoval = {
  removed: boolean
  directory: string
  directories: string[]
  entries: SessionWorkingDirectory[]
}

export type ParsedDirsCommand =
  | { kind: 'list' }
  | { kind: 'add'; path: string; access: SessionWorkingDirectoryAccess }
  | { kind: 'remove'; path: string }
  | { kind: 'invalid' }

export function parseWorkingDirectoryAccessToken(raw: unknown): SessionWorkingDirectoryAccess | null {
  const normalized = String(raw || '')
    .trim()
    .toLowerCase()
  if (normalized === 'inherit' || normalized === 'default') return 'inherit'
  if (normalized === 'readonly' || normalized === 'read_only' || normalized === 'read-only' || normalized === 'ro') {
    return 'readonly'
  }
  if (normalized === 'readwrite' || normalized === 'read_write' || normalized === 'read-write' || normalized === 'rw') {
    return 'readwrite'
  }
  return null
}

export function normalizeWorkingDirectoryAccess(raw: unknown): SessionWorkingDirectoryAccess {
  return parseWorkingDirectoryAccessToken(raw) ?? 'inherit'
}

export function normalizeWorkingDirectoryEntry(raw: unknown): SessionWorkingDirectory | null {
  if (typeof raw === 'string') {
    const path = raw.trim()
    return path ? { path, access: 'inherit' } : null
  }
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return null
  const record = raw as Record<string, unknown>
  const path = typeof record.path === 'string' ? record.path.trim() : ''
  if (!path) return null
  return {
    path,
    access: normalizeWorkingDirectoryAccess(record.access),
  }
}

export function normalizeWorkingDirectoriesView(raw: unknown): SessionWorkingDirectoriesView {
  if (Array.isArray(raw)) {
    const entries = raw
      .map(normalizeWorkingDirectoryEntry)
      .filter((entry): entry is SessionWorkingDirectory => entry !== null)
    return {
      primary: null,
      directories: entries.map((entry) => entry.path),
      entries,
    }
  }
  if (!raw || typeof raw !== 'object') {
    return { primary: null, directories: [], entries: [] }
  }
  const record = raw as Record<string, unknown>
  const primary = typeof record.primary === 'string' && record.primary.trim() ? record.primary.trim() : null
  const rawEntries = Array.isArray(record.entries)
    ? record.entries
    : Array.isArray(record.directories)
      ? record.directories
      : []
  const entries = rawEntries
    .map(normalizeWorkingDirectoryEntry)
    .filter((entry): entry is SessionWorkingDirectory => entry !== null)
  const directories = Array.isArray(record.directories)
    ? record.directories.filter((value): value is string => typeof value === 'string' && value.trim().length > 0)
    : entries.map((entry) => entry.path)
  return { primary, directories, entries }
}

export function splitDirsCommandTokens(raw: string): string[] | null {
  const tokens: string[] = []
  let current = ''
  let inToken = false
  let quote: "'" | '"' | null = null
  let escaped = false

  for (const ch of raw) {
    if (escaped) {
      current += ch
      inToken = true
      escaped = false
      continue
    }
    if (quote !== null) {
      if (ch === '\\' && quote === '"') {
        escaped = true
      } else if (ch === quote) {
        quote = null
      } else {
        current += ch
      }
      inToken = true
      continue
    }
    if (ch === '\\') {
      escaped = true
      inToken = true
    } else if (ch === "'" || ch === '"') {
      quote = ch
      inToken = true
    } else if (/\s/.test(ch)) {
      if (inToken) {
        tokens.push(current)
        current = ''
        inToken = false
      }
    } else {
      current += ch
      inToken = true
    }
  }

  if (escaped || quote !== null) return null
  if (inToken) tokens.push(current)
  return tokens
}

export function parseDirsSlashArgs(raw: string): ParsedDirsCommand {
  const tokens = splitDirsCommandTokens(String(raw || '').trim())
  if (tokens === null) return { kind: 'invalid' }
  if (tokens.length === 0) return { kind: 'list' }

  const head = tokens[0]!.toLowerCase()
  if (head === 'list' || head === 'ls') {
    return tokens.length === 1 ? { kind: 'list' } : { kind: 'invalid' }
  }
  if (head === 'add') {
    if (tokens.length === 2) {
      const path = tokens[1]!.trim()
      return path ? { kind: 'add', path, access: 'inherit' } : { kind: 'invalid' }
    }
    if (tokens.length === 3) {
      const path = tokens[1]!.trim()
      const access = parseWorkingDirectoryAccessToken(tokens[2])
      return path && access ? { kind: 'add', path, access } : { kind: 'invalid' }
    }
    return { kind: 'invalid' }
  }
  if (head === 'remove' || head === 'rm' || head === 'del' || head === 'delete') {
    if (tokens.length !== 2) return { kind: 'invalid' }
    const path = tokens[1]!.trim()
    return path ? { kind: 'remove', path } : { kind: 'invalid' }
  }
  if (tokens.length === 1) {
    const path = tokens[0]!.trim()
    return path ? { kind: 'add', path, access: 'inherit' } : { kind: 'invalid' }
  }
  if (tokens.length === 2) {
    const path = tokens[0]!.trim()
    const access = parseWorkingDirectoryAccessToken(tokens[1])
    return path && access ? { kind: 'add', path, access } : { kind: 'invalid' }
  }
  return { kind: 'invalid' }
}

export async function fetchSessionWorkingDirectories(
  sessionId: string,
  signal?: AbortSignal,
): Promise<SessionWorkingDirectoriesView> {
  const payload = await apiJson<unknown>(
    `/api/v1/sessions/${encodeURIComponent(sessionId)}/working-directories?detailed=true`,
    signal ? { signal } : undefined,
  )
  return normalizeWorkingDirectoriesView(payload)
}

export async function registerSessionWorkingDirectory(
  sessionId: string,
  path: string,
  access: SessionWorkingDirectoryAccess = 'inherit',
): Promise<SessionWorkingDirectoryRegistration> {
  const payload = await apiJson<Record<string, unknown>>(
    `/api/v1/sessions/${encodeURIComponent(sessionId)}/working-directories`,
    {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ path, access }),
    },
  )
  const view = normalizeWorkingDirectoriesView(payload)
  return {
    added: payload.added === true,
    updated: payload.updated === true,
    directory: typeof payload.directory === 'string' && payload.directory ? payload.directory : path,
    access: normalizeWorkingDirectoryAccess(payload.access ?? access),
    directories: view.directories,
    entries: view.entries,
  }
}

export async function removeSessionWorkingDirectory(
  sessionId: string,
  path: string,
): Promise<SessionWorkingDirectoryRemoval> {
  const payload = await apiJson<Record<string, unknown>>(
    `/api/v1/sessions/${encodeURIComponent(sessionId)}/working-directories`,
    {
      method: 'DELETE',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ path }),
    },
  )
  const view = normalizeWorkingDirectoriesView(payload)
  return {
    removed: payload.removed === true,
    directory: typeof payload.directory === 'string' && payload.directory ? payload.directory : path,
    directories: view.directories,
    entries: view.entries,
  }
}
