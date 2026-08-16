import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { i18n } from '@/i18n'

import * as chatApi from '@/stores/chat/api'
import { apiJson } from '@/lib/api'
import { normalizeDirectories } from '@/features/sessions/model/projects'
import type { DirectoryEntry } from '@/features/sessions/model/types'
import { normalizeDirForCompare } from '@/features/sessions/model/labels'
import type { Session } from '@/types/chat'
import type { SseEvent } from '@/lib/sse'
import { defaultChatSidebarUiPrefs, patchChatSidebarUiPrefs, type ChatSidebarUiPrefs } from '@/data/chatSidebarUiPrefs'
import { useChatStore } from './chat'

import {
  normalizeRuntime,
  runtimeIsActive,
  runtimeStateEquivalent,
  type SessionRuntimeState,
} from './directorySessionRuntime'
import type { JsonObject as UnknownRecord, JsonValue } from '@/types/json'

type SidebarFooterKind = 'pinned' | 'recent' | 'running'
// Agena has no opencode-style sidebar endpoints. The sidebar data sources are:
//   GET /api/v1/workspaces        → directory list (WorkspaceResource {id, path})
//   GET /api/v1/sessions/overview → attention/running/recent session buckets
//   GET /api/v1/sessions          → flat session list (search/workspace filters)
// Sidebar command state (collapsed/pinned/expanded/pages) is client-local via
// uiPrefs; the server never persists sidebar chrome.

type AgenaOverviewWire = {
  attention: UnknownRecord[]
  running: UnknownRecord[]
  recent: UnknownRecord[]
}

function agenaSessionId(value: UnknownRecord | null | undefined): string {
  const raw = value?.id
  if (typeof raw === 'number' && Number.isFinite(raw)) return String(raw)
  if (typeof raw === 'string' && raw.trim()) return raw.trim()
  return ''
}

function agenaWorkspaceId(value: UnknownRecord | null | undefined): string {
  const raw = value?.workspace_id
  if (typeof raw === 'number' && Number.isFinite(raw)) return String(raw)
  if (typeof raw === 'string' && raw.trim()) return raw.trim()
  return ''
}

async function fetchAgenaWorkspaces(opts?: {
  limit?: number
  search?: string
  signal?: AbortSignal
}): Promise<{ entries: DirectoryEntry[]; hasMore: boolean }> {
  const limit = Math.max(1, Math.min(1000, Math.floor(opts?.limit || SIDEBAR_DIRECTORIES_PAGE_SIZE)))
  const params = new URLSearchParams()
  params.set('limit', String(limit))
  const search = (opts?.search || '').trim()
  if (search) params.set('search', search)
  const payload = await apiJson<JsonValue>(
    `/api/v1/workspaces?${params.toString()}`,
    opts?.signal ? { signal: opts.signal } : undefined,
  )
  const record = asRecord(payload)
  const items = Array.isArray(record?.items) ? record.items : []
  const entries: DirectoryEntry[] = []
  for (const item of items) {
    const ws = asRecord(item)
    const id = agenaSessionId(ws)
    const path = typeof ws?.path === 'string' ? ws.path.trim() : ''
    if (!id || !path) continue
    entries.push({ id, path })
  }
  const page = asRecord(record?.page)
  return { entries, hasMore: page?.has_more === true }
}

async function fetchAgenaOverview(signal?: AbortSignal): Promise<AgenaOverviewWire> {
  const payload = await apiJson<JsonValue>(
    '/api/v1/sessions/overview',
    signal ? { signal } : undefined,
  )
  const record = asRecord(payload)
  const list = (key: string): UnknownRecord[] => {
    const raw = record?.[key]
    return Array.isArray(raw) ? (raw as UnknownRecord[]) : []
  }
  return { attention: list('attention'), running: list('running'), recent: list('recent') }
}

function toSidebarRowFromAgenaSession(
  record: UnknownRecord | null | undefined,
  directory: DirectoryEntry | null,
): SidebarSessionRow | null {
  const sid = agenaSessionId(record)
  if (!sid) return null
  const session: SidebarSessionSummary = { ...(record as UnknownRecord), id: sid }
  return {
    id: sid,
    session,
    directory,
    renderKey: sid,
    depth: 0,
    parentId: null,
    rootId: sid,
    isParent: false,
    isExpanded: false,
  }
}

function filterOverviewByWorkspace(
  overview: AgenaOverviewWire,
  workspaceId: string,
): { sessions: UnknownRecord[]; running: number; blocked: number } {
  const sessions: UnknownRecord[] = []
  let running = 0
  let blocked = 0
  for (const bucket of [overview.recent, overview.running, overview.attention]) {
    for (const session of bucket) {
      if (agenaWorkspaceId(session) !== workspaceId) continue
      sessions.push(session)
      const state = typeof session.state === 'string' ? session.state.trim() : ''
      if (state === 'running' || state === 'creating') running += 1
      if (state === 'awaiting_user' || state === 'interrupted') blocked += 1
    }
  }
  return { sessions, running, blocked }
}

const SIDEBAR_DIRECTORIES_PAGE_SIZE = 15
const SIDEBAR_FOOTER_PAGE_SIZE = 10
const SIDEBAR_DIRECTORY_SESSIONS_PAGE_SIZE = 10
const SIDEBAR_RECOVERY_THROTTLE_MS = 1500
const SIDEBAR_STATE_REQUEST_STALE_MS = 12000
const SIDEBAR_SESSION_HYDRATION_RETRY_MS = 10000
const SIDEBAR_RECOVERY_EVENT_TYPES = new Set([
  'session.created',
  'session.updated',
  'session.deleted',
  'session.status',
  'session.idle',
  'session.error',
  'permission.asked',
  'permission.replied',
  'question.asked',
  'question.replied',
  'question.rejected',
  'opencode-studio:session-activity',
])

type SidebarSessionSummary = UnknownRecord & {
  id: string
}

type SidebarSessionRow = {
  id: string
  session: SidebarSessionSummary | null
  directory: DirectoryEntry | null
  renderKey: string
  depth: number
  parentId: string | null
  rootId: string
  isParent: boolean
  isExpanded: boolean
}

type DirectorySidebarView = {
  sessionCount: number
  rootPage: number
  rootPageCount: number
  hasActiveOrBlocked: boolean
  hasRunningSessions: boolean
  hasBlockedSessions: boolean
  pinnedRows: SidebarSessionRow[]
  recentRows: SidebarSessionRow[]
  recentParentById: Record<string, string | null>
  recentRootIds: string[]
}

type SidebarFooterView = {
  total: number
  page: number
  pageCount: number
  rows: SidebarSessionRow[]
}

type SidebarFocusedSession = {
  sessionId: string
  directoryId: string
  directoryPath: string
}

type SidebarStateQuery = {
  limitPerDirectory?: number
  directoriesPage?: number
  directoryQuery?: string
  focusSessionId?: string
  pinnedPage?: number
  recentPage?: number
  runningPage?: number
}

type PersistedSidebarStateQuery = Omit<SidebarStateQuery, 'focusSessionId'>

type RevalidateRuntimeOpts = {
  silent?: boolean
}

type SidebarCommandRuntimeOpts = {
  silent?: boolean
}

type SidebarInFlightVoidRequest = {
  key: string
  promise: Promise<void>
  controller: AbortController | null
  startedAt: number
}

type SidebarCommandRequest =
  | { type: 'setDirectoriesPage'; page: number }
  | { type: 'setDirectoryCollapsed'; directoryId: string; collapsed: boolean }
  | { type: 'setDirectoryRootPage'; directoryId: string; page: number }
  | { type: 'setSessionPinned'; sessionId: string; pinned: boolean }
  | { type: 'setSessionExpanded'; sessionId: string; expanded: boolean }
  | { type: 'setFooterOpen'; kind: SidebarFooterKind; open: boolean }
  | { type: 'setFooterPage'; kind: SidebarFooterKind; page: number }

type NormalizedSidebarView = {
  directorySidebarById: Record<string, DirectorySidebarView>
  pinnedFooterView: SidebarFooterView
  recentFooterView: SidebarFooterView
  runningFooterView: SidebarFooterView
}

function asRecord(value: JsonValue): UnknownRecord | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  return value
}

function hasOwn(input: object, key: string): boolean {
  return Object.prototype.hasOwnProperty.call(input, key)
}

function toSessionSummarySnapshot(value: JsonValue): SidebarSessionSummary | null {
  const record = asRecord(value)
  const id = typeof record?.id === 'string' ? record.id.trim() : ''
  if (!record || !id) return null
  return { ...record, id }
}

function toDirectoryEntry(value: JsonValue | null | undefined): DirectoryEntry | null {
  const record = asRecord((value as JsonValue) || undefined)
  if (!record) return null
  const id = typeof record.id === 'string' ? record.id.trim() : ''
  const path = typeof record.path === 'string' ? record.path.trim() : ''
  if (!id || !path) return null
  const label = typeof record.label === 'string' && record.label.trim() ? record.label.trim() : undefined
  return { id, path, ...(label ? { label } : {}) }
}

function readEventType(evt: SseEvent): string {
  return typeof evt.type === 'string' ? evt.type.trim() : ''
}

function createAbortController(): AbortController | null {
  if (typeof AbortController === 'undefined') return null
  return new AbortController()
}

function isAbortError(err: unknown): boolean {
  if (err instanceof DOMException) return err.name === 'AbortError'
  if (!err || typeof err !== 'object') return false
  return (err as { name?: unknown }).name === 'AbortError'
}

function inFlightRequestIsStale(request: SidebarInFlightVoidRequest): boolean {
  return Date.now() - request.startedAt > SIDEBAR_STATE_REQUEST_STALE_MS
}

function normalizeUiPrefs(input: Partial<ChatSidebarUiPrefs> | null | undefined): ChatSidebarUiPrefs {
  return patchChatSidebarUiPrefs(defaultChatSidebarUiPrefs(), (input || {}) as Partial<ChatSidebarUiPrefs>)
}

