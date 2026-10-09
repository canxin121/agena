import { defineStore } from 'pinia'
import { computed, onScopeDispose, ref, watch } from 'vue'
import { i18n } from '@/i18n'

import * as chatApi from '@/stores/chat/api'
import { loadSidebarSessionPage } from './chat/sidebarPaging'
import { sessionWasDeleted, subscribeSessionDeletions } from './chat/sessionMutationSync'
import { isDocumentVisible, limitBackgroundReads } from '@/lib/backgroundReads'
import { createRevalidator } from '@/lib/revalidation'
import { conditionalJson } from '@/lib/conditionalJson'
import { apiJson } from '@/lib/api'
import {
  captureResourceObservation,
  canReuseResource,
  checkResourceVersions,
  invalidateResources,
  noteResourceVersion,
  subscribeResource,
} from '@/lib/resourceSync'
import { normalizeDirectories } from '@/features/sessions/model/projects'
import type { DirectoryEntry } from '@/features/sessions/model/types'
import { normalizeDirForCompare } from '@/features/sessions/model/labels'
import { loadExpandedTree } from '@/features/sessions/model/expandedTree'
import { sessionStateKind, type Session } from '@/types/chat'
import type { SseEvent } from '@/lib/sse'
import { defaultChatSidebarUiPrefs, patchChatSidebarUiPrefs, type ChatSidebarUiPrefs } from '@/data/chatSidebarUiPrefs'
import { useChatStore } from './chat'

import {
  sessionStateIsActive,
  stateSnapshotEquivalent,
  stateSnapshotFromAgenaSession,
  type SessionStateSnapshot,
} from './directorySessionRuntime'
import type { JsonObject as UnknownRecord, JsonValue } from '@/types/json'

type SidebarFooterKind = 'pinned' | 'favorite' | 'recent' | 'running'
// Sidebar data sources are:
//   GET /api/v1/workspaces        → directory list (WorkspaceResource {id, path})
//   GET /api/v1/sessions          → cross-workspace favorite/attention/running/recent buckets
//   GET /api/v1/sessions          → flat session list (search/workspace filters)
// Sidebar chrome (collapsed/expanded/pages) is client-local via uiPrefs.
// Session favorite/pinned flags are durable server metadata.

type WorkspaceStats = { total: number; roots: number; pinned: number; running: number; attention: number }
type SidebarWorkspace = DirectoryEntry & { stats?: WorkspaceStats }

function agenaSessionId(value: UnknownRecord | null | undefined): string {
  const raw = value?.id
  if (typeof raw === 'number' && Number.isFinite(raw)) return String(raw)
  if (typeof raw === 'string' && raw.trim()) return raw.trim()
  return ''
}

function pinnedSessionIdSet(sessions: UnknownRecord[]): Set<string> {
  return new Set(
    sessions
      .filter((session) => session.pinned === true)
      .map(agenaSessionId)
      .filter(Boolean),
  )
}

function stateMapFromAgenaSessions(sessions: UnknownRecord[]): Record<string, SessionStateSnapshot> {
  const states: Record<string, SessionStateSnapshot> = {}
  for (const session of sessions) {
    const id = agenaSessionId(session)
    if (id) states[id] = stateSnapshotFromAgenaSession(session)
  }
  return states
}

async function fetchAgenaWorkspaces(opts?: {
  limit?: number
  page?: number
  search?: string
  signal?: AbortSignal
}): Promise<{ entries: SidebarWorkspace[]; hasMore: boolean }> {
  const limit = Math.max(1, Math.min(200, Math.floor(opts?.limit || SIDEBAR_DIRECTORIES_PAGE_SIZE)))
  const params = new URLSearchParams({
    limit: String(limit),
    offset: String(Math.max(0, opts?.page || 0) * limit),
    include_session_count: 'false',
  })
  if (opts?.search) params.set('search', opts.search)
  const payload = asRecord(
    await conditionalJson<JsonValue>('workspaces:catalog', `/api/v1/workspaces?${params}`, { signal: opts?.signal }),
  )
  const entries: SidebarWorkspace[] = []
  for (const item of Array.isArray(payload?.items) ? payload.items : []) {
    const ws = asRecord(item)
    const id = agenaSessionId(ws)
    if (!id || typeof ws?.path !== 'string') continue
    entries.push({ id, path: ws.path, stats: asRecord(ws.session_stats) as WorkspaceStats | undefined })
  }
  return { entries, hasMore: asRecord(payload?.page)?.has_more === true }
}

function toSidebarRowFromAgenaSession(
  record: UnknownRecord | null | undefined,
  directory: DirectoryEntry | null,
  expandedParents?: Set<string>,
): SidebarSessionRow | null {
  const sid = agenaSessionId(record)
  if (!sid || sessionWasDeleted(sid)) return null
  const session: SidebarSessionSummary = { ...(record as UnknownRecord), id: sid }
  const parentId = agenaSessionId({ id: record?.parent_id } as UnknownRecord) || null
  const rootId = agenaSessionId({ id: record?.root_id } as UnknownRecord) || sid
  const depthRaw = Number(record?.depth)
  const childCountRaw = Number(record?.child_session_count)
  return {
    id: sid,
    session,
    directory,
    renderKey: sid,
    depth: Number.isFinite(depthRaw) ? Math.max(0, Math.floor(depthRaw)) : 0,
    parentId,
    rootId,
    isParent: Number.isFinite(childCountRaw) && childCountRaw > 0,
    isExpanded: expandedParents?.has(sid) === true,
  }
}

const SIDEBAR_DIRECTORIES_PAGE_SIZE = 15
const SIDEBAR_FOOTER_PAGE_SIZE = 10
const SIDEBAR_DIRECTORY_SESSIONS_PAGE_SIZE = 10
const SIDEBAR_REFRESH_INTERVAL_MS = 200
const SIDEBAR_RETRY_INTERVAL_MS = 10_000
const SIDEBAR_STATE_REQUEST_STALE_MS = 12000
const SIDEBAR_SESSION_HYDRATION_RETRY_MS = 10000
const SIDEBAR_RECOVERY_EVENT_TYPES = new Set(['session_changed', 'runtime_signal', 'lagged'])

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
  childPage?: number
  childPageCount?: number
}