function isStaleAuthoritativePrefs(
  incomingRaw: Partial<ChatSidebarUiPrefs> | null | undefined,
  localRaw: Partial<ChatSidebarUiPrefs> | null | undefined,
): boolean {
  const incoming = normalizeUiPrefs(incomingRaw)
  const local = normalizeUiPrefs(localRaw)
  if (incoming.version !== local.version) return false
  return incoming.updatedAt < local.updatedAt
}

function normalizePath(path: string): string {
  return normalizeDirForCompare(path)
}

function jsonValueEquivalent(left: JsonValue | undefined, right: JsonValue | undefined): boolean {
  if (Object.is(left, right)) return true
  if (typeof left !== typeof right) return false
  if (left === null || right === null) return left === right

  if (Array.isArray(left) || Array.isArray(right)) {
    if (!Array.isArray(left) || !Array.isArray(right)) return false
    if (left.length !== right.length) return false
    for (let i = 0; i < left.length; i += 1) {
      if (!jsonValueEquivalent(left[i], right[i])) return false
    }
    return true
  }

  if (typeof left === 'object' && typeof right === 'object') {
    const leftRecord = left as UnknownRecord
    const rightRecord = right as UnknownRecord
    const leftKeys = Object.keys(leftRecord)
    const rightKeys = Object.keys(rightRecord)
    if (leftKeys.length !== rightKeys.length) return false
    for (const key of leftKeys) {
      if (!hasOwn(rightRecord, key)) return false
      if (!jsonValueEquivalent(leftRecord[key], rightRecord[key])) return false
    }
    return true
  }

  return false
}

function directoryEntryEquivalent(
  left: DirectoryEntry | null | undefined,
  right: DirectoryEntry | null | undefined,
): boolean {
  if (!left && !right) return true
  if (!left || !right) return false
  return left.id === right.id && left.path === right.path && left.label === right.label
}

function sessionRowEquivalent(
  left: SidebarSessionRow | null | undefined,
  right: SidebarSessionRow | null | undefined,
): boolean {
  if (!left && !right) return true
  if (!left || !right) return false
  return (
    left.id === right.id &&
    left.renderKey === right.renderKey &&
    left.depth === right.depth &&
    left.parentId === right.parentId &&
    left.rootId === right.rootId &&
    left.isParent === right.isParent &&
    left.isExpanded === right.isExpanded &&
    directoryEntryEquivalent(left.directory, right.directory) &&
    jsonValueEquivalent(left.session as JsonValue | undefined, right.session as JsonValue | undefined)
  )
}

function sessionRowsEquivalent(left: SidebarSessionRow[], right: SidebarSessionRow[]): boolean {
  if (left === right) return true
  if (left.length !== right.length) return false
  for (let i = 0; i < left.length; i += 1) {
    if (!sessionRowEquivalent(left[i], right[i])) return false
  }
  return true
}

function stringArraysEquivalent(left: string[], right: string[]): boolean {
  if (left === right) return true
  if (left.length !== right.length) return false
  for (let i = 0; i < left.length; i += 1) {
    if (left[i] !== right[i]) return false
  }
  return true
}

function nullableStringRecordEquivalent(
  left: Record<string, string | null>,
  right: Record<string, string | null>,
): boolean {
  const leftKeys = Object.keys(left)
  const rightKeys = Object.keys(right)
  if (leftKeys.length !== rightKeys.length) return false
  for (const key of leftKeys) {
    if (!hasOwn(right, key)) return false
    if (left[key] !== right[key]) return false
  }
  return true
}

function footerViewEquivalent(left: SidebarFooterView, right: SidebarFooterView): boolean {
  return (
    left.total === right.total &&
    left.page === right.page &&
    left.pageCount === right.pageCount &&
    sessionRowsEquivalent(left.rows, right.rows)
  )
}

function directorySidebarViewEquivalent(left: DirectorySidebarView, right: DirectorySidebarView): boolean {
  return (
    left.sessionCount === right.sessionCount &&
    left.rootPage === right.rootPage &&
    left.rootPageCount === right.rootPageCount &&
    left.hasActiveOrBlocked === right.hasActiveOrBlocked &&
    left.hasRunningSessions === right.hasRunningSessions &&
    left.hasBlockedSessions === right.hasBlockedSessions &&
    sessionRowsEquivalent(left.pinnedRows, right.pinnedRows) &&
    sessionRowsEquivalent(left.recentRows, right.recentRows) &&
    nullableStringRecordEquivalent(left.recentParentById, right.recentParentById) &&
    stringArraysEquivalent(left.recentRootIds, right.recentRootIds)
  )
}

function directorySidebarByIdEquivalent(
  left: Record<string, DirectorySidebarView>,
  right: Record<string, DirectorySidebarView>,
): boolean {
  const leftKeys = Object.keys(left)
  const rightKeys = Object.keys(right)
  if (leftKeys.length !== rightKeys.length) return false
  for (const key of leftKeys) {
    if (!hasOwn(right, key)) return false
    if (!directorySidebarViewEquivalent(left[key], right[key])) return false
  }
  return true
}

function runtimeMapEquivalent(
  left: Record<string, SessionRuntimeState>,
  right: Record<string, SessionRuntimeState>,
): boolean {
  const leftKeys = Object.keys(left)
  const rightKeys = Object.keys(right)
  if (leftKeys.length !== rightKeys.length) return false
  for (const key of leftKeys) {
    if (!hasOwn(right, key)) return false
    if (!runtimeStateEquivalent(left[key], right[key])) return false
    if (left[key].updatedAt !== right[key].updatedAt) return false
  }
  return true
}

function sidebarFocusEquivalent(left: SidebarFocusedSession | null, right: SidebarFocusedSession | null): boolean {
  if (!left && !right) return true
  if (!left || !right) return false
  return (
    left.sessionId === right.sessionId &&
    left.directoryId === right.directoryId &&
    left.directoryPath === right.directoryPath
  )
}

function directoryEntriesEquivalent(left: DirectoryEntry[], right: DirectoryEntry[]): boolean {
  if (left === right) return true
  if (left.length !== right.length) return false
  for (let i = 0; i < left.length; i += 1) {
    if (!directoryEntryEquivalent(left[i], right[i])) return false
  }
  return true
}

function directoryEntriesByIdEquivalent(
  left: Record<string, DirectoryEntry>,
  right: Record<string, DirectoryEntry>,
): boolean {
  const leftKeys = Object.keys(left)
  const rightKeys = Object.keys(right)
  if (leftKeys.length !== rightKeys.length) return false
  for (const key of leftKeys) {
    if (!hasOwn(right, key)) return false
    if (!directoryEntryEquivalent(left[key], right[key])) return false
  }
  return true
}

function directoryEntryByPath(path: string, directoriesById: Record<string, DirectoryEntry>): DirectoryEntry | null {
  const normalized = normalizePath(path)
  if (!normalized) return null
  for (const entry of Object.values(directoriesById)) {
    const id = String(entry?.id || '').trim()
    if (!id) continue
    if (normalizePath(String(entry?.path || '')) === normalized) {
      return entry
    }
  }
  return null
}

function normalizeSidebarSessionRow(raw: JsonValue): SidebarSessionRow | null {
  const record = asRecord(raw)
  if (!record) return null

  const id = typeof record.id === 'string' ? record.id.trim() : ''
  if (!id) return null

  const session = toSessionSummarySnapshot(record.session as JsonValue)
  const wireDirectory = toDirectoryEntry(record.directory as JsonValue)
  const directory = wireDirectory || null

  const depthRaw = Number(record.depth)
  const depth = Number.isFinite(depthRaw) ? Math.max(0, Math.floor(depthRaw)) : 0
  const renderKey = typeof record.renderKey === 'string' && record.renderKey.trim() ? record.renderKey.trim() : id
  const parentRaw = record.parentId ?? record.parentID ?? record.parent_id
  const parentId = typeof parentRaw === 'string' && parentRaw.trim() ? parentRaw.trim() : null
  const rootId = typeof record.rootId === 'string' && record.rootId.trim() ? record.rootId.trim() : id

  return {
    id,
    session,
    directory,
    renderKey,
    depth,
    parentId,
    rootId,
    isParent: record.isParent === true,
    isExpanded: record.isExpanded === true,
  }
}

function normalizeSidebarFooterView(raw: JsonValue | null | undefined): SidebarFooterView {
  const record = asRecord((raw as JsonValue) || undefined)
  const rowsRaw = Array.isArray(record?.rows) ? record.rows : []
  const rows: SidebarSessionRow[] = []
  for (const item of rowsRaw) {
    const row = normalizeSidebarSessionRow(item)
    if (row) rows.push(row)
  }

  const totalRaw = Number(record?.total)
  const pageRaw = Number(record?.page)
  const pageCountRaw = Number(record?.pageCount ?? record?.page_count)
  const total = Number.isFinite(totalRaw) ? Math.max(0, Math.floor(totalRaw)) : rows.length
  const pageCount = Number.isFinite(pageCountRaw) ? Math.max(1, Math.floor(pageCountRaw)) : 1
  const page = Number.isFinite(pageRaw) ? Math.max(0, Math.min(pageCount - 1, Math.floor(pageRaw))) : 0

  return {
    total,
    page,
    pageCount,
    rows,
  }
}

function normalizeDirectorySidebarSection(raw: JsonValue, directoryId: string): DirectorySidebarView | null {
  const section = asRecord(raw)
  if (!section) return null

  const did = String(directoryId || '').trim()
  if (!did) return null

  const pinnedRowsRaw = Array.isArray(section.pinnedRows) ? section.pinnedRows : []
  const recentRowsRaw = Array.isArray(section.recentRows) ? section.recentRows : []

  const pinnedRows: SidebarSessionRow[] = []
  for (const item of pinnedRowsRaw) {
    const row = normalizeSidebarSessionRow(item)
    if (row) pinnedRows.push(row)
  }

  const recentRows: SidebarSessionRow[] = []
  for (const item of recentRowsRaw) {
    const row = normalizeSidebarSessionRow(item)
    if (row) recentRows.push(row)
  }

  const sessionCountRaw = Number(section.sessionCount ?? section.session_count)
  const rootPageRaw = Number(section.rootPage ?? section.root_page)
  const rootPageCountRaw = Number(section.rootPageCount ?? section.root_page_count)

  const sessionCount = Number.isFinite(sessionCountRaw) ? Math.max(0, Math.floor(sessionCountRaw)) : recentRows.length
  const rootPageCount = Number.isFinite(rootPageCountRaw) ? Math.max(1, Math.floor(rootPageCountRaw)) : 1
  const rootPage = Number.isFinite(rootPageRaw) ? Math.max(0, Math.min(rootPageCount - 1, Math.floor(rootPageRaw))) : 0

  const recentParentByIdRaw = asRecord((section.recentParentById ?? section.recent_parent_by_id) as JsonValue) || {}
  const recentParentById: Record<string, string | null> = {}
  for (const [sessionIdRaw, parentRaw] of Object.entries(recentParentByIdRaw)) {
    const sessionId = String(sessionIdRaw || '').trim()
    if (!sessionId) continue
    if (typeof parentRaw === 'string' && parentRaw.trim()) {
      recentParentById[sessionId] = parentRaw.trim()
      continue
    }
    recentParentById[sessionId] = null
  }

  const recentRootIdsRaw = Array.isArray(section.recentRootIds)
    ? section.recentRootIds
    : Array.isArray(section.recent_root_ids)
      ? section.recent_root_ids
      : []
  const recentRootIds = recentRootIdsRaw.map((value) => String(value || '').trim()).filter(Boolean)

  const hasRunningSessions = section.hasRunningSessions === true || section.has_running_sessions === true
  const hasBlockedSessions = section.hasBlockedSessions === true || section.has_blocked_sessions === true
  const hasActiveOrBlocked =
    section.hasActiveOrBlocked === true ||
    section.has_active_or_blocked === true ||
    hasRunningSessions ||
    hasBlockedSessions

  return {
    sessionCount,
    rootPage,
    rootPageCount,
    hasActiveOrBlocked,
    hasRunningSessions,
    hasBlockedSessions,
    pinnedRows,
    recentRows,
    recentParentById,
    recentRootIds,
  }
}

function normalizeDirectoryRowsById(raw: JsonValue | null | undefined): Record<string, DirectorySidebarView> {
  const directoryRowsByIdRaw = asRecord((raw as JsonValue) || undefined) || {}

  const nextDirectorySidebarById: Record<string, DirectorySidebarView> = {}
  for (const [directoryIdRaw, sectionRaw] of Object.entries(directoryRowsByIdRaw)) {
    const directoryId = String(directoryIdRaw || '').trim()
    if (!directoryId) continue
    const section = normalizeDirectorySidebarSection(sectionRaw, directoryId)
    if (!section) continue
    nextDirectorySidebarById[directoryId] = section
  }
  return nextDirectorySidebarById
}

function normalizeSidebarView(raw: JsonValue | null | undefined): NormalizedSidebarView {
  const view = asRecord((raw as JsonValue) || undefined)
  const directoryRowsById = (view?.directoryRowsById ?? view?.directory_rows_by_id) as JsonValue
  const pinnedFooter = (view?.pinnedFooter ?? view?.pinned_footer) as JsonValue
  const recentFooter = (view?.recentFooter ?? view?.recent_footer) as JsonValue
  const runningFooter = (view?.runningFooter ?? view?.running_footer) as JsonValue

  return {
    directorySidebarById: normalizeDirectoryRowsById(directoryRowsById),
    pinnedFooterView: normalizeSidebarFooterView(pinnedFooter),
    recentFooterView: normalizeSidebarFooterView(recentFooter),
    runningFooterView: normalizeSidebarFooterView(runningFooter),
  }
}

function sessionSnapshotHasIdentity(session: SidebarSessionSummary | null | undefined): boolean {
  const title = typeof session?.title === 'string' ? session.title.trim() : ''
  const slug = typeof session?.slug === 'string' ? session.slug.trim() : ''
  return Boolean(title || slug)
}

function sessionSnapshotDirectory(session: SidebarSessionSummary | null | undefined): string {
  const directory = typeof session?.directory === 'string' ? session.directory.trim() : ''
  if (directory) return directory
  const cwd = typeof session?.cwd === 'string' ? session.cwd.trim() : ''
  return cwd
}

function readLocatedSessionId(record: UnknownRecord | null | undefined): string {
  return typeof record?.id === 'string' ? record.id.trim() : ''
}

function mergeSidebarSessionSummary(
  current: SidebarSessionSummary | null | undefined,
  incoming: Partial<Session> | SidebarSessionSummary | null | undefined,
  fallbackId: string,
): SidebarSessionSummary | null {
  const sid = String(fallbackId || '').trim()
  if (!sid) return null
  const currentRecord = current ? { ...current } : null
  const incomingRecord = incoming && typeof incoming === 'object' ? ({ ...incoming } as SidebarSessionSummary) : null

  if (!currentRecord && !incomingRecord) return null

  const merged = {
    ...(currentRecord || {}),
    ...(incomingRecord || {}),
    id: sid,
  } as SidebarSessionSummary

  const title =
    (typeof incomingRecord?.title === 'string' ? incomingRecord.title.trim() : '') ||
    (typeof currentRecord?.title === 'string' ? currentRecord.title.trim() : '')
  const slug =
    (typeof incomingRecord?.slug === 'string' ? incomingRecord.slug.trim() : '') ||
    (typeof currentRecord?.slug === 'string' ? currentRecord.slug.trim() : '')
  const directory = sessionSnapshotDirectory(incomingRecord) || sessionSnapshotDirectory(currentRecord)

  if (title) merged.title = title
  else delete merged.title
  if (slug) merged.slug = slug
  else delete merged.slug
  if (directory) {
    merged.directory = directory
  } else {
    delete merged.directory
  }
  const cwd = typeof merged.cwd === 'string' ? merged.cwd.trim() : ''
  if (cwd) merged.cwd = cwd
  else delete merged.cwd
  return merged
}

function mergeSidebarRowSnapshot(
  row: SidebarSessionRow,
  opts?: { session?: Partial<Session> | SidebarSessionSummary | null; directory?: DirectoryEntry | null },
): SidebarSessionRow {
  const session = mergeSidebarSessionSummary(row.session, opts?.session, row.id)
  const directory = opts?.directory || row.directory

  return {
    ...row,
    session,
    directory: directory || row.directory || null,
  }
}

function sidebarSessionNeedsHydration(row: SidebarSessionRow): boolean {
  if (!row.session) return true
  if (!sessionSnapshotHasIdentity(row.session)) return true
  if (!row.directory && !sessionSnapshotDirectory(row.session)) return true
  return false
}

function applyPersistentStateQueryOverrides(
  base: PersistedSidebarStateQuery,
  opts: SidebarStateQuery | undefined,
): PersistedSidebarStateQuery {
  if (!opts) return base
  const next = { ...base }

  if (hasOwn(opts, 'limitPerDirectory')) {
    const value = Number(opts.limitPerDirectory)
    if (Number.isFinite(value) && value > 0) {
      next.limitPerDirectory = Math.max(1, Math.floor(value))
    } else {
      delete next.limitPerDirectory
    }
  }

  if (hasOwn(opts, 'directoriesPage')) {
    const value = Number(opts.directoriesPage)
    if (Number.isFinite(value)) {
      next.directoriesPage = Math.max(0, Math.floor(value))
    } else {
      delete next.directoriesPage
    }
  }

  if (hasOwn(opts, 'directoryQuery')) {
    const query = typeof opts.directoryQuery === 'string' ? opts.directoryQuery.trim() : ''
    if (query) {
      next.directoryQuery = query
    } else {
      delete next.directoryQuery
    }
  }

  if (hasOwn(opts, 'pinnedPage')) {
    const value = Number(opts.pinnedPage)
    if (Number.isFinite(value)) {
      next.pinnedPage = Math.max(0, Math.floor(value))
    } else {
      delete next.pinnedPage
    }
  }

  if (hasOwn(opts, 'recentPage')) {
    const value = Number(opts.recentPage)
    if (Number.isFinite(value)) {
      next.recentPage = Math.max(0, Math.floor(value))
    } else {
      delete next.recentPage
    }
  }

  if (hasOwn(opts, 'runningPage')) {
    const value = Number(opts.runningPage)
    if (Number.isFinite(value)) {
      next.runningPage = Math.max(0, Math.floor(value))
    } else {
      delete next.runningPage
    }
  }

  return next
}