type DirectorySidebarView = {
  sessionCount: number
  rootPage: number
  rootPageCount: number
  pinnedPage?: number
  pinnedPageCount?: number
  hasActiveOrAttention: boolean
  hasRunningSessions: boolean
  hasAttentionSessions: boolean
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
  favoritePage?: number
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
  | { type: 'setSessionExpanded'; sessionId: string; expanded: boolean }
  | { type: 'setFooterOpen'; kind: SidebarFooterKind; open: boolean }
  | { type: 'setFooterPage'; kind: SidebarFooterKind; page: number }

type NormalizedSidebarView = {
  directorySidebarById: Record<string, DirectorySidebarView>
  pinnedFooterView: SidebarFooterView
  favoriteFooterView: SidebarFooterView
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
    left.childPage === right.childPage &&
    left.childPageCount === right.childPageCount &&
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
    left.pinnedPage === right.pinnedPage &&
    left.pinnedPageCount === right.pinnedPageCount &&
    left.hasActiveOrAttention === right.hasActiveOrAttention &&
    left.hasRunningSessions === right.hasRunningSessions &&
    left.hasAttentionSessions === right.hasAttentionSessions &&
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

function stateMapEquivalent(
  left: Record<string, SessionStateSnapshot>,
  right: Record<string, SessionStateSnapshot>,
): boolean {
  const leftKeys = Object.keys(left)
  const rightKeys = Object.keys(right)
  if (leftKeys.length !== rightKeys.length) return false
  for (const key of leftKeys) {
    if (!hasOwn(right, key)) return false
    if (!stateSnapshotEquivalent(left[key], right[key])) return false
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
  if (!id || sessionWasDeleted(id)) return null

  const session = toSessionSummarySnapshot(record.session as JsonValue)
  const wireDirectory = toDirectoryEntry(record.directory as JsonValue)
  const directory = wireDirectory || null

  const depthRaw = Number(record.depth)
  const depth = Number.isFinite(depthRaw) ? Math.max(0, Math.floor(depthRaw)) : 0
  const renderKey = typeof record.renderKey === 'string' && record.renderKey.trim() ? record.renderKey.trim() : id
  const parentRaw = record.parentId
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
    childPage: Number(record.childPage) || 0,
    childPageCount: Number(record.childPageCount) || 1,
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
  const pageCountRaw = Number(record?.pageCount)
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

  const sessionCountRaw = Number(section.sessionCount)
  const rootPageRaw = Number(section.rootPage)
  const rootPageCountRaw = Number(section.rootPageCount)

  const sessionCount = Number.isFinite(sessionCountRaw) ? Math.max(0, Math.floor(sessionCountRaw)) : recentRows.length
  const rootPageCount = Number.isFinite(rootPageCountRaw) ? Math.max(1, Math.floor(rootPageCountRaw)) : 1
  const rootPage = Number.isFinite(rootPageRaw) ? Math.max(0, Math.min(rootPageCount - 1, Math.floor(rootPageRaw))) : 0

  const recentParentByIdRaw = asRecord(section.recentParentById as JsonValue) || {}
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

  const recentRootIdsRaw = Array.isArray(section.recentRootIds) ? section.recentRootIds : []
  const recentRootIds = recentRootIdsRaw.map((value) => String(value || '').trim()).filter(Boolean)

  const hasRunningSessions = section.hasRunningSessions === true
  const hasAttentionSessions = section.hasAttentionSessions === true
  const hasActiveOrAttention = section.hasActiveOrAttention === true || hasRunningSessions || hasAttentionSessions

  return {
    sessionCount,
    rootPage,
    rootPageCount,
    pinnedPage: Number(section.pinnedPage) || 0,
    pinnedPageCount: Number(section.pinnedPageCount) || 1,
    hasActiveOrAttention,
    hasRunningSessions,
    hasAttentionSessions,
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
  const directoryRowsById = view?.directoryRowsById as JsonValue
  const pinnedFooter = view?.pinnedFooter as JsonValue
  const favoriteFooter = view?.favoriteFooter as JsonValue
  const recentFooter = view?.recentFooter as JsonValue
  const runningFooter = view?.runningFooter as JsonValue

  return {
    directorySidebarById: normalizeDirectoryRowsById(directoryRowsById),
    pinnedFooterView: normalizeSidebarFooterView(pinnedFooter),
    favoriteFooterView: normalizeSidebarFooterView(favoriteFooter),
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
  if (incomingRecord?.version !== undefined && Number(currentRecord?.version || 0) > Number(incomingRecord.version)) {
    return currentRecord
  }

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

  if (hasOwn(opts, 'favoritePage')) {
    const value = Number(opts.favoritePage)
    if (Number.isFinite(value)) {
      next.favoritePage = Math.max(0, Math.floor(value))
    } else {
      delete next.favoritePage
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
  const stateBySessionId = ref<Record<string, SessionStateSnapshot>>({})

  const directorySidebarById = ref<Record<string, DirectorySidebarView>>({})
  const pinnedFooterView = ref<SidebarFooterView>({ total: 0, page: 0, pageCount: 1, rows: [] })
  const favoriteFooterView = ref<SidebarFooterView>({ total: 0, page: 0, pageCount: 1, rows: [] })
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
      favoritePage: Math.max(0, Math.floor(Number(prefs.favoriteSessionsPage || 0))),
      recentPage: Math.max(0, Math.floor(Number(prefs.recentSessionsPage || 0))),
      runningPage: Math.max(0, Math.floor(Number(prefs.runningSessionsPage || 0))),
    }
  }

  let disposed = false
  let sidebarInitialized = false
  let catalogDirty = false
  let recoveryCheck = false
  const dirtyDirectories = new Set<string>()
  const dirtyStats = new Set<string>()
  const dirtyFooters = new Set<SidebarFooterKind>()
  const expandedChildRecoveryAttempts = new Map<string, number>()
  const expandedChildRecoveryTimers = new Map<string, number>()
  let sidebarSync = createSidebarSync()
  const workspaceSubscriptions = new Map<string, () => void>()
  const footerSubscriptions = new Map<SidebarFooterKind, { key: string; release: () => void }>()
  const footerKinds = ['pinned', 'favorite', 'recent', 'running'] as const
  const sidebarResourceKeys = () => [
    'workspaces:catalog',
    ...footerKinds.map((kind) =>
      chatApi.sessionListResourceKey({ bucket: kind, countOnly: !uiPrefs.value[`${kind}SessionsOpen`] }),
    ),
    ...workspaceSubscriptions.keys(),
  ]
  const releaseSidebarResources = [
    subscribeResource('workspaces:catalog', () => {
      catalogDirty = true
      sidebarSync.invalidate(180)
    }),
  ]
  const releaseSessionActions = chat.$onAction(({ name, after }) => {
    if (name === 'updateSessionMetadata' || name === 'renameSession') {
      after((session) => {
        if (disposed || !session) return
        const previous = knownSidebarRowBySessionId()[session.id]
        if (Number(previous?.session?.version || 0) > Number(session.version || 0)) return
        applyHydratedSidebarSessions(
          new Map([[session.id, { session, directory: directoriesById.value[String(session.workspace_id)] ?? null }]]),
        )
        if (previous) {
          for (const [kind, view] of [
            ['favorite', favoriteFooterView],
            ['pinned', pinnedFooterView],
          ] as const) {
            const before = previous.session?.[kind] === true
            const member = session[kind] === true
            if (before === member) continue
            const old = view.value
            let rows = old.rows.filter((row) => row.id !== session.id)
            if (member && old.page === 0 && uiPrefs.value[`${kind}SessionsOpen`]) {
              const row = toSidebarRowFromAgenaSession(
                session as unknown as UnknownRecord,
                directoriesById.value[String(session.workspace_id)] ?? null,
              )
              if (row) rows = [row, ...rows].slice(0, SIDEBAR_FOOTER_PAGE_SIZE)
            }
            view.value = { ...old, total: Math.max(0, old.total + (member ? 1 : -1)), rows }
          }
        }
      })
      return
    }
    if (name !== 'createSession' && name !== 'forkSession' && name !== 'revertToMessage') return
    after((result) => {
      const session = name === 'revertToMessage' ? result?.session : result
      if (disposed || !session) return
      const directoryId = String(session.workspace_id)
      const needsReveal =
        uiPrefs.value.collapsedDirectoryIds.includes(directoryId) ||
        (!session.parent_id && (uiPrefs.value.sessionRootPageByDirectoryId[directoryId] ?? 0) > 0) ||
        (Boolean(session.parent_id) && !uiPrefs.value.expandedParentSessionIds.includes(String(session.parent_id)))
      const patch: Partial<ChatSidebarUiPrefs> = {
        collapsedDirectoryIds: uiPrefs.value.collapsedDirectoryIds.filter((id) => id !== directoryId),
      }
      if (!session.parent_id) {
        // New roots sort onto the first page. Make the result visible even
        // when creation started from a collapsed directory or an older page.
        patch.sessionRootPageByDirectoryId = {
          ...uiPrefs.value.sessionRootPageByDirectoryId,
          [directoryId]: 0,
        }
      } else {
        patch.expandedParentSessionIds = [
          ...new Set([...uiPrefs.value.expandedParentSessionIds, String(session.parent_id)]),
        ]
      }
      applyAuthoritativeUiPrefs(patchChatSidebarUiPrefs(uiPrefs.value, patch))
      syncWorkspaceSubscriptions(directoryPageRows.value)
      // Chat hydration may finish after the mutation's revision check. A
      // newly revealed section still needs its page read in that case.
      if (needsReveal && sidebarInitialized && directoriesById.value[directoryId]) {
        dirtyDirectories.add(directoryId)
        sidebarSync.invalidate(180)
      }
    })
  })
  const releaseSessionDeletions = subscribeSessionDeletions((ids) => {
    if (disposed) return
    const removed = new Set(ids)
    const knownRows = knownSidebarRowBySessionId()
    for (let previous = 0; previous !== removed.size; ) {
      previous = removed.size
      for (const row of Object.values(knownRows)) if (row.parentId && removed.has(row.parentId)) removed.add(row.id)
    }
    for (const [id, section] of Object.entries(directorySidebarById.value)) {
      const affected = new Set(
        [...section.recentRows, ...section.pinnedRows].filter((row) => removed.has(row.id)).map((row) => row.id),
      )
      if (!affected.size) continue
      directorySidebarById.value[id] = {
        ...section,
        sessionCount: Math.max(0, section.sessionCount - affected.size),
        recentRows: section.recentRows.filter((row) => !removed.has(row.id)),
        pinnedRows: section.pinnedRows.filter((row) => !removed.has(row.id)),
        recentRootIds: section.recentRootIds.filter((id) => !removed.has(id)),
        recentParentById: Object.fromEntries(
          Object.entries(section.recentParentById).filter(([id]) => !removed.has(id)),
        ),
      }
    }
    for (const view of [pinnedFooterView, favoriteFooterView, recentFooterView, runningFooterView]) {
      const old = view.value
      const rows = old.rows.filter((row) => !removed.has(row.id))
      if (rows.length !== old.rows.length)
        view.value = { ...old, total: Math.max(0, old.total - old.rows.length + rows.length), rows }
    }
    for (const id of removed) delete stateBySessionId.value[id]
    uiPrefs.value = patchChatSidebarUiPrefs(uiPrefs.value, {
      pinnedSessionIds: uiPrefs.value.pinnedSessionIds.filter((id) => !removed.has(id)),
      expandedParentSessionIds: uiPrefs.value.expandedParentSessionIds.filter((id) => !removed.has(id)),
    })
  })
  watch(
    () => footerKinds.map((kind) => uiPrefs.value[`${kind}SessionsOpen`]),
    () => {
      for (const kind of footerKinds) {
        const key = chatApi.sessionListResourceKey({ bucket: kind, countOnly: !uiPrefs.value[`${kind}SessionsOpen`] })
        if (footerSubscriptions.get(kind)?.key === key) continue
        footerSubscriptions.get(kind)?.release()
        footerSubscriptions.set(kind, {
          key,
          release: subscribeResource(key, () => {
            dirtyFooters.add(kind)
            sidebarSync.invalidate(180)
          }),
        })
      }
    },
    { immediate: true, flush: 'sync' },
  )
  let sidebarStateRequestInFlight: SidebarInFlightVoidRequest | null = null
  let targetedController: AbortController | null = null
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
    rows.push(
      ...pinnedFooterView.value.rows,
      ...favoriteFooterView.value.rows,
      ...recentFooterView.value.rows,
      ...runningFooterView.value.rows,
    )
    return rows
  }

  const knownSidebarRows = computed((): Record<string, SidebarSessionRow> => {
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
  })
  function knownSidebarRowBySessionId() {
    return knownSidebarRows.value
  }

  function knownDirectoryForSession(row: SidebarSessionRow): DirectoryEntry | null {
    if (row.directory?.id && row.directory.path) return row.directory
    const workspace = directoriesById.value[String(row.session?.workspace_id)]
    if (workspace) return workspace
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
      pinnedRows: section.pinnedRows.map((row) => {
        const enriched = enrichSidebarRowWithKnownData(row, knownRows)
        return sessionRowEquivalent(knownRows[row.id], enriched) ? knownRows[row.id]! : enriched
      }),
      recentRows: section.recentRows.map((row) => {
        const enriched = enrichSidebarRowWithKnownData(row, knownRows)
        return sessionRowEquivalent(knownRows[row.id], enriched) ? knownRows[row.id]! : enriched
      }),
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
          if (sessionRowEquivalent(row, nextRow)) return row
          changed = true
          return nextRow
        }),
        recentRows: section.recentRows.map((row) => {
          const hydrated = entries.get(row.id)
          if (!hydrated) return row
          const nextRow = mergeSidebarRowSnapshot(row, hydrated)
          if (sessionRowEquivalent(row, nextRow)) return row
          changed = true
          return nextRow
        }),
      }
      nextDirectorySidebarById[directoryId] = directorySidebarViewEquivalent(section, nextSection)
        ? section
        : nextSection
    }

    const mergeFooterRows = (rows: SidebarSessionRow[]): SidebarSessionRow[] =>
      rows.map((row) => {
        const hydrated = entries.get(row.id)
        if (!hydrated) return row
        const nextRow = mergeSidebarRowSnapshot(row, hydrated)
        if (sessionRowEquivalent(row, nextRow)) return row
        changed = true
        return nextRow
      })

    const nextPinnedFooterView = { ...pinnedFooterView.value, rows: mergeFooterRows(pinnedFooterView.value.rows) }
    const nextFavoriteFooterView = {
      ...favoriteFooterView.value,
      rows: mergeFooterRows(favoriteFooterView.value.rows),
    }
    const nextRecentFooterView = { ...recentFooterView.value, rows: mergeFooterRows(recentFooterView.value.rows) }
    const nextRunningFooterView = { ...runningFooterView.value, rows: mergeFooterRows(runningFooterView.value.rows) }

    if (changed) {
      directorySidebarById.value = nextDirectorySidebarById
      if (!footerViewEquivalent(pinnedFooterView.value, nextPinnedFooterView))
        pinnedFooterView.value = nextPinnedFooterView
      if (!footerViewEquivalent(favoriteFooterView.value, nextFavoriteFooterView))
        favoriteFooterView.value = nextFavoriteFooterView
      if (!footerViewEquivalent(recentFooterView.value, nextRecentFooterView))
        recentFooterView.value = nextRecentFooterView
      if (!footerViewEquivalent(runningFooterView.value, nextRunningFooterView))
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
    let directory =
      resolveDirectoryEntryForSessionSnapshot(rawSession, {
        directoryId: typeof located?.workspace_id === 'number' ? String(located.workspace_id) : hint?.directoryId,
        directoryPath: hint?.directoryPath,
      }) || null
    if (!directory) {
      directory = await fetchDirectoryForSessionSnapshot(rawSession)
    }
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

  async function fetchDirectoryForSessionSnapshot(
    session: Partial<Session> | SidebarSessionSummary | null | undefined,
  ): Promise<DirectoryEntry | null> {
    const workspaceId = Number((session as SidebarSessionSummary | null)?.workspace_id)
    if (!Number.isSafeInteger(workspaceId) || workspaceId <= 0) return null
    const known = directoriesById.value[String(workspaceId)]
    if (known?.path) return known
    const workspace = await chatApi.getWorkspace(workspaceId).catch(() => null)
    if (!workspace?.path) return null
    return { id: String(workspace.id), path: workspace.path }
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
        await Promise.all(ids.map((id) => limitBackgroundReads(() => chatApi.getSession(id)).catch(() => null)))
      ).filter((s): s is Session => Boolean(s))
      for (const session of hydratedSessions) {
        const sid = typeof session?.id === 'string' ? session.id.trim() : ''
        if (!sid || !candidateIds.has(sid)) continue
        unresolved.delete(sid)
        const directory =
          resolveDirectoryEntryForSessionSnapshot(session, {
            directoryPath,
            directoryId: hintsBySessionId.get(sid)?.directoryId,
          }) || (await fetchDirectoryForSessionSnapshot(session))
        hydrated.set(sid, {
          session,
          directory,
        })
      }
    })

    await Promise.allSettled(directoryTasks)

    const locateTargets = [...unresolved]
    const locateTasks = locateTargets.map((sid) => {
      let task = sidebarSessionHydrationInFlight.get(sid)
      if (!task) {
        task = limitBackgroundReads(() => hydrateSessionViaLocate(sid, hintsBySessionId.get(sid))).finally(() => {
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
          Date.now() - SIDEBAR_SESSION_HYDRATION_RETRY_MS + SIDEBAR_RETRY_INTERVAL_MS,
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
          Date.now() - SIDEBAR_SESSION_HYDRATION_RETRY_MS + SIDEBAR_RETRY_INTERVAL_MS,
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
      // Agena has no sidebar-chrome command endpoint; these commands mutate
      // the client-local collapsed/expanded/page preferences.
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
      } else if (command.type === 'setSessionExpanded') {
        const current = new Set(uiPrefs.value.expandedParentSessionIds || [])
        if (command.expanded) current.add(command.sessionId)
        else current.delete(command.sessionId)
        patch.expandedParentSessionIds = [...current]
      } else if (command.type === 'setFooterOpen') {
        if (command.kind === 'pinned') patch.pinnedSessionsOpen = command.open
        else if (command.kind === 'favorite') patch.favoriteSessionsOpen = command.open
        else if (command.kind === 'recent') patch.recentSessionsOpen = command.open
        else patch.runningSessionsOpen = command.open
      } else if (command.type === 'setFooterPage') {
        const target = Math.max(0, Math.floor(Number(command.page || 0)))
        if (command.kind === 'pinned') patch.pinnedSessionsPage = target
        else if (command.kind === 'favorite') patch.favoriteSessionsPage = target
        else if (command.kind === 'recent') patch.recentSessionsPage = target
        else patch.runningSessionsPage = target
      }

      const next = normalizeUiPrefs(patchChatSidebarUiPrefs(uiPrefs.value, patch))
      applyAuthoritativeUiPrefs(next)
      syncWorkspaceSubscriptions(directoryPageRows.value)
      sidebarStateRequestInFlight?.controller?.abort()
      sidebarStateRequestInFlight = null
      if (command.type === 'setFooterOpen' || command.type === 'setFooterPage') {
        if (!(await revalidateFooterFromApi(command.kind, { silent: true }))) return false
      } else if (command.type === 'setSessionExpanded') {
        const row = knownSidebarRowBySessionId()[command.sessionId]
        const directory = row && knownDirectoryForSession(row)
        const footers = footerKinds.filter((kind) =>
          footerViewForKind(kind).rows.some((row) => row.id === command.sessionId),
        )
        const results = await Promise.all([
          ...(directory && directorySidebarById.value[directory.id]
            ? [revalidateDirectorySessionPageFromApi(directory.id, { silent: true })]
            : []),
          ...footers.map((kind) => revalidateFooterFromApi(kind, { silent: true })),
        ])
        if (!results.every(Boolean)) return false
      } else if (command.type === 'setDirectoryCollapsed' || command.type === 'setDirectoryRootPage') {
        if (!(await revalidateDirectorySessionPageFromApi(command.directoryId, { silent: true }))) return false
      } else if (command.type === 'setDirectoriesPage') {
        await revalidateFromStateApi()
      }
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
    if (!opts?.silent) {
      loading.value = true
      error.value = null
    }
    try {
      const updated = await chat.updateSessionMetadata(sid, { pinned })
      if (!updated) return false

      // Keep the local projection responsive; version recovery updates only
      // the affected directory and dependent footer buckets.
      const current = new Set(uiPrefs.value.pinnedSessionIds || [])
      if (pinned) current.add(sid)
      else current.delete(sid)
      applyAuthoritativeUiPrefs(
        normalizeUiPrefs(
          patchChatSidebarUiPrefs(uiPrefs.value, {
            pinnedSessionIds: [...current],
          }),
        ),
      )
      sidebarStateRequestInFlight?.controller?.abort()
      sidebarStateRequestInFlight = null
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

  async function commandSetSessionExpanded(
    sessionId: string,
    expanded: boolean,
    opts?: { silent?: boolean },
  ): Promise<boolean> {
    const sid = String(sessionId || '').trim()
    if (!sid) return false
    if (!expanded) clearExpandedChildRecovery(sid)
    return executeSidebarCommand({ type: 'setSessionExpanded', sessionId: sid, expanded }, opts)
  }

  async function commandExpandSessionAncestors(ids: string[]): Promise<boolean> {
    const expanded = new Set(uiPrefs.value.expandedParentSessionIds)
    const directories = new Set<string>()
    const known = knownSidebarRowBySessionId()
    for (const id of ids) {
      if (!id || expanded.has(id)) continue
      expanded.add(id)
      const row = known[id]
      const directory = row && knownDirectoryForSession(row)
      if (directory) directories.add(directory.id)
    }
    applyAuthoritativeUiPrefs(
      normalizeUiPrefs(patchChatSidebarUiPrefs(uiPrefs.value, { expandedParentSessionIds: [...expanded] })),
    )
    const results = await Promise.all(
      [...directories].map((id) => revalidateDirectorySessionPageFromApi(id, { silent: true })),
    )
    return results.every(Boolean)
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
        : kind === 'favorite'
          ? { favoritePage: target }
          : kind === 'recent'
            ? { recentPage: target }
            : { runningPage: target }),
    }
    return executeSidebarCommand({ type: 'setFooterPage', kind, page: target }, opts)
  }

  function parseStateMap(raw: JsonValue): Record<string, SessionStateSnapshot> {
    const statePayload = asRecord(raw) || {}
    const next: Record<string, SessionStateSnapshot> = {}
    for (const [sessionIdRaw, stateRaw] of Object.entries(statePayload)) {
      const sessionId = String(sessionIdRaw || '').trim()
      if (!sessionId) continue
      next[sessionId] = stateSnapshotFromAgenaSession({
        state: asRecord(stateRaw as JsonValue)?.state,
        updated_at: asRecord(stateRaw as JsonValue)?.updatedAt,
      })
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
      throw new Error(i18n.global.t('chat.errors.sidebarPayloadMissingPreferences'))
    }
    applyAuthoritativeUiPrefs((stateRecord.preferences as Partial<ChatSidebarUiPrefs>) || undefined)

    applyDirectoriesPagePayload(stateRecord.directoriesPage as JsonValue)

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
      favoriteFooterView: enrichFooterView(normalizedViewRaw.favoriteFooterView, knownRows),
      recentFooterView: enrichFooterView(normalizedViewRaw.recentFooterView, knownRows),
      runningFooterView: enrichFooterView(normalizedViewRaw.runningFooterView, knownRows),
    }

    const nextStateBySessionId = parseStateMap(stateRecord.stateBySessionId as JsonValue)
    if (!stateMapEquivalent(stateBySessionId.value, nextStateBySessionId)) {
      stateBySessionId.value = nextStateBySessionId
    }
    if (!directorySidebarByIdEquivalent(directorySidebarById.value, normalizedView.directorySidebarById)) {
      directorySidebarById.value = normalizedView.directorySidebarById
    }
    if (!footerViewEquivalent(pinnedFooterView.value, normalizedView.pinnedFooterView)) {
      pinnedFooterView.value = normalizedView.pinnedFooterView
    }
    if (!footerViewEquivalent(favoriteFooterView.value, normalizedView.favoriteFooterView)) {
      favoriteFooterView.value = normalizedView.favoriteFooterView
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

  const workspaceStats = new Map<string, WorkspaceStats>()
  const workspaceStatsVersions = new Map<string, string>()
  const childPageById = new Map<string, number>()
  const pinnedPageByDirectory = new Map<string, number>()
  const pageRequests = new Map<string, AbortController>()
  type DirectoryPage = Awaited<ReturnType<typeof loadSidebarSessionPage>>
  const directoryPages = new Map<string, { resource: string; scope: number; token?: string; value: DirectoryPage }>()

  function subscribeDirectoryList(key: string, id: string) {
    if (workspaceSubscriptions.has(key)) return
    workspaceSubscriptions.set(
      key,
      subscribeResource(key, () => {
        dirtyDirectories.add(id)
        for (const kind of footerKinds)
          if (footerViewForKind(kind).rows.some((row) => row.isExpanded && key.endsWith(`:parent:${row.id}`)))
            dirtyFooters.add(kind)
        sidebarSync.invalidate(180)
      }),
    )
  }

  async function loadDirectoryPage(
    options: Parameters<typeof chatApi.listSessions>[0],
    page: number,
    size: number,
    force = false,
  ) {
    const resource = chatApi.sessionListResourceKey(options)
    subscribeDirectoryList(resource, String(options?.workspaceId))
    const key = `${resource}:${page}:${size}`
    const cached = directoryPages.get(key)
    if (cached && !force) {
      await checkResourceVersions([resource], options?.signal)
      if (
        cached.scope === captureResourceObservation(resource).scope &&
        cached.token &&
        canReuseResource(resource, cached.token)
      )
        return cached.value
    }
    const value = await loadSidebarSessionPage(options ?? {}, page, size, chatApi.listSessions, force)
    options?.signal?.throwIfAborted()
    const observed = value.observation
    if (observed && observed.scope === captureResourceObservation(resource).scope) {
      directoryPages.set(key, { resource, scope: observed.scope, token: observed.token, value })
      // Retain just the displayed page for each list, plus a finite ceiling.
      for (const [oldKey, old] of directoryPages)
        if (oldKey !== key && old.resource === resource) directoryPages.delete(oldKey)
      while (directoryPages.size > 256) directoryPages.delete(directoryPages.keys().next().value!)
    }
    return value
  }

  function syncWorkspaceSubscriptions(entries: DirectoryEntry[]) {
    const liveIds = new Set(entries.map((entry) => entry.id))
    for (const id of workspaceStats.keys())
      if (!liveIds.has(id)) {
        workspaceStats.delete(id)
        workspaceStatsVersions.delete(id)
      }
    const liveKeys = new Set(
      entries.flatMap((entry) => {
        const stats = `workspace:${entry.id}:stats`
        if (uiPrefs.value.collapsedDirectoryIds.includes(entry.id)) return [stats]
        const section = directorySidebarById.value[entry.id]
        const children = [...(section?.recentRows ?? []), ...(section?.pinnedRows ?? [])]
          .filter((row) => row.isExpanded && row.isParent)
          .map((row) => chatApi.sessionListResourceKey({ workspaceId: entry.id, parentId: row.id }))
        return [
          stats,
          chatApi.sessionListResourceKey({ workspaceId: entry.id, roots: true }),
          ...(workspaceStats.get(entry.id)?.pinned === 0
            ? []
            : [chatApi.sessionListResourceKey({ workspaceId: entry.id, bucket: 'pinned' })]),
          ...children,
        ]
      }),
    )
    for (const kind of footerKinds)
      for (const row of footerViewForKind(kind).rows)
        if (row.isExpanded && row.isParent && row.session?.workspace_id)
          liveKeys.add(
            chatApi.sessionListResourceKey({ workspaceId: Number(row.session.workspace_id), parentId: row.id }),
          )
    for (const [key, release] of workspaceSubscriptions)
      if (!liveKeys.has(key)) {
        release()
        workspaceSubscriptions.delete(key)
      }
    for (const [key, cached] of directoryPages) if (!liveKeys.has(cached.resource)) directoryPages.delete(key)
    for (const entry of entries) {
      const key = `workspace:${entry.id}:stats`
      if (!workspaceSubscriptions.has(key))
        workspaceSubscriptions.set(
          key,
          subscribeResource(key, () => {
            dirtyStats.add(entry.id)
            dirtyDirectories.add(entry.id)
            sidebarSync.invalidate(180)
          }),
        )
      for (const list of liveKeys)
        if (list.startsWith(`workspace:${entry.id}:sessions:`)) subscribeDirectoryList(list, entry.id)
    }
  }

  async function loadWorkspaceStats(ids: string[], signal?: AbortSignal) {
    if (!ids.length) return
    const observations = new Map(ids.map((id) => [id, captureResourceObservation(`workspace:${id}:stats`)]))
    const payload = await limitBackgroundReads(
      () =>
        apiJson<{ items: Array<{ workspace_id: number; revision: string; stats: WorkspaceStats }> }>(
          `/api/v1/workspaces/session-stats?ids=${ids.join(',')}`,
          {
            cache: 'no-store',
            signal: signal ? AbortSignal.any([signal, AbortSignal.timeout(30_000)]) : AbortSignal.timeout(30_000),
          },
        ),
      signal,
    )
    signal?.throwIfAborted()
    for (const row of payload.items) {
      const id = String(row.workspace_id)
      if (!observations.has(id)) continue
      if (noteResourceVersion(`workspace:${id}:stats`, row.revision, observations.get(id))) {
        workspaceStats.set(id, row.stats)
        workspaceStatsVersions.set(id, row.revision)
      }
    }
  }

  function syncLoadedPinnedFlags(sessions: UnknownRecord[]) {
    const pinnedIds = new Set(uiPrefs.value.pinnedSessionIds)
    for (const session of sessions) {
      const id = agenaSessionId(session)
      if (session.pinned === true) pinnedIds.add(id)
      else pinnedIds.delete(id)
    }
    uiPrefs.value = normalizeUiPrefs(patchChatSidebarUiPrefs(uiPrefs.value, { pinnedSessionIds: [...pinnedIds] }))
  }

  function clearExpandedChildRecovery(sessionId: string) {
    const timer = expandedChildRecoveryTimers.get(sessionId)
    if (timer !== undefined) window.clearTimeout(timer)
    expandedChildRecoveryTimers.delete(sessionId)
    expandedChildRecoveryAttempts.delete(sessionId)
  }

  function scheduleExpandedChildRecovery(workspaceId: number, sessionId: string) {
    if (disposed || !Number.isSafeInteger(workspaceId) || workspaceId <= 0) return
    if (!uiPrefs.value.expandedParentSessionIds.includes(sessionId)) return
    if (expandedChildRecoveryTimers.has(sessionId)) return
    const attempt = expandedChildRecoveryAttempts.get(sessionId) || 0
    // Session creation and parent-list revisions can arrive in adjacent SSE
    // events. Retry an empty expanded branch a few times so an empty page from
    // the first event cannot strand the disclosure before the child commits.
    if (attempt >= 3) return
    expandedChildRecoveryAttempts.set(sessionId, attempt + 1)
    const timer = window.setTimeout(
      () => {
        expandedChildRecoveryTimers.delete(sessionId)
        if (disposed || !uiPrefs.value.expandedParentSessionIds.includes(sessionId)) {
          clearExpandedChildRecovery(sessionId)
          return
        }
        dirtyDirectories.add(String(workspaceId))
        for (const kind of footerKinds)
          if (footerViewForKind(kind).rows.some((row) => row.id === sessionId && row.isExpanded)) dirtyFooters.add(kind)
        sidebarSync.invalidate(0)
      },
      350 * 2 ** attempt,
    )
    expandedChildRecoveryTimers.set(sessionId, timer)
  }

  function footerViewForKind(kind: SidebarFooterKind): SidebarFooterView {
    return kind === 'pinned'
      ? pinnedFooterView.value
      : kind === 'favorite'
        ? favoriteFooterView.value
        : kind === 'recent'
          ? recentFooterView.value
          : runningFooterView.value
  }

  async function loadSessionTree(
    sessions: UnknownRecord[],
    directory: DirectoryEntry | null,
    pageSize: number,
    signal?: AbortSignal,
  ): Promise<SidebarSessionRow[]> {
    const expanded = new Set(uiPrefs.value.expandedParentSessionIds)
    const seen = new Set<string>()
    return loadExpandedTree<UnknownRecord, SidebarSessionRow>(
      sessions,
      async (session, depth, root) => {
        const workspaceId = Number(session.workspace_id)
        const row = toSidebarRowFromAgenaSession(
          session,
          directory || directoriesById.value[String(workspaceId)] || null,
          expanded,
        )
        if (!row || seen.has(row.id)) return null
        seen.add(row.id)
        row.depth = depth
        row.rootId = agenaSessionId(root)
        const previousRow = knownSidebarRowBySessionId()[row.id]
        if (!row.isExpanded) {
          // A parent can briefly report zero children while the task-created
          // child row is committing. Keep its disclosure affordance through a
          // collapse so the user can retry the branch after the count catches up.
          if (previousRow?.isParent) row.isParent = true
          return { row }
        }
        const childOptions = { workspaceId, parentId: row.id, signal }
        let children = await loadDirectoryPage(childOptions, childPageById.get(row.id) || 0, pageSize)
        const expectedChildren = Number(session.child_session_count)
        if (children.sessions.length === 0) {
          // An expanded branch was shown to the user as a parent previously.
          // Treat the persisted expansion as evidence even if this refresh's
          // summary temporarily says zero children, and bypass the ETag once
          // in case an empty child page survived the session-created event.
          children = await loadDirectoryPage(childOptions, childPageById.get(row.id) || 0, pageSize, true)
        }
        const knownParent =
          row.isParent ||
          row.isExpanded ||
          (Number.isFinite(expectedChildren) && expectedChildren > 0) ||
          previousRow?.isParent === true
        if (children.sessions.length > 0) clearExpandedChildRecovery(row.id)
        else if (knownParent || row.isExpanded) scheduleExpandedChildRecovery(workspaceId, row.id)
        row.isParent =
          children.total > 0 ||
          children.sessions.length > 0 ||
          (Number.isFinite(expectedChildren) && expectedChildren > 0) ||
          knownParent
        row.childPage = children.page
        row.childPageCount = children.pageCount
        return { row, children: children.sessions as unknown as UnknownRecord[] }
      },
      signal,
    )
  }

  async function loadDirectoryView(
    directory: DirectoryEntry,
    page: number,
    pageSize: number,
    signal?: AbortSignal,
  ): Promise<DirectorySidebarView> {
    const stats = workspaceStats.get(directory.id)
    const collapsed = uiPrefs.value.collapsedDirectoryIds.includes(directory.id)
    const empty = { sessions: [], page: 0, pageCount: 1, total: 0 }
    const [roots, pins] = collapsed
      ? [empty, empty]
      : await Promise.all([
          loadDirectoryPage({ workspaceId: directory.id, roots: true, signal }, page, pageSize),
          stats?.pinned === 0
            ? empty
            : loadDirectoryPage(
                { workspaceId: directory.id, bucket: 'pinned', signal },
                pinnedPageByDirectory.get(directory.id) || 0,
                pageSize,
              ),
        ])
    const [recentRows, pinnedRows] = await Promise.all([
      loadSessionTree(roots.sessions as unknown as UnknownRecord[], directory, pageSize, signal),
      loadSessionTree(pins.sessions as unknown as UnknownRecord[], directory, pageSize, signal),
    ])
    const parentById = Object.fromEntries(recentRows.map((row) => [row.id, row.parentId]))
    return {
      sessionCount: stats?.total ?? roots.total,
      rootPage: collapsed ? page : roots.page,
      rootPageCount: collapsed ? Math.max(1, Math.ceil((stats?.roots || 0) / pageSize)) : roots.pageCount,
      pinnedPage: pins.page,
      pinnedPageCount: pins.pageCount,
      hasRunningSessions: (stats?.running || 0) > 0,
      hasAttentionSessions: (stats?.attention || 0) > 0,
      hasActiveOrAttention: (stats?.running || 0) + (stats?.attention || 0) > 0,
      pinnedRows,
      recentRows,
      recentParentById: parentById,
      recentRootIds: roots.sessions.filter((session) => !sessionWasDeleted(session.id)).map((session) => session.id),
    }
  }

  async function loadFooterView(
    kind: SidebarFooterKind,
    page: number,
    pageSize: number,
    signal?: AbortSignal,
  ): Promise<SidebarFooterView> {
    const countOnly = !uiPrefs.value[`${kind}SessionsOpen`]
    const result = await loadSidebarSessionPage({ bucket: kind, countOnly, signal }, page, pageSize)
    return {
      total: result.total,
      page: result.page,
      pageCount: result.pageCount,
      rows: await loadSessionTree(result.sessions as unknown as UnknownRecord[], null, pageSize, signal),
    }
  }

  async function buildAgenaSidebarPayload(signal?: AbortSignal): Promise<JsonValue> {
    // One small preflight covers every list/page in the sidebar. Unchanged
    // workspaces and buckets reuse their bodies without another HTTP request.
    await checkResourceVersions(sidebarResourceKeys(), signal)
    const page = Math.max(0, Math.floor(Number(persistedStateQuery.directoriesPage || 0)))
    const pageSize = SIDEBAR_DIRECTORIES_PAGE_SIZE
    const query = (persistedStateQuery.directoryQuery || '').trim()
    const footer = Promise.all(
      (['pinned', 'favorite', 'recent', 'running'] as const).map((kind) => {
        const open = uiPrefs.value[`${kind}SessionsOpen`]
        const requestedPage = uiPrefs.value[`${kind}SessionsPage`]
        return loadFooterView(kind, open ? requestedPage : 0, open ? SIDEBAR_FOOTER_PAGE_SIZE : 1, signal).then(
          (view) =>
            open
              ? view
              : {
                  ...view,
                  rows: [],
                  page: requestedPage,
                  pageCount: Math.max(1, Math.ceil(view.total / SIDEBAR_FOOTER_PAGE_SIZE)),
                },
        )
      }),
    )
    const [workspaces, footerViews] = await Promise.all([
      fetchAgenaWorkspaces({ limit: pageSize, page, search: query || undefined, signal }),
      footer,
    ])
    syncWorkspaceSubscriptions(workspaces.entries)
    await loadWorkspaceStats(
      workspaces.entries.map((entry) => entry.id),
      signal,
    )
    const directories = await Promise.all(
      workspaces.entries.map(
        async (entry) =>
          [
            entry.id,
            await loadDirectoryView(
              entry,
              uiPrefs.value.sessionRootPageByDirectoryId[entry.id] || 0,
              SIDEBAR_DIRECTORY_SESSIONS_PAGE_SIZE,
              signal,
            ),
          ] as const,
      ),
    )
    const [pinnedFooter, favoriteFooter, recentFooter, runningFooter] = footerViews
    const allOverview = [
      ...directories.flatMap(([, view]) => [...view.recentRows, ...view.pinnedRows]),
      ...pinnedFooter.rows,
      ...favoriteFooter.rows,
      ...recentFooter.rows,
      ...runningFooter.rows,
    ].flatMap((row) => (row.session ? [row.session] : []))
    const pinnedIds = pinnedSessionIdSet(allOverview)
    return {
      preferences: normalizeUiPrefs(patchChatSidebarUiPrefs(uiPrefs.value, { pinnedSessionIds: [...pinnedIds] })),
      directoriesPage: {
        items: workspaces.entries.map(({ id, path }) => ({ id, path })),
        total: page * pageSize + workspaces.entries.length + (workspaces.hasMore ? 1 : 0),
        offset: page * pageSize,
        limit: pageSize,
      },
      view: {
        directoryRowsById: Object.fromEntries(directories),
        pinnedFooter,
        favoriteFooter,
        recentFooter,
        runningFooter,
      } as unknown as JsonValue,
      stateBySessionId: stateMapFromAgenaSessions(allOverview) as unknown as JsonValue,
      focus: null,
    }
  }

  onScopeDispose(() => {
    disposed = true
    for (const timer of expandedChildRecoveryTimers.values()) window.clearTimeout(timer)
    expandedChildRecoveryTimers.clear()
    expandedChildRecoveryAttempts.clear()
    sidebarStateRequestInFlight?.controller?.abort()
    targetedController?.abort()
    for (const controller of pageRequests.values()) controller.abort()
    sidebarSync.dispose()
    releaseSessionActions()
    releaseSessionDeletions()
    for (const release of releaseSidebarResources) release()
    for (const subscription of footerSubscriptions.values()) subscription.release()
    for (const release of workspaceSubscriptions.values()) release()
    if (typeof document !== 'undefined') document.removeEventListener?.('visibilitychange', resumeSidebarSync)
  })

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
    targetedController?.abort()
    for (const request of pageRequests.values()) request.abort()
    const controller = createAbortController()
    const timeout = window.setTimeout(() => controller?.abort(), 30_000)

    let requestPromise!: Promise<void>
    requestPromise = (async () => {
      try {
        const state = await buildAgenaSidebarPayload(controller ? controller.signal : undefined)
        if (!controller?.signal.aborted) {
          applySidebarStatePayload(state)
          syncWorkspaceSubscriptions(directoryPageRows.value)
          sidebarInitialized = true
        }
      } catch (err) {
        if (isAbortError(err)) return
        throw err
      } finally {
        window.clearTimeout(timeout)
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

  function createSidebarSync() {
    return createRevalidator(
      async () => {
        // Obtain a trailing snapshot if an event arrived during a foreground read.
        await sidebarStateRequestInFlight?.promise.catch(() => {})
        await Promise.allSettled([...sectionLoads.values()])
        if (!disposed && sidebarInitialized && isDocumentVisible()) await refreshDirtySidebar()
      },
      {
        intervalMs: SIDEBAR_REFRESH_INTERVAL_MS,
        retryMs: SIDEBAR_RETRY_INTERVAL_MS,
        enabled: () => !disposed && isDocumentVisible(),
      },
    )
  }

  async function refreshDirtySidebar() {
    const controller = new AbortController()
    targetedController = controller
    try {
      await refreshDirtySidebarImpl(controller.signal)
    } finally {
      if (targetedController === controller) targetedController = null
    }
  }

  async function refreshDirtySidebarImpl(signal: AbortSignal) {
    if (recoveryCheck) {
      recoveryCheck = false
      try {
        invalidateResources(sidebarResourceKeys())
        await checkResourceVersions(sidebarResourceKeys(), signal)
      } catch (error) {
        recoveryCheck = true
        throw error
      }
    }
    const catalog = catalogDirty
    const directories = new Set(dirtyDirectories)
    const stats = new Set(dirtyStats)
    const footers = new Set(dirtyFooters)
    catalogDirty = false
    dirtyDirectories.clear()
    dirtyStats.clear()
    dirtyFooters.clear()
    try {
      if (catalog) {
        const page = persistedStateQuery.directoriesPage || 0
        const result = await fetchAgenaWorkspaces({ page, search: persistedStateQuery.directoryQuery, signal })
        signal.throwIfAborted()
        for (const entry of result.entries) {
          const old = directoriesById.value[entry.id]
          if (!old || old.path !== entry.path || !directorySidebarById.value[entry.id]) directories.add(entry.id)
          if (!workspaceStats.has(entry.id)) stats.add(entry.id)
        }
        applyDirectoriesPagePayload({
          items: result.entries,
          offset: page * SIDEBAR_DIRECTORIES_PAGE_SIZE,
          limit: SIDEBAR_DIRECTORIES_PAGE_SIZE,
          total: page * SIDEBAR_DIRECTORIES_PAGE_SIZE + result.entries.length + (result.hasMore ? 1 : 0),
        } as unknown as JsonValue)
        syncWorkspaceSubscriptions(result.entries)
        for (const id of Object.keys(directorySidebarById.value))
          if (!directoriesById.value[id]) {
            pageRequests.get(`directory:${id}`)?.abort()
            delete directorySidebarById.value[id]
            workspaceStats.delete(id)
          }
      }
      const current = (id: string) => Boolean(directoriesById.value[id])
      await loadWorkspaceStats(
        [...stats].filter(
          (id) =>
            current(id) &&
            (!workspaceStats.has(id) ||
              workspaceStatsVersions.get(id) !== captureResourceObservation(`workspace:${id}:stats`).token),
        ),
        signal,
      )
      signal.throwIfAborted()
      const results = await Promise.all([
        ...[...directories].filter(current).map((id) => revalidateDirectorySessionPageFromApi(id, { silent: true })),
        ...[...footers].map((kind) => revalidateFooterFromApi(kind, { silent: true })),
      ])
      if (results.some((ok) => !ok)) throw new Error(error.value || 'Sidebar refresh interrupted')
    } catch (error) {
      catalogDirty ||= catalog
      for (const id of directories) dirtyDirectories.add(id)
      for (const id of stats) dirtyStats.add(id)
      for (const kind of footers) dirtyFooters.add(kind)
      throw error
    }
  }

  function resumeSidebarSync() {
    if (isDocumentVisible()) sidebarSync.resume()
    else {
      sidebarSync.pause()
      targetedController?.abort()
      for (const controller of pageRequests.values()) controller.abort()
    }
  }
  if (typeof document !== 'undefined') document.addEventListener?.('visibilitychange', resumeSidebarSync)

  function scheduleSidebarRecoverySync(reason: string, delayMs = 180, opts?: { force?: boolean }) {
    recoveryCheck = true
    // Recovery checks versions first; "force" cannot bypass the request budget.
    sidebarSync.invalidate(opts?.force ? delayMs : Math.max(180, delayMs))
    void reason
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
        page,
        search: query || undefined,
      })
      applyDirectoriesPagePayload({
        items: entries.map((entry) => ({ id: entry.id, path: entry.path })),
        total: hasMore ? page * pageSize + entries.length + 1 : page * pageSize + entries.length,
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
    if (evt.properties?.resource_revisions) return
    const type = readEventType(evt)
    if (!type) return
    const normalizedType = type.toLowerCase()

    if (normalizedType === 'chat-sidebar.delta') {
      applyChatSidebarDeltaEvent(evt)
      return
    }

    if (SIDEBAR_RECOVERY_EVENT_TYPES.has(normalizedType)) {
      const change = evt.properties
      if (normalizedType === 'session_changed' && (change?.kind === 'part_updated' || change?.kind === 'part_added')) {
        const part = change.part
        if (part && typeof part === 'object' && !Array.isArray(part) && part.kind !== 'run') return
      }
      if (normalizedType === 'runtime_signal') return
      scheduleSidebarRecoverySync(`event:${normalizedType}`, 180)
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

  const sectionLoads = new Map<string, Promise<boolean>>()
  function revalidateDirectorySessionPageFromApi(
    directoryId: string,
    opts?: { page?: number; pageSize?: number; silent?: boolean; refreshStats?: boolean },
  ): Promise<boolean> {
    const known = knownSidebarRowBySessionId()
    const expanded = uiPrefs.value.expandedParentSessionIds.filter(
      (id) =>
        !known[id] ||
        known[id]?.directory?.id === directoryId ||
        String(known[id]?.session?.workspace_id || '') === directoryId,
    )
    const key = `directory:${directoryId}:${JSON.stringify([
      opts?.page ?? uiPrefs.value.sessionRootPageByDirectoryId[directoryId] ?? 0,
      opts?.pageSize ?? persistedStateQuery.limitPerDirectory ?? SIDEBAR_DIRECTORY_SESSIONS_PAGE_SIZE,
      Boolean(opts?.refreshStats),
      uiPrefs.value.collapsedDirectoryIds.includes(directoryId),
      expanded.map((id) => [id, childPageById.get(id) ?? 0]),
      pinnedPageByDirectory.get(directoryId) ?? 0,
    ])}`
    const existing = sectionLoads.get(key)
    if (existing) return existing
    const promise = revalidateDirectorySessionPageFromApiImpl(directoryId, opts).finally(() => {
      if (sectionLoads.get(key) === promise) sectionLoads.delete(key)
    })
    sectionLoads.set(key, promise)
    return promise
  }
  function revalidateFooterFromApi(
    kind: SidebarFooterKind,
    opts?: { page?: number; pageSize?: number; silent?: boolean },
  ): Promise<boolean> {
    const key = `footer:${kind}:${JSON.stringify([
      opts?.page ?? uiPrefs.value[`${kind}SessionsPage`],
      opts?.pageSize ?? SIDEBAR_FOOTER_PAGE_SIZE,
      uiPrefs.value[`${kind}SessionsOpen`],
      uiPrefs.value.expandedParentSessionIds.map((id) => [id, childPageById.get(id) ?? 0]),
    ])}`
    const existing = sectionLoads.get(key)
    if (existing) return existing
    const promise = revalidateFooterFromApiImpl(kind, opts).finally(() => {
      if (sectionLoads.get(key) === promise) sectionLoads.delete(key)
    })
    sectionLoads.set(key, promise)
    return promise
  }

  async function revalidateDirectorySessionPageFromApiImpl(
    directoryId: string,
    opts?: { page?: number; pageSize?: number; silent?: boolean; refreshStats?: boolean },
  ): Promise<boolean> {
    const did = String(directoryId || '').trim()
    if (!did) return false
    const requestKey = `directory:${did}`
    pageRequests.get(requestKey)?.abort()
    const controller = new AbortController()
    pageRequests.set(requestKey, controller)
    const timeout = window.setTimeout(() => controller.abort(), 30_000)
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
      const page = Math.max(0, Math.floor(Number(requestedPageRaw || 0)))

      const workspace = directoriesById.value[did] || directoryEntryByPath(did, directoriesById.value)
      if (!workspace) return false
      const directory = { id: workspace.id, path: workspace.path }
      if (opts?.refreshStats) {
        invalidateResources([`workspace:${did}:sessions`, `workspace:${did}:stats`])
        await checkResourceVersions([`workspace:${did}:sessions`, `workspace:${did}:stats`], controller.signal)
        await loadWorkspaceStats([did], controller.signal)
      }
      const sectionBase = await loadDirectoryView(directory, page, pageSize, controller.signal)
      if (controller.signal.aborted) return false
      syncLoadedPinnedFlags(
        [...sectionBase.pinnedRows, ...sectionBase.recentRows].flatMap((row) => (row.session ? [row.session] : [])),
      )
      const section = enrichDirectorySidebarView(sectionBase, knownSidebarRowBySessionId())
      if (!section) return false
      for (const row of [...section.recentRows, ...section.pinnedRows])
        if (row.session) {
          const next = stateSnapshotFromAgenaSession(row.session)
          if (!stateSnapshotEquivalent(stateBySessionId.value[row.id], next)) stateBySessionId.value[row.id] = next
        }

      const previousSection = directorySidebarById.value[did]
      if (!previousSection || !directorySidebarViewEquivalent(previousSection, section)) {
        directorySidebarById.value = {
          ...directorySidebarById.value,
          [did]: section,
        }
      }
      syncWorkspaceSubscriptions(directoryPageRows.value)

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
      if (controller.signal.aborted) return false
      error.value = err instanceof Error ? err.message : String(err)
      return false
    } finally {
      window.clearTimeout(timeout)
      if (pageRequests.get(requestKey) === controller) {
        pageRequests.delete(requestKey)
        if (!opts?.silent) loading.value = false
        scheduleSidebarSessionHydration()
      }
    }
  }

  async function revalidateFooterFromApiImpl(
    kind: SidebarFooterKind,
    opts?: { page?: number; pageSize?: number; silent?: boolean },
  ): Promise<boolean> {
    const requestKey = `footer:${kind}`
    pageRequests.get(requestKey)?.abort()
    const controller = new AbortController()
    pageRequests.set(requestKey, controller)
    const timeout = window.setTimeout(() => controller.abort(), 30_000)
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
            : targetKind === 'favorite'
              ? uiPrefs.value.favoriteSessionsPage
              : targetKind === 'recent'
                ? uiPrefs.value.recentSessionsPage
                : uiPrefs.value.runningSessionsPage
      const page = Math.max(0, Math.floor(Number(pageRaw || 0)))
      const pageSizeRaw =
        typeof opts?.pageSize === 'number' && Number.isFinite(opts.pageSize) ? opts.pageSize : SIDEBAR_FOOTER_PAGE_SIZE
      const pageSize = Math.max(1, Math.floor(Number(pageSizeRaw || SIDEBAR_FOOTER_PAGE_SIZE)))

      const open = uiPrefs.value[`${kind}SessionsOpen`]
      const result = await loadFooterView(kind, open ? page : 0, open ? pageSize : 1, controller.signal)
      const loaded = open
        ? result
        : { ...result, rows: [], page, pageCount: Math.max(1, Math.ceil(result.total / pageSize)) }
      if (controller.signal.aborted) return false
      syncLoadedPinnedFlags(loaded.rows.flatMap((row) => (row.session ? [row.session] : [])))
      const view = enrichFooterView(loaded, knownSidebarRowBySessionId())
      for (const row of view.rows)
        if (row.session) {
          const next = stateSnapshotFromAgenaSession(row.session)
          if (!stateSnapshotEquivalent(stateBySessionId.value[row.id], next)) stateBySessionId.value[row.id] = next
        }

      if (targetKind === 'pinned') {
        if (!footerViewEquivalent(pinnedFooterView.value, view)) {
          pinnedFooterView.value = view
        }
      } else if (targetKind === 'favorite') {
        if (!footerViewEquivalent(favoriteFooterView.value, view)) {
          favoriteFooterView.value = view
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
      } else if (targetKind === 'favorite') {
        patch.favoriteSessionsPage = view.page
      } else if (targetKind === 'recent') {
        patch.recentSessionsPage = view.page
      } else {
        patch.runningSessionsPage = view.page
      }
      uiPrefs.value = normalizeUiPrefs(patchChatSidebarUiPrefs(uiPrefs.value, patch))
      syncWorkspaceSubscriptions(directoryPageRows.value)

      return true
    } catch (err) {
      if (controller.signal.aborted) return false
      error.value = err instanceof Error ? err.message : String(err)
      return false
    } finally {
      window.clearTimeout(timeout)
      if (pageRequests.get(requestKey) === controller) {
        pageRequests.delete(requestKey)
        if (!opts?.silent) loading.value = false
        scheduleSidebarSessionHydration()
      }
    }
  }

  async function setChildSessionPage(directoryId: string, sessionId: string, page: number) {
    sidebarStateRequestInFlight?.controller?.abort()
    sidebarStateRequestInFlight = null
    childPageById.delete(sessionId)
    childPageById.set(sessionId, Math.max(0, page))
    if (childPageById.size > 512) childPageById.delete(childPageById.keys().next().value!)
    const row = knownSidebarRowBySessionId()[sessionId]
    const did = directoryId || (row && knownDirectoryForSession(row)?.id) || ''
    await Promise.all([
      ...(directorySidebarById.value[did] ? [revalidateDirectorySessionPageFromApi(did, { silent: true })] : []),
      ...footerKinds
        .filter((kind) => footerViewForKind(kind).rows.some((row) => row.id === sessionId))
        .map((kind) => revalidateFooterFromApi(kind, { silent: true })),
    ])
  }
  async function setDirectoryPinnedPage(directoryId: string, page: number) {
    sidebarStateRequestInFlight?.controller?.abort()
    sidebarStateRequestInFlight = null
    pinnedPageByDirectory.set(directoryId, Math.max(0, page))
    await revalidateDirectorySessionPageFromApi(directoryId, { silent: true })
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
    const nestedSession = asRecord((located?.session ?? null) as JsonValue)
    const locatedSession = nestedSession || located
    const locatedSessionId = readLocatedSessionId(locatedSession)
    const canUseRemoteLocate = !locatedSessionId || locatedSessionId === sid
    const rawPid = locatedSession?.workspace_id
    const rawPath = located?.path

    const pid =
      canUseRemoteLocate && (typeof rawPid === 'number' || typeof rawPid === 'string') ? String(rawPid).trim() : ''
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
      const workspaceId = Number(pid)
      if (Number.isSafeInteger(workspaceId) && workspaceId > 0) {
        const workspace = await chatApi.getWorkspace(workspaceId).catch(() => null)
        if (workspace?.path) {
          return {
            directoryId: String(workspace.id),
            directoryPath: workspace.path,
            locatedDir: locatePath || workspace.path,
          }
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
    const snapshot = stateBySessionId.value[sid]
    const state = snapshot?.state
    const kind = sessionStateKind(state)
    if (state?.kind === 'awaiting_interaction') {
      const request = state.data.requests?.[0]
      const requestKind =
        request && typeof request === 'object' && !Array.isArray(request)
          ? (request as Record<string, unknown>).kind
          : undefined
      if (requestKind === 'permission') {
        return {
          label: String(i18n.global.t('chat.sidebar.sessionRow.status.needsPermission')),
          dotClass: 'bg-amber-500',
        }
      }
      return {
        label: String(i18n.global.t('chat.sidebar.sessionRow.status.needsReply')),
        dotClass: 'bg-sky-500',
      }
    }
    if (kind === 'failed') {
      return {
        label: String(i18n.global.t('chat.sidebar.sessionRow.status.needsRecovery')),
        dotClass: 'bg-destructive',
      }
    }
    if (kind === 'running' || kind === 'creating') {
      return {
        label: String(i18n.global.t('chat.sidebar.sessionRow.status.running')),
        dotClass: 'bg-primary animate-pulse',
      }
    }
    return { label: String(i18n.global.t('chat.sidebar.sessionRow.status.idle')), dotClass: '' }
  }

  function isSessionStateActive(sessionId: string): boolean {
    const sid = String(sessionId || '').trim()
    if (!sid) return false
    return sessionStateIsActive(stateBySessionId.value[sid])
  }

  async function bootstrapWithStaleWhileRevalidate() {
    await revalidateFromApi()
  }

  async function resetAllPersistedState() {
    targetedController?.abort()
    for (const controller of pageRequests.values()) controller.abort()
    pageRequests.clear()
    for (const timer of expandedChildRecoveryTimers.values()) window.clearTimeout(timer)
    expandedChildRecoveryTimers.clear()
    expandedChildRecoveryAttempts.clear()
    sectionLoads.clear()
    workspaceStats.clear()
    workspaceStatsVersions.clear()
    directoryPages.clear()
    sidebarInitialized = false
    catalogDirty = false
    recoveryCheck = false
    dirtyDirectories.clear()
    dirtyStats.clear()
    dirtyFooters.clear()
    childPageById.clear()
    pinnedPageByDirectory.clear()
    for (const release of workspaceSubscriptions.values()) release()
    workspaceSubscriptions.clear()
    sidebarSync.dispose()
    sidebarSync = createSidebarSync()

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
    stateBySessionId.value = {}

    directorySidebarById.value = {}
    pinnedFooterView.value = { total: 0, page: 0, pageCount: 1, rows: [] }
    favoriteFooterView.value = { total: 0, page: 0, pageCount: 1, rows: [] }
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
    stateBySessionId,
    directorySidebarById,
    pinnedFooterView,
    favoriteFooterView,
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
    setChildSessionPage,
    setDirectoryPinnedPage,
    revalidateFooterFromApi,
    commandSetDirectoriesPage,
    commandSetDirectoryCollapsed,
    commandSetDirectoryRootPage,
    commandSetSessionPinned,
    commandSetSessionExpanded,
    commandExpandSessionAncestors,
    commandSetFooterOpen,
    commandSetFooterPage,
    resolveDirectoryForSession,
    statusLabelForSessionId,
    isSessionStateActive,
    applyGlobalEvent,
    scheduleSidebarRecoverySync,
    revalidateFromApi,
    bootstrapWithStaleWhileRevalidate,
    resetAllPersistedState,
  }
})