export const useDirectorySessionStore = defineStore('directorySession', () => {
  const chat = useChatStore()
  const directoriesById = ref<Record<string, DirectoryEntry>>({})
  const directoryOrder = ref<string[]>([])
  const runtimeBySessionId = ref<Record<string, SessionRuntimeState>>({})

  const directorySidebarById = ref<Record<string, DirectorySidebarView>>({})
  const pinnedFooterView = ref<SidebarFooterView>({ total: 0, page: 0, pageCount: 1, rows: [] })
  const recentFooterView = ref<SidebarFooterView>({ total: 0, page: 0, pageCount: 1, rows: [] })
  const runningFooterView = ref<SidebarFooterView>({ total: 0, page: 0, pageCount: 1, rows: [] })
  const sidebarStateFocus = ref<SidebarFocusedSession | null>(null)
  const directoriesPageIndex = ref(0)
  const directoryPageRows = ref<DirectoryEntry[]>([])
  const directoryPageTotal = ref(0)
  const uiPrefs = ref<ChatSidebarUiPrefs>(defaultChatSidebarUiPrefs())

  const loading = ref(false)
  const error = ref<string | null>(null)

  let persistedStateQuery: PersistedSidebarStateQuery = {}

  function syncPersistedPagingQueryFromPrefs(prefsRaw: Partial<ChatSidebarUiPrefs> | null | undefined) {
    const prefs = normalizeUiPrefs(prefsRaw)
    persistedStateQuery = {
      ...persistedStateQuery,
      directoriesPage: Math.max(0, Math.floor(Number(prefs.directoriesPage || 0))),
      pinnedPage: Math.max(0, Math.floor(Number(prefs.pinnedSessionsPage || 0))),
      recentPage: Math.max(0, Math.floor(Number(prefs.recentSessionsPage || 0))),
      runningPage: Math.max(0, Math.floor(Number(prefs.runningSessionsPage || 0))),
    }
  }

  let sidebarStateSyncTimer: number | null = null
  let sidebarStateSyncInFlight = false
  let sidebarStateSyncQueued = false
  let sidebarRecoverySyncTimer: number | null = null
  let lastSidebarRecoverySyncAt = 0
  let sidebarStateRequestInFlight: SidebarInFlightVoidRequest | null = null
  const sidebarSessionHydrationInFlight = new Map<
    string,
    Promise<{ session: Session; directory: DirectoryEntry | null } | null>
  >()
  const sidebarSessionHydrationAttemptAt = new Map<string, number>()
  let sidebarSessionHydrationRunning: Promise<void> | null = null
  let sidebarSessionHydrationQueued = false

  const visibleDirectories = computed<DirectoryEntry[]>(() => {
    return directoryOrder.value
      .map((id) => directoriesById.value[id])
      .filter((entry): entry is DirectoryEntry => Boolean(entry))
  })

  function setDirectoryEntries(entries: DirectoryEntry[]) {
    const nextById: Record<string, DirectoryEntry> = {}
    const order: string[] = []

    for (const entry of entries) {
      const id = String(entry?.id || '').trim()
      const path = String(entry?.path || '').trim()
      if (!id || !path) continue

      const label = typeof entry.label === 'string' && entry.label.trim() ? entry.label.trim() : undefined
      nextById[id] = { id, path, ...(label ? { label } : {}) }
      order.push(id)
    }

    if (!directoryEntriesByIdEquivalent(directoriesById.value, nextById)) {
      directoriesById.value = nextById
    }
    if (!stringArraysEquivalent(directoryOrder.value, order)) {
      directoryOrder.value = order
    }
  }

  function collectLoadedSidebarRows(): SidebarSessionRow[] {
    const rows: SidebarSessionRow[] = []
    for (const section of Object.values(directorySidebarById.value)) {
      rows.push(...section.pinnedRows, ...section.recentRows)
    }
    rows.push(...pinnedFooterView.value.rows, ...recentFooterView.value.rows, ...runningFooterView.value.rows)
    return rows
  }

  function knownSidebarRowBySessionId(): Record<string, SidebarSessionRow> {
    const known: Record<string, SidebarSessionRow> = {}
    for (const row of collectLoadedSidebarRows()) {
      const sid = String(row.id || '').trim()
      if (!sid) continue
      const previous = known[sid]
      if (!previous) {
        known[sid] = row
        continue
      }
      const merged = mergeSidebarRowSnapshot(previous, {
        session: row.session,
        directory: row.directory || previous.directory,
      })
      known[sid] = merged
    }
    return known
  }

  function knownDirectoryForSession(row: SidebarSessionRow): DirectoryEntry | null {
    if (row.directory?.id && row.directory.path) return row.directory
    const sessionPath = sessionSnapshotDirectory(row.session)
    if (!sessionPath) return null
    return directoryEntryByPath(sessionPath, directoriesById.value)
  }

  function enrichSidebarRowWithKnownData(
    row: SidebarSessionRow,
    knownRows: Record<string, SidebarSessionRow>,
  ): SidebarSessionRow {
    const sid = String(row.id || '').trim()
    if (!sid) return row
    const cachedSession = chat.getSessionById(sid)
    const knownRow = knownRows[sid]

    // Prefer sidebar/server snapshots over chat cache when both exist.
    // Cache can be temporarily stale/missing directory during cross-window SSE races.
    const preferredSession = mergeSidebarSessionSummary(
      cachedSession || null,
      row.session || knownRow?.session || null,
      sid,
    )

    const directory =
      row.directory ||
      knownRow?.directory ||
      knownDirectoryForSession(row) ||
      (knownRow ? knownDirectoryForSession(knownRow) : null)

    return mergeSidebarRowSnapshot(row, {
      session: preferredSession,
      directory,
    })
  }

  function enrichDirectorySidebarView(
    section: DirectorySidebarView,
    knownRows: Record<string, SidebarSessionRow>,
  ): DirectorySidebarView {
    return {
      ...section,
      pinnedRows: section.pinnedRows.map((row) => enrichSidebarRowWithKnownData(row, knownRows)),
      recentRows: section.recentRows.map((row) => enrichSidebarRowWithKnownData(row, knownRows)),
    }
  }

  function enrichFooterView(view: SidebarFooterView, knownRows: Record<string, SidebarSessionRow>): SidebarFooterView {
    return {
      ...view,
      rows: view.rows.map((row) => enrichSidebarRowWithKnownData(row, knownRows)),
    }
  }

  function applyHydratedSidebarSessions(
    entries: Map<string, { session: Session; directory: DirectoryEntry | null }>,
  ): boolean {
    if (entries.size === 0) return false
    let changed = false

    const nextDirectorySidebarById: Record<string, DirectorySidebarView> = {}
    for (const [directoryId, section] of Object.entries(directorySidebarById.value)) {
      const nextSection: DirectorySidebarView = {
        ...section,
        pinnedRows: section.pinnedRows.map((row) => {
          const hydrated = entries.get(row.id)
          if (!hydrated) return row
          const nextRow = mergeSidebarRowSnapshot(row, hydrated)
          if (!sessionRowEquivalent(row, nextRow)) changed = true
          return nextRow
        }),
        recentRows: section.recentRows.map((row) => {
          const hydrated = entries.get(row.id)
          if (!hydrated) return row
          const nextRow = mergeSidebarRowSnapshot(row, hydrated)
          if (!sessionRowEquivalent(row, nextRow)) changed = true
          return nextRow
        }),
      }
      nextDirectorySidebarById[directoryId] = nextSection
    }

    const mergeFooterRows = (rows: SidebarSessionRow[]): SidebarSessionRow[] =>
      rows.map((row) => {
        const hydrated = entries.get(row.id)
        if (!hydrated) return row
        const nextRow = mergeSidebarRowSnapshot(row, hydrated)
        if (!sessionRowEquivalent(row, nextRow)) changed = true
        return nextRow
      })

    const nextPinnedFooterView = { ...pinnedFooterView.value, rows: mergeFooterRows(pinnedFooterView.value.rows) }
    const nextRecentFooterView = { ...recentFooterView.value, rows: mergeFooterRows(recentFooterView.value.rows) }
    const nextRunningFooterView = { ...runningFooterView.value, rows: mergeFooterRows(runningFooterView.value.rows) }

    if (changed) {
      directorySidebarById.value = nextDirectorySidebarById
      pinnedFooterView.value = nextPinnedFooterView
      recentFooterView.value = nextRecentFooterView
      runningFooterView.value = nextRunningFooterView
    }

    return changed
  }

  async function hydrateSessionViaLocate(
    sessionId: string,
    hint?: { directoryId?: string; directoryPath?: string },
  ): Promise<{ session: Session; directory: DirectoryEntry | null } | null> {
    const sid = String(sessionId || '').trim()
    if (!sid) return null
    // Agena has no project/directory concept on sessions: read the session
    // directly and fall back to the caller's directory hint (workspace match
    // is attempted when the session carries a numeric workspace_id).
    const located = asRecord(await chatApi.getSession(sid).catch(() => null))
    const rawSession = located
    const locatedSessionId = readLocatedSessionId(rawSession)
    if (!rawSession || !locatedSessionId || locatedSessionId !== sid) return null
    const directory =
      resolveDirectoryEntryForSessionSnapshot(rawSession, {
        directoryId:
          typeof located?.workspace_id === 'number' ? String(located.workspace_id) : hint?.directoryId,
        directoryPath: hint?.directoryPath,
      }) || null
    return {
      session: rawSession as Session,
      directory,
    }
  }

  function resolveDirectoryEntryForSessionSnapshot(
    session: Partial<Session> | SidebarSessionSummary | null | undefined,
    hint?: { directoryId?: string; directoryPath?: string },
  ): DirectoryEntry | null {
    const sessionDirectory = sessionSnapshotDirectory(session as SidebarSessionSummary | null)
    const hintId = String(hint?.directoryId || '').trim()
    const hintPath = String(hint?.directoryPath || '').trim()
    if (hintId) {
      const byId = directoriesById.value[hintId]
      if (byId?.path) return byId
    }
    if (sessionDirectory) {
      const byPath = directoryEntryByPath(sessionDirectory, directoriesById.value)
      if (byPath?.id && byPath.path) return byPath
    }
    if (hintPath) {
      const byPath = directoryEntryByPath(hintPath, directoriesById.value)
      if (byPath?.id && byPath.path) return byPath
      if (hintId) return { id: hintId, path: hintPath }
    }
    return null
  }

  async function hydrateIncompleteSidebarSessions(): Promise<void> {
    const knownRows = knownSidebarRowBySessionId()
    const candidateIds = new Set<string>()
    const groupedByDirectory = new Map<string, Set<string>>()
    const hintsBySessionId = new Map<string, { directoryId?: string; directoryPath?: string }>()
    const now = Date.now()

    for (const row of Object.values(knownRows)) {
      if (!sidebarSessionNeedsHydration(row)) continue
      const sid = String(row.id || '').trim()
      if (!sid) continue
      if (sidebarSessionHydrationInFlight.has(sid)) continue
      const lastAttempt = sidebarSessionHydrationAttemptAt.get(sid) || 0
      if (now - lastAttempt < SIDEBAR_SESSION_HYDRATION_RETRY_MS) continue
      candidateIds.add(sid)

      const directory = knownDirectoryForSession(row)
      const directoryPath = directory?.path || sessionSnapshotDirectory(row.session)
      if (directory?.id || directoryPath) {
        hintsBySessionId.set(sid, {
          directoryId: directory?.id || undefined,
          directoryPath: directoryPath || undefined,
        })
      }
      if (directoryPath) {
        const key = directoryPath.trim()
        const bucket = groupedByDirectory.get(key) || new Set<string>()
        bucket.add(sid)
        groupedByDirectory.set(key, bucket)
      }
      sidebarSessionHydrationAttemptAt.set(sid, now)
    }

    if (candidateIds.size === 0) return

    const hydrated = new Map<string, { session: Session; directory: DirectoryEntry | null }>()
    const unresolved = new Set<string>(candidateIds)

    const directoryTasks = [...groupedByDirectory.entries()].map(async ([directoryPath, sessionIds]) => {
      const ids = [...sessionIds]
      if (ids.length === 0) return
      // Agena has no by-id session listing; hydrate each candidate directly.
      const hydratedSessions = (
        await Promise.all(ids.map((id) => chatApi.getSession(id).catch(() => null)))
      ).filter((s): s is Session => Boolean(s))
      for (const session of hydratedSessions) {
        const sid = typeof session?.id === 'string' ? session.id.trim() : ''
        if (!sid || !candidateIds.has(sid)) continue
        unresolved.delete(sid)
        hydrated.set(sid, {
          session,
          directory: resolveDirectoryEntryForSessionSnapshot(session, {
            directoryPath,
            directoryId: hintsBySessionId.get(sid)?.directoryId,
          }),
        })
      }
    })

    await Promise.allSettled(directoryTasks)

    const locateTargets = [...unresolved]
    const locateTasks = locateTargets.map((sid) => {
      let task = sidebarSessionHydrationInFlight.get(sid)
      if (!task) {
        task = hydrateSessionViaLocate(sid, hintsBySessionId.get(sid)).finally(() => {
          sidebarSessionHydrationInFlight.delete(sid)
        })
        sidebarSessionHydrationInFlight.set(sid, task)
      }
      return task.then((result) => {
        if (!result?.session) return
        unresolved.delete(sid)
        hydrated.set(sid, {
          session: result.session,
          directory:
            result.directory || resolveDirectoryEntryForSessionSnapshot(result.session, hintsBySessionId.get(sid)),
        })
      })
    })

    await Promise.allSettled(locateTasks)

    if (hydrated.size === 0) {
      for (const sid of unresolved) {
        sidebarSessionHydrationAttemptAt.set(
          sid,
          Date.now() - SIDEBAR_SESSION_HYDRATION_RETRY_MS + SIDEBAR_RECOVERY_THROTTLE_MS,
        )
      }
      scheduleSidebarRecoverySync('sidebar-session-hydration-missed', 220)
      return
    }

    chat.cacheSessions([...hydrated.values()].map((entry) => entry.session))
    applyHydratedSidebarSessions(hydrated)
    if (unresolved.size > 0) {
      for (const sid of unresolved) {
        sidebarSessionHydrationAttemptAt.set(
          sid,
          Date.now() - SIDEBAR_SESSION_HYDRATION_RETRY_MS + SIDEBAR_RECOVERY_THROTTLE_MS,
        )
      }
      scheduleSidebarRecoverySync('sidebar-session-hydration-partial', 220)
    }
  }

  function scheduleSidebarSessionHydration() {
    if (sidebarSessionHydrationRunning) {
      sidebarSessionHydrationQueued = true
      return
    }

    sidebarSessionHydrationRunning = (async () => {
      try {
        await hydrateIncompleteSidebarSessions()
      } finally {
        sidebarSessionHydrationRunning = null
        if (sidebarSessionHydrationQueued) {
          sidebarSessionHydrationQueued = false
          scheduleSidebarSessionHydration()
        }
      }
    })()
  }

  function applyAuthoritativeUiPrefs(incomingRaw: Partial<ChatSidebarUiPrefs> | null | undefined): boolean {
    if (isStaleAuthoritativePrefs(incomingRaw, uiPrefs.value)) {
      return false
    }
    const next = normalizeUiPrefs(incomingRaw)
    syncPersistedPagingQueryFromPrefs(next)
    if (jsonValueEquivalent(uiPrefs.value as JsonValue, next as JsonValue)) {
      return false
    }
    uiPrefs.value = next
    return true
  }


  async function executeSidebarCommand(
    command: SidebarCommandRequest,
    opts?: SidebarCommandRuntimeOpts,
  ): Promise<boolean> {
    if (!opts?.silent) {
      loading.value = true
      error.value = null
    }

    try {
      // Agena has no sidebar command endpoint; commands mutate the client-local
      // uiPrefs directly (collapsed/pinned/expanded/page prefs live in localStorage).
      const patch: Partial<ChatSidebarUiPrefs> = {}
      if (command.type === 'setDirectoriesPage') {
        patch.directoriesPage = Math.max(0, Math.floor(Number(command.page || 0)))
      } else if (command.type === 'setDirectoryCollapsed') {
        const current = new Set(uiPrefs.value.collapsedDirectoryIds || [])
        if (command.collapsed) current.add(command.directoryId)
        else current.delete(command.directoryId)
        patch.collapsedDirectoryIds = [...current]
      } else if (command.type === 'setDirectoryRootPage') {
        patch.sessionRootPageByDirectoryId = {
          ...(uiPrefs.value.sessionRootPageByDirectoryId || {}),
          [command.directoryId]: Math.max(0, Math.floor(Number(command.page || 0))),
        }
      } else if (command.type === 'setSessionPinned') {
        const current = new Set(uiPrefs.value.pinnedSessionIds || [])
        if (command.pinned) current.add(command.sessionId)
        else current.delete(command.sessionId)
        patch.pinnedSessionIds = [...current]
      } else if (command.type === 'setSessionExpanded') {
        const current = new Set(uiPrefs.value.expandedParentSessionIds || [])
        if (command.expanded) current.add(command.sessionId)
        else current.delete(command.sessionId)
        patch.expandedParentSessionIds = [...current]
      } else if (command.type === 'setFooterOpen') {
        if (command.kind === 'pinned') patch.pinnedSessionsOpen = command.open
        else if (command.kind === 'recent') patch.recentSessionsOpen = command.open
        else patch.runningSessionsOpen = command.open
      } else if (command.type === 'setFooterPage') {
        const target = Math.max(0, Math.floor(Number(command.page || 0)))
        if (command.kind === 'pinned') patch.pinnedSessionsPage = target
        else if (command.kind === 'recent') patch.recentSessionsPage = target
        else patch.runningSessionsPage = target
      }

      const next = normalizeUiPrefs(patchChatSidebarUiPrefs(uiPrefs.value, patch))
      applyAuthoritativeUiPrefs(next)
      return true
    } catch (err) {
      if (!opts?.silent) {
        error.value = err instanceof Error ? err.message : String(err)
      }
      return false
    } finally {
      if (!opts?.silent) {
        loading.value = false
      }
    }
  }

  async function commandSetDirectoriesPage(page: number, opts?: { silent?: boolean }): Promise<boolean> {
    const target = Math.max(0, Math.floor(Number(page || 0)))
    persistedStateQuery = {
      ...persistedStateQuery,
      directoriesPage: target,
    }
    return executeSidebarCommand({ type: 'setDirectoriesPage', page: target }, opts)
  }

  async function commandSetDirectoryCollapsed(
    directoryId: string,
    collapsed: boolean,
    opts?: { silent?: boolean },
  ): Promise<boolean> {
    const did = String(directoryId || '').trim()
    if (!did) return false
    const ok = await executeSidebarCommand({ type: 'setDirectoryCollapsed', directoryId: did, collapsed }, opts)
    return ok
  }

  async function commandSetDirectoryRootPage(
    directoryId: string,
    page: number,
    opts?: { silent?: boolean },
  ): Promise<boolean> {
    const did = String(directoryId || '').trim()
    if (!did) return false
    const target = Math.max(0, Math.floor(Number(page || 0)))
    return executeSidebarCommand({ type: 'setDirectoryRootPage', directoryId: did, page: target }, opts)
  }

  async function commandSetSessionPinned(
    sessionId: string,
    pinned: boolean,
    opts?: { silent?: boolean },
  ): Promise<boolean> {
    const sid = String(sessionId || '').trim()
    if (!sid) return false
    return executeSidebarCommand({ type: 'setSessionPinned', sessionId: sid, pinned }, opts)
  }

  async function commandSetSessionExpanded(
    sessionId: string,
    expanded: boolean,
    opts?: { silent?: boolean },
  ): Promise<boolean> {
    const sid = String(sessionId || '').trim()
    if (!sid) return false
    return executeSidebarCommand({ type: 'setSessionExpanded', sessionId: sid, expanded }, opts)
  }

  async function commandSetFooterOpen(
    kind: SidebarFooterKind,
    open: boolean,
    opts?: { silent?: boolean },
  ): Promise<boolean> {
    return executeSidebarCommand({ type: 'setFooterOpen', kind, open }, opts)
  }

  async function commandSetFooterPage(
    kind: SidebarFooterKind,
    page: number,
    opts?: { silent?: boolean },
  ): Promise<boolean> {
    const target = Math.max(0, Math.floor(Number(page || 0)))
    persistedStateQuery = {
      ...persistedStateQuery,
      ...(kind === 'pinned'
        ? { pinnedPage: target }
        : kind === 'recent'
          ? { recentPage: target }
          : { runningPage: target }),
    }
    return executeSidebarCommand({ type: 'setFooterPage', kind, page: target }, opts)
  }

  function parseRuntimeMap(raw: JsonValue): Record<string, SessionRuntimeState> {
    const runtimePayload = asRecord(raw) || {}
    const next: Record<string, SessionRuntimeState> = {}
    for (const [sessionIdRaw, runtimeRaw] of Object.entries(runtimePayload)) {
      const sessionId = String(sessionIdRaw || '').trim()
      if (!sessionId) continue
      next[sessionId] = normalizeRuntime((asRecord(runtimeRaw) as Partial<SessionRuntimeState>) || undefined)
    }
    return next
  }

  function applyDirectoriesPagePayload(directoriesPageRaw: JsonValue): DirectoryEntry[] {
    const directoriesPage = asRecord(directoriesPageRaw)
    const entries = normalizeDirectories((directoriesPage?.items as JsonValue) || [])
    setDirectoryEntries(entries)

    if (!directoryEntriesEquivalent(directoryPageRows.value, entries)) {
      directoryPageRows.value = entries
    }
    const nextDirectoryPageTotal =
      typeof directoriesPage?.total === 'number' && Number.isFinite(directoriesPage.total)
        ? Math.max(0, Math.floor(directoriesPage.total))
        : entries.length
    if (directoryPageTotal.value !== nextDirectoryPageTotal) {
      directoryPageTotal.value = nextDirectoryPageTotal
    }

    const offsetRaw = Number(directoriesPage?.offset)
    const limitRaw = Number(directoriesPage?.limit)
    const offset = Number.isFinite(offsetRaw) ? Math.max(0, Math.floor(offsetRaw)) : 0
    const limit = Number.isFinite(limitRaw) && limitRaw > 0 ? Math.max(1, Math.floor(limitRaw)) : 1
    const nextPageIndex = Math.max(0, Math.floor(offset / limit))
    if (directoriesPageIndex.value !== nextPageIndex) {
      directoriesPageIndex.value = nextPageIndex
    }

    return entries
  }

  function applySidebarStatePayload(stateRaw: JsonValue) {
    const stateRecord = asRecord(stateRaw) || {}
    if (!hasOwn(stateRecord, 'preferences')) {
      throw new Error('chat sidebar state payload is missing preferences')
    }
    applyAuthoritativeUiPrefs((stateRecord.preferences as Partial<ChatSidebarUiPrefs>) || undefined)

    applyDirectoriesPagePayload((stateRecord.directoriesPage ?? stateRecord.directories_page) as JsonValue)

    const knownRows = knownSidebarRowBySessionId()
    const normalizedViewRaw = normalizeSidebarView(stateRecord.view as JsonValue)
    const normalizedView: NormalizedSidebarView = {
      directorySidebarById: Object.fromEntries(
        Object.entries(normalizedViewRaw.directorySidebarById).map(([directoryId, section]) => [
          directoryId,
          enrichDirectorySidebarView(section, knownRows),
        ]),
      ),
      pinnedFooterView: enrichFooterView(normalizedViewRaw.pinnedFooterView, knownRows),
      recentFooterView: enrichFooterView(normalizedViewRaw.recentFooterView, knownRows),
      runningFooterView: enrichFooterView(normalizedViewRaw.runningFooterView, knownRows),
    }

    const nextRuntimeBySessionId = parseRuntimeMap(
      (stateRecord.runtimeBySessionId ?? stateRecord.runtime_by_session_id) as JsonValue,
    )
    if (!runtimeMapEquivalent(runtimeBySessionId.value, nextRuntimeBySessionId)) {
      runtimeBySessionId.value = nextRuntimeBySessionId
    }
    if (!directorySidebarByIdEquivalent(directorySidebarById.value, normalizedView.directorySidebarById)) {
      directorySidebarById.value = normalizedView.directorySidebarById
    }
    if (!footerViewEquivalent(pinnedFooterView.value, normalizedView.pinnedFooterView)) {
      pinnedFooterView.value = normalizedView.pinnedFooterView
    }
    if (!footerViewEquivalent(recentFooterView.value, normalizedView.recentFooterView)) {
      recentFooterView.value = normalizedView.recentFooterView
    }
    if (!footerViewEquivalent(runningFooterView.value, normalizedView.runningFooterView)) {
      runningFooterView.value = normalizedView.runningFooterView
    }

    const focusRecord = asRecord(stateRecord.focus as JsonValue)
    const focusSid =
      typeof focusRecord?.sessionId === 'string'
        ? focusRecord.sessionId.trim()
        : typeof focusRecord?.session_id === 'string'
          ? focusRecord.session_id.trim()
          : ''
    const focusDid =
      typeof focusRecord?.directoryId === 'string'
        ? focusRecord.directoryId.trim()
        : typeof focusRecord?.directory_id === 'string'
          ? focusRecord.directory_id.trim()
          : ''
    const focusPath =
      typeof focusRecord?.directoryPath === 'string'
        ? focusRecord.directoryPath.trim()
        : typeof focusRecord?.directory_path === 'string'
          ? focusRecord.directory_path.trim()
          : ''
    const nextFocus =
      focusSid && focusDid && focusPath
        ? {
            sessionId: focusSid,
            directoryId: focusDid,
            directoryPath: focusPath,
          }
        : null
    if (!sidebarFocusEquivalent(sidebarStateFocus.value, nextFocus)) {
      sidebarStateFocus.value = nextFocus
    }

    scheduleSidebarSessionHydration()
  }

  async function buildAgenaSidebarPayload(signal?: AbortSignal): Promise<JsonValue> {
    const page = Math.max(0, Math.floor(Number(persistedStateQuery.directoriesPage || 0)))
    const pageSize = SIDEBAR_DIRECTORIES_PAGE_SIZE
    const query = (persistedStateQuery.directoryQuery || '').trim()
    const [workspaces, overview] = await Promise.all([
      fetchAgenaWorkspaces({ limit: pageSize, search: query || undefined, signal }),
      fetchAgenaOverview(signal),
    ])

    const pinnedIds = new Set(uiPrefs.value.pinnedSessionIds || [])
    const directoryRowsById: Record<string, JsonValue> = {}
    for (const entry of workspaces.entries) {
      const { sessions, running, blocked } = filterOverviewByWorkspace(overview, entry.id)
      const pinnedRows: SidebarSessionRow[] = []
      const recentRows: SidebarSessionRow[] = []
      for (const session of sessions) {
        const row = toSidebarRowFromAgenaSession(session, entry)
        if (!row) continue
        if (pinnedIds.has(row.id)) pinnedRows.push(row)
        recentRows.push(row)
      }
      const sessionCount = sessions.length
      const rootPageCount = Math.max(1, Math.ceil(sessionCount / SIDEBAR_DIRECTORY_SESSIONS_PAGE_SIZE))
      const rootPage = Math.min(
        Math.max(0, Math.floor(Number(uiPrefs.value.sessionRootPageByDirectoryId[entry.id] || 0))),
        rootPageCount - 1,
      )
      const hasRunning = running > 0
      const hasBlocked = blocked > 0
      directoryRowsById[entry.id] = {
        sessionCount,
        rootPage,
        rootPageCount,
        hasActiveOrBlocked: hasRunning || hasBlocked,
        hasRunningSessions: hasRunning,
        hasBlockedSessions: hasBlocked,
        pinnedRows: pinnedRows as unknown as JsonValue,
        recentRows: recentRows as unknown as JsonValue,
        recentParentById: {},
        recentRootIds: recentRows.map((row) => row.id),
      }
    }

    const pinnedRows: SidebarSessionRow[] = []
    const allOverview = [...overview.recent, ...overview.running, ...overview.attention]
    for (const session of allOverview) {
      const sid = agenaSessionId(session)
      if (!pinnedIds.has(sid)) continue
      const row = toSidebarRowFromAgenaSession(session, null)
      if (row) pinnedRows.push(row)
    }
    const footerRows = (sessions: UnknownRecord[]): JsonValue =>
      sessions
        .map((s) => toSidebarRowFromAgenaSession(s, null))
        .filter((r): r is SidebarSessionRow => Boolean(r)) as unknown as JsonValue

    return {
      preferences: uiPrefs.value,
      directoriesPage: {
        items: workspaces.entries.map((entry) => ({ id: entry.id, path: entry.path })),
        total: workspaces.hasMore
          ? page * pageSize + workspaces.entries.length + 1
          : workspaces.entries.length,
        offset: page * pageSize,
        limit: pageSize,
      },
      view: {
        directoryRowsById,
        pinnedFooter: { total: pinnedRows.length, page: 0, pageCount: 1, rows: pinnedRows as unknown as JsonValue },
        recentFooter: { total: overview.recent.length, page: 0, pageCount: 1, rows: footerRows(overview.recent) },
        runningFooter: { total: overview.running.length, page: 0, pageCount: 1, rows: footerRows(overview.running) },
      },
      runtimeBySessionId: {},
      focus: null,
    }
  }

  async function revalidateFromStateApi(opts?: SidebarStateQuery): Promise<void> {
    persistedStateQuery = applyPersistentStateQueryOverrides(persistedStateQuery, opts)
    const focusSessionId = typeof opts?.focusSessionId === 'string' ? opts.focusSessionId.trim() : ''
    const stateKey = `agena:${persistedStateQuery.directoriesPage ?? 0}:${persistedStateQuery.directoryQuery ?? ''}:${focusSessionId}`

    const existingRequest = sidebarStateRequestInFlight
    if (existingRequest?.key === stateKey && !inFlightRequestIsStale(existingRequest)) {
      return existingRequest.promise
    }
    if (existingRequest?.controller) {
      existingRequest.controller.abort()
    }
    const controller = createAbortController()

    let requestPromise!: Promise<void>
    requestPromise = (async () => {
      try {
        const state = await buildAgenaSidebarPayload(controller ? controller.signal : undefined)
        applySidebarStatePayload(state)
      } catch (err) {
        if (isAbortError(err)) return
        throw err
      } finally {
        if (sidebarStateRequestInFlight?.promise === requestPromise) {
          sidebarStateRequestInFlight = null
        }
      }
    })()

    sidebarStateRequestInFlight = {
      key: stateKey,
      promise: requestPromise,
      controller,
      startedAt: Date.now(),
    }
    return requestPromise
  }

  function scheduleSidebarStateSync(delayMs = 0) {
    if (sidebarStateSyncTimer !== null) {
      window.clearTimeout(sidebarStateSyncTimer)
      sidebarStateSyncTimer = null
    }

    sidebarStateSyncTimer = window.setTimeout(
      () => {
        sidebarStateSyncTimer = null
        void syncSidebarStateFromServer()
      },
      Math.max(0, Math.floor(delayMs)),
    )
  }

  async function syncSidebarStateFromServer() {
    if (sidebarStateSyncInFlight) {
      sidebarStateSyncQueued = true
      return
    }

    sidebarStateSyncInFlight = true
    try {
      await revalidateFromStateApi()
    } catch {
      // Keep existing sidebar cache on transient background sync failures.
    } finally {
      sidebarStateSyncInFlight = false
      if (sidebarStateSyncQueued) {
        sidebarStateSyncQueued = false
        scheduleSidebarStateSync(120)
      }
    }
  }

  function scheduleSidebarRecoverySync(reason: string, delayMs = 120, opts?: { force?: boolean }) {
    if (opts?.force) {
      if (sidebarRecoverySyncTimer !== null) {
        window.clearTimeout(sidebarRecoverySyncTimer)
        sidebarRecoverySyncTimer = null
      }
      lastSidebarRecoverySyncAt = Date.now()
      scheduleSidebarStateSync(Math.max(0, Math.floor(delayMs)))
      void reason
      return
    }

    const now = Date.now()
    const elapsed = now - lastSidebarRecoverySyncAt
    const throttleDelay = elapsed >= SIDEBAR_RECOVERY_THROTTLE_MS ? 0 : SIDEBAR_RECOVERY_THROTTLE_MS - elapsed
    const waitMs = Math.max(0, Math.floor(Math.max(delayMs, throttleDelay)))

    if (sidebarRecoverySyncTimer !== null) {
      window.clearTimeout(sidebarRecoverySyncTimer)
      sidebarRecoverySyncTimer = null
    }

    sidebarRecoverySyncTimer = window.setTimeout(() => {
      sidebarRecoverySyncTimer = null
      lastSidebarRecoverySyncAt = Date.now()
      scheduleSidebarStateSync(0)
      void reason
    }, waitMs)
  }


  async function revalidateFromApi(opts?: SidebarStateQuery, runtimeOpts?: RevalidateRuntimeOpts): Promise<boolean> {
    if (!runtimeOpts?.silent) {
      loading.value = true
    }
    error.value = null
    try {
      await revalidateFromStateApi(opts)
      return true
    } catch (err) {
      error.value = err instanceof Error ? err.message : String(err)
      return false
    } finally {
      if (!runtimeOpts?.silent) {
        loading.value = false
      }
    }
  }

  async function revalidateDirectoriesPageFromApi(opts?: {
    page?: number
    pageSize?: number
    query?: string
    silent?: boolean
  }): Promise<boolean> {
    if (!opts?.silent) {
      loading.value = true
    }
    error.value = null
    try {
      const pageRaw =
        typeof opts?.page === 'number' && Number.isFinite(opts.page) ? opts.page : uiPrefs.value.directoriesPage
      const page = Math.max(0, Math.floor(Number(pageRaw || 0)))
      const pageSizeRaw =
        typeof opts?.pageSize === 'number' && Number.isFinite(opts.pageSize)
          ? opts.pageSize
          : SIDEBAR_DIRECTORIES_PAGE_SIZE
      const pageSize = Math.max(1, Math.floor(Number(pageSizeRaw || SIDEBAR_DIRECTORIES_PAGE_SIZE)))
      const query =
        typeof opts?.query === 'string'
          ? opts.query.trim()
          : typeof persistedStateQuery.directoryQuery === 'string'
            ? persistedStateQuery.directoryQuery.trim()
            : ''

      const { entries, hasMore } = await fetchAgenaWorkspaces({
        limit: pageSize,
        search: query || undefined,
      })
      applyDirectoriesPagePayload({
        items: entries.map((entry) => ({ id: entry.id, path: entry.path })),
        total: hasMore ? page * pageSize + entries.length + 1 : entries.length,
        offset: page * pageSize,
        limit: pageSize,
      })
      return true
    } catch (err) {
      error.value = err instanceof Error ? err.message : String(err)
      return false
    } finally {
      if (!opts?.silent) {
        loading.value = false
      }
    }
  }

  function applyChatSidebarDeltaEvent(evt: SseEvent) {
    // Agena never emits chat-sidebar deltas; the global stream is
    // session_changed/runtime_signal/lagged. No-op keeps the event bus
    // explicit about what this store does (and does not) react to.
    void evt
  }

  function applyGlobalEvent(evt: SseEvent) {
    const type = readEventType(evt)
    if (!type) return
    const normalizedType = type.toLowerCase()

    if (normalizedType === 'chat-sidebar.delta') {
      applyChatSidebarDeltaEvent(evt)
      return
    }

    // Agena global stream events refresh the sidebar data. lagged/session
    // events schedule a coalesced reload rather than per-event fetches.
    if (
      normalizedType === 'session_changed' ||
      normalizedType === 'runtime_signal' ||
      normalizedType === 'lagged' ||
      normalizedType === 'session.created' ||
      normalizedType === 'session.updated' ||
      normalizedType === 'session.deleted' ||
      normalizedType === 'session.status'
    ) {
      scheduleSidebarRecoverySync(`event:${normalizedType}`, 180)
      return
    }

    if (SIDEBAR_RECOVERY_EVENT_TYPES.has(normalizedType)) {
      scheduleSidebarRecoverySync(`event:${normalizedType}`, 90)
    }
  }

  function setSessionRootPage(directoryId: string, page: number, pageSizeRaw: number): number {
    const pageSize = Math.max(1, Math.floor(Number(pageSizeRaw || 0) || 1))
    const did = String(directoryId || '').trim()
    if (!did) return 0

    const section = directorySidebarById.value[did]
    const fallbackMaxPage = Math.max(0, Math.ceil(Math.max(0, Number(section?.sessionCount || 0)) / pageSize) - 1)
    const maxPage = Math.max(0, Math.floor(Number(section?.rootPageCount || fallbackMaxPage + 1)) - 1)
    return Math.max(0, Math.min(maxPage, Math.floor(Number(page || 0))))
  }

  async function revalidateDirectorySessionPageFromApi(
    directoryId: string,
    opts?: { page?: number; pageSize?: number; silent?: boolean },
  ): Promise<boolean> {
    const did = String(directoryId || '').trim()
    if (!did) return false
    if (!opts?.silent) {
      loading.value = true
    }
    error.value = null
    try {
      const pageSizeRaw =
        typeof opts?.pageSize === 'number' && Number.isFinite(opts.pageSize)
          ? opts.pageSize
          : (persistedStateQuery.limitPerDirectory ?? SIDEBAR_DIRECTORY_SESSIONS_PAGE_SIZE)
      const pageSize = Math.max(1, Math.floor(Number(pageSizeRaw || SIDEBAR_DIRECTORY_SESSIONS_PAGE_SIZE)))
      const requestedPageRaw =
        typeof opts?.page === 'number' && Number.isFinite(opts.page)
          ? opts.page
          : (uiPrefs.value.sessionRootPageByDirectoryId[did] ?? 0)
      const page = setSessionRootPage(did, requestedPageRaw, pageSize)

      const workspace = directoriesById.value[did] || directoryEntryByPath(did, directoriesById.value)
      if (!workspace) return false
      const overview = await fetchAgenaOverview()
      const { sessions } = filterOverviewByWorkspace(overview, workspace.id)
      const directory = { id: workspace.id, path: workspace.path }
      const pinnedIds = new Set(uiPrefs.value.pinnedSessionIds || [])
      const pinnedRows: SidebarSessionRow[] = []
      const recentRows: SidebarSessionRow[] = []
      for (const session of sessions) {
        const row = toSidebarRowFromAgenaSession(session, directory)
        if (!row) continue
        if (pinnedIds.has(row.id)) pinnedRows.push(row)
        recentRows.push(row)
      }
      const sectionBase: DirectorySidebarView = {
        sessionCount: sessions.length,
        rootPage: page,
        rootPageCount: Math.max(1, Math.ceil(sessions.length / pageSize)),
        hasActiveOrBlocked: recentRows.some((row) => String(row.session?.state) === 'awaiting_user'),
        hasRunningSessions: recentRows.some((row) => String(row.session?.state) === 'running'),
        hasBlockedSessions: recentRows.some((row) => String(row.session?.state) === 'interrupted'),
        pinnedRows,
        recentRows,
        recentParentById: {},
        recentRootIds: recentRows.map((row) => row.id),
      }
      const section = enrichDirectorySidebarView(sectionBase, knownSidebarRowBySessionId())
      if (!section) return false

      const previousSection = directorySidebarById.value[did]
      if (!previousSection || !directorySidebarViewEquivalent(previousSection, section)) {
        directorySidebarById.value = {
          ...directorySidebarById.value,
          [did]: section,
        }
      }

      const previousRootPage = Math.max(0, Math.floor(Number(uiPrefs.value.sessionRootPageByDirectoryId[did] || 0)))
      if (previousRootPage !== section.rootPage) {
        const nextMap = {
          ...uiPrefs.value.sessionRootPageByDirectoryId,
          [did]: section.rootPage,
        }
        uiPrefs.value = normalizeUiPrefs(
          patchChatSidebarUiPrefs(uiPrefs.value, {
            sessionRootPageByDirectoryId: nextMap,
          }),
        )
      }

      return true
    } catch (err) {
      error.value = err instanceof Error ? err.message : String(err)
      return false
    } finally {
      if (!opts?.silent) {
        loading.value = false
      }
      scheduleSidebarSessionHydration()
    }
  }

  async function revalidateFooterFromApi(
    kind: SidebarFooterKind,
    opts?: { page?: number; pageSize?: number; silent?: boolean },
  ): Promise<boolean> {
    if (!opts?.silent) {
      loading.value = true
    }
    error.value = null
    try {
      const targetKind: SidebarFooterKind = kind
      const pageRaw =
        typeof opts?.page === 'number' && Number.isFinite(opts.page)
          ? opts.page
          : targetKind === 'pinned'
            ? uiPrefs.value.pinnedSessionsPage
            : targetKind === 'recent'
              ? uiPrefs.value.recentSessionsPage
              : uiPrefs.value.runningSessionsPage
      const page = Math.max(0, Math.floor(Number(pageRaw || 0)))
      const pageSizeRaw =
        typeof opts?.pageSize === 'number' && Number.isFinite(opts.pageSize) ? opts.pageSize : SIDEBAR_FOOTER_PAGE_SIZE
      const pageSize = Math.max(1, Math.floor(Number(pageSizeRaw || SIDEBAR_FOOTER_PAGE_SIZE)))

      const overview = await fetchAgenaOverview()
      const pinnedIds = new Set(uiPrefs.value.pinnedSessionIds || [])
      const sourceRows: SidebarSessionRow[] = []
      if (targetKind === 'pinned') {
        for (const session of [...overview.recent, ...overview.running, ...overview.attention]) {
          const sid = agenaSessionId(session)
          if (!pinnedIds.has(sid)) continue
          const row = toSidebarRowFromAgenaSession(session, null)
          if (row) sourceRows.push(row)
        }
      } else if (targetKind === 'recent') {
        for (const session of overview.recent) {
          const row = toSidebarRowFromAgenaSession(session, null)
          if (row) sourceRows.push(row)
        }
      } else {
        for (const session of overview.running) {
          const row = toSidebarRowFromAgenaSession(session, null)
          if (row) sourceRows.push(row)
        }
      }
      const offset = page * pageSize
      const total = sourceRows.length
      const view = enrichFooterView(
        {
          total,
          page,
          pageCount: Math.max(1, Math.ceil(total / pageSize)),
          rows: sourceRows.slice(offset, offset + pageSize),
        },
        knownSidebarRowBySessionId(),
      )

      if (targetKind === 'pinned') {
        if (!footerViewEquivalent(pinnedFooterView.value, view)) {
          pinnedFooterView.value = view
        }
      } else if (targetKind === 'recent') {
        if (!footerViewEquivalent(recentFooterView.value, view)) {
          recentFooterView.value = view
        }
      } else {
        if (!footerViewEquivalent(runningFooterView.value, view)) {
          runningFooterView.value = view
        }
      }

      const patch: Partial<ChatSidebarUiPrefs> = {}
      if (targetKind === 'pinned') {
        patch.pinnedSessionsPage = view.page
      } else if (targetKind === 'recent') {
        patch.recentSessionsPage = view.page
      } else {
        patch.runningSessionsPage = view.page
      }
      uiPrefs.value = normalizeUiPrefs(patchChatSidebarUiPrefs(uiPrefs.value, patch))

      return true
    } catch (err) {
      error.value = err instanceof Error ? err.message : String(err)
      return false
    } finally {
      if (!opts?.silent) {
        loading.value = false
      }
      scheduleSidebarSessionHydration()
    }
  }

  async function resolveDirectoryForSession(
    sessionId: string,
    hint?: { directoryId?: string; directoryPath?: string; locateResult?: JsonValue; skipRemote?: boolean },
  ): Promise<{ directoryId: string; directoryPath: string; locatedDir: string } | null> {
    const sid = String(sessionId || '').trim()
    if (!sid) return null

    const hintId = String(hint?.directoryId || '').trim()
    const hintPath = String(hint?.directoryPath || '').trim()
    if (hintId && hintPath) {
      return { directoryId: hintId, directoryPath: hintPath, locatedDir: hintPath }
    }

    const focus = sidebarStateFocus.value
    if (focus && focus.sessionId === sid) {
      return {
        directoryId: focus.directoryId,
        directoryPath: focus.directoryPath,
        locatedDir: focus.directoryPath,
      }
    }

    if (hint?.skipRemote) return null

    const located = hint?.locateResult
      ? asRecord(hint.locateResult)
      : asRecord((await chatApi.getSession(sid).catch(() => null)) ?? null)
    const locatedSession = asRecord((located?.session ?? null) as JsonValue)
    const locatedSessionId = readLocatedSessionId(locatedSession)
    const canUseRemoteLocate = !locatedSession || !locatedSessionId || locatedSessionId === sid
    const rawPid =
      typeof located?.workspace_id === 'number'
        ? String(located.workspace_id)
        : (located?.project_id as string | undefined)
    const rawPath = located?.path as string | undefined

    const pid = canUseRemoteLocate && typeof rawPid === 'string' ? rawPid.trim() : ''
    const ppath = canUseRemoteLocate && typeof rawPath === 'string' ? rawPath.trim() : ''
    const locatedDir = canUseRemoteLocate && typeof located?.directory === 'string' ? located.directory.trim() : ''

    const locatePath = locatedDir || ppath
    const matchedByPath = locatePath ? directoryEntryByPath(locatePath, directoriesById.value) : null
    if (matchedByPath?.id && matchedByPath.path) {
      return {
        directoryId: matchedByPath.id,
        directoryPath: matchedByPath.path,
        locatedDir: locatePath || matchedByPath.path,
      }
    }

    if (pid) {
      const matchedById = directoriesById.value[pid]
      if (matchedById?.path) {
        return {
          directoryId: matchedById.id,
          directoryPath: matchedById.path,
          locatedDir: locatePath || matchedById.path,
        }
      }
      if (ppath) {
        return {
          directoryId: pid,
          directoryPath: ppath,
          locatedDir: locatePath || ppath,
        }
      }
    }

    if (hintId) {
      const hintedById = directoriesById.value[hintId]
      if (hintedById?.path) {
        return {
          directoryId: hintedById.id,
          directoryPath: hintedById.path,
          locatedDir: locatePath || hintedById.path,
        }
      }
    }

    if (hintPath) {
      const hintedByPath = directoryEntryByPath(hintPath, directoriesById.value)
      if (hintedByPath?.id && hintedByPath.path) {
        return {
          directoryId: hintedByPath.id,
          directoryPath: hintedByPath.path,
          locatedDir: locatePath || hintedByPath.path,
        }
      }
    }

    return null
  }

  function statusLabelForSessionId(sessionId: string): { label: string; dotClass: string } {
    const sid = String(sessionId || '').trim()
    const runtime = runtimeBySessionId.value[sid]
    if (!runtime) return { label: String(i18n.global.t('chat.sidebar.sessionRow.status.idle')), dotClass: '' }

    if (runtime.displayState === 'needsPermission') {
      return {
        label: String(i18n.global.t('chat.sidebar.sessionRow.status.needsPermission')),
        dotClass: 'bg-amber-500',
      }
    }
    if (runtime.displayState === 'needsReply') {
      return {
        label: String(i18n.global.t('chat.sidebar.sessionRow.status.needsReply')),
        dotClass: 'bg-sky-500',
      }
    }
    if (runtime.displayState === 'retrying') {
      return {
        label: String(i18n.global.t('chat.sidebar.sessionRow.status.retrying')),
        dotClass: 'bg-primary animate-pulse',
      }
    }
    if (runtime.displayState === 'running') {
      return {
        label: String(i18n.global.t('chat.sidebar.sessionRow.status.running')),
        dotClass: 'bg-primary animate-pulse',
      }
    }
    if (runtime.displayState === 'coolingDown') {
      return {
        label: String(i18n.global.t('chat.sidebar.sessionRow.status.coolingDown')),
        dotClass: 'bg-primary/70',
      }
    }

    return { label: String(i18n.global.t('chat.sidebar.sessionRow.status.idle')), dotClass: '' }
  }

  function isSessionRuntimeActive(sessionId: string, opts?: { includeCooldown?: boolean }): boolean {
    const sid = String(sessionId || '').trim()
    if (!sid) return false
    return runtimeIsActive(runtimeBySessionId.value[sid], opts)
  }

  async function bootstrapWithStaleWhileRevalidate() {
    await revalidateFromApi()
  }

  async function resetAllPersistedState() {
    if (sidebarStateSyncTimer !== null) {
      window.clearTimeout(sidebarStateSyncTimer)
      sidebarStateSyncTimer = null
    }
    sidebarStateSyncInFlight = false
    sidebarStateSyncQueued = false

    if (sidebarRecoverySyncTimer !== null) {
      window.clearTimeout(sidebarRecoverySyncTimer)
      sidebarRecoverySyncTimer = null
    }
    lastSidebarRecoverySyncAt = 0

    if (sidebarStateRequestInFlight?.controller) {
      sidebarStateRequestInFlight.controller.abort()
    }
    sidebarStateRequestInFlight = null
    sidebarSessionHydrationInFlight.clear()
    sidebarSessionHydrationAttemptAt.clear()
    sidebarSessionHydrationRunning = null
    sidebarSessionHydrationQueued = false

    persistedStateQuery = {}

    directoriesById.value = {}
    directoryOrder.value = []
    runtimeBySessionId.value = {}

    directorySidebarById.value = {}
    pinnedFooterView.value = { total: 0, page: 0, pageCount: 1, rows: [] }
    recentFooterView.value = { total: 0, page: 0, pageCount: 1, rows: [] }
    runningFooterView.value = { total: 0, page: 0, pageCount: 1, rows: [] }
    sidebarStateFocus.value = null
    directoriesPageIndex.value = 0
    directoryPageRows.value = []
    directoryPageTotal.value = 0
    uiPrefs.value = defaultChatSidebarUiPrefs()
    loading.value = false
    error.value = null
  }

  return {
    directoriesById,
    runtimeBySessionId,
    directorySidebarById,
    pinnedFooterView,
    recentFooterView,
    runningFooterView,
    sidebarStateFocus,
    directoriesPageIndex,
    directoryPageRows,
    directoryPageTotal,
    uiPrefs,
    loading,
    error,
    visibleDirectories,
    setSessionRootPage,
    revalidateDirectoriesPageFromApi,
    revalidateDirectorySessionPageFromApi,
    revalidateFooterFromApi,
    commandSetDirectoriesPage,
    commandSetDirectoryCollapsed,
    commandSetDirectoryRootPage,
    commandSetSessionPinned,
    commandSetSessionExpanded,
    commandSetFooterOpen,
    commandSetFooterPage,
    resolveDirectoryForSession,
    statusLabelForSessionId,
    isSessionRuntimeActive,
    applyGlobalEvent,
    scheduleSidebarRecoverySync,
    revalidateFromApi,
    bootstrapWithStaleWhileRevalidate,
    resetAllPersistedState,
  }
})
