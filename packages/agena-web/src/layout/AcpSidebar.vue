<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute, useRouter } from 'vue-router'
import {
  RiArrowRightUpLine,
  RiDeleteBinLine,
  RiEditLine,
  RiFileCopyLine,
  RiGitBranchLine,
  RiInboxArchiveLine,
  RiInboxUnarchiveLine,
} from '@remixicon/vue'

import { readSessionIdFromQuery } from '@/app/navigation/sessionQuery'
import { useWorkspaceNavigation } from '@/app/navigation/useWorkspaceNavigation'
import AcpInstallGuideCard from '@/components/acp/AcpInstallGuideCard.vue'
import RenameSessionDialog from '@/components/chat/RenameSessionDialog.vue'
import { acpAgentsSettingsDestination } from '@/components/settings/settingsDestination'
import OptionMenu from '@/components/ui/OptionMenu.vue'
import SidebarTextButton from '@/components/ui/SidebarTextButton.vue'
import type { OptionMenuGroup, OptionMenuItem } from '@/components/ui/optionMenu.types'
import {
  deleteNativeAgentSession,
  listExternalAgents,
  listNativeAgentSessions,
  manageNativeAgentSession,
  openNativeAgentSession,
  renameNativeAgentSession,
  setNativeAgentArchived,
  useExternalAgentCatalog,
  watchNativeAgentSessions,
  type NativeIndexCapabilities,
  type NativeIndexedSession,
  type NativeSessionStatus,
} from '@/features/sessions/api/nativeAgentSessions'
import type { DirectoryEntry } from '@/features/sessions/model/types'
import AddDirectoryDialog from '@/layout/chatSidebar/components/AddDirectoryDialog.vue'
import SidebarSectionSkeleton from '@/layout/chatSidebar/components/SidebarSectionSkeleton.vue'
import { isMissingAcpCommandError } from '@/lib/acpInstallGuide'
import { isAcpSessionId } from '@/lib/acpSession'
import { copyTextToClipboard } from '@/lib/clipboard'
import { vKeyboardCollection } from '@/lib/keyboardCollection'
import { getLocalJson, getLocalString, setLocalJson, setLocalString } from '@/lib/persist'
import { localStorageKeys } from '@/lib/persistence/storageKeys'
import { subscribeServerAuthTokenVersion } from '@/lib/serverAuthToken'
import { useChatStoreUnscoped } from '@/stores/chat'
import * as chatApi from '@/stores/chat/api'
import { useDirectoryStore } from '@/stores/directory'
import { fetchAgenaWorkspaces } from '@/stores/directorySessionStore'
import { useToastsStore } from '@/stores/toasts'
import { useUiStore } from '@/stores/ui'
import { formatExternalAgentLabel, type ExternalAgent } from '@/types/chat'

import AcpConversationRow from './acpSidebar/AcpConversationRow.vue'
import AcpProjectRow from './acpSidebar/AcpProjectRow.vue'
import AcpSidebarHeader from './acpSidebar/AcpSidebarHeader.vue'
import {
  invalidateCachedAcpProject,
  readCachedAcpProject,
  writeCachedAcpProject,
} from './acpSidebar/acpSidebarCache'
import {
  acpDirectoryActivityState,
  filterAcpDirectories,
  mergeProjectConversations,
  type AcpConversationRow as AcpConversationRowModel,
  type AcpDraftSession,
} from './acpSidebar/acpSidebarModel'

type ProjectState = {
  sessions: NativeIndexedSession[]
  cursor: string | null
  capabilities: NativeIndexCapabilities | null
  loading: boolean
  loaded: boolean
  error: string
  showArchived: boolean
}

const { t } = useI18n()
const route = useRoute()
const router = useRouter()
const ui = useUiStore()
const chat = useChatStoreUnscoped()
const directoryStore = useDirectoryStore()
const toasts = useToastsStore()
const navigation = useWorkspaceNavigation()

const agents = useExternalAgentCatalog()
const savedAgent = getLocalString(localStorageKeys.ui.acpSidebarAgent).trim()
const agent = ref<ExternalAgent>(savedAgent || agents.value[0]?.id || 'claude-code')
const projects = ref<DirectoryEntry[]>([])
const projectsLoading = ref(true)
const projectsError = ref('')
const query = ref('')
const recheckingAgent = ref(false)
const creatingProjectId = ref('')
const openingRowKey = ref('')
const busyRowKey = ref('')

// Add directory dialog
const isAddDirectoryOpen = ref(false)
const newDirectoryPath = ref('')

// Expanded directories (persisted)
const savedExpanded = getLocalJson<string[]>(localStorageKeys.ui.acpSidebarExpandedDirs, [])
const hasSavedExpanded = ref(Array.isArray(savedExpanded) && savedExpanded.length > 0)
const expandedDirectoryIds = ref<Set<string>>(
  new Set(Array.isArray(savedExpanded) ? savedExpanded.map((id) => String(id).trim()).filter(Boolean) : []),
)

// Per-project conversation state (keyed by directory.id)
const projectStates = reactive<Record<string, ProjectState>>({})
const readControllers = new Map<string, AbortController>()
const watchControllers = new Map<string, AbortController>()
const refreshPendingIds = new Set<string>()

// Unified draft & native<->ephemeral session linkage
const draftSessions = ref<AcpDraftSession[]>([])
const sessionIdByNativeId = ref<Map<string, string>>(new Map())
const nativeIdBySessionId = ref<Map<string, string>>(new Map())
const statusBySessionId = ref<Map<string, NativeSessionStatus>>(new Map())
const statusLoadingIds = ref<Set<string>>(new Set())

// Row overflow / context menu
const menuOpen = ref(false)
const menuAnchorEl = ref<HTMLElement | null>(null)
const menuTarget = ref<AcpConversationRowModel | null>(null)

// Rename (inline on desktop like ChatSidebar, dialog on compact/mobile)
const renameOpen = ref(false)
const renameInlineKey = ref('')
const renameDraft = ref('')
const renameBusy = ref(false)
const renameTarget = ref<AcpConversationRowModel | null>(null)

const lifetime = new AbortController()

const currentId = computed(() => readSessionIdFromQuery(route.query))
const current = computed(() => (isAcpSessionId(currentId.value) ? chat.getSessionById(currentId.value) : null))
const activeAgentEntry = computed(() => agents.value.find((entry) => entry.id === agent.value))
const isAgentUnavailable = computed(() => activeAgentEntry.value?.available === false)
const agentLabel = (id: ExternalAgent) => formatExternalAgentLabel(id, agents.value)

function persistExpanded() {
  setLocalJson(localStorageKeys.ui.acpSidebarExpandedDirs, [...expandedDirectoryIds.value])
}

function ensureProjectState(workspaceId: string): ProjectState {
  if (!projectStates[workspaceId]) {
    projectStates[workspaceId] = {
      sessions: [],
      cursor: null,
      capabilities: null,
      loading: false,
      loaded: false,
      error: '',
      showArchived: false,
    }
  }
  return projectStates[workspaceId]
}

function hydrateProjectFromCache(workspaceId: string): boolean {
  if (!agent.value || isAgentUnavailable.value) return false
  const st = ensureProjectState(workspaceId)
  if (st.loaded) return true
  const cached = readCachedAcpProject(agent.value, workspaceId)
  if (!cached) return false
  st.sessions = cached.sessions
  st.cursor = cached.cursor
  st.capabilities = cached.capabilities
  st.showArchived = cached.showArchived
  st.loaded = true
  return true
}

function isDirectoryExpanded(directoryId: string): boolean {
  return expandedDirectoryIds.value.has(directoryId)
}

function setDirectoryExpanded(directoryId: string, expanded: boolean) {
  const next = new Set(expandedDirectoryIds.value)
  if (expanded) next.add(directoryId)
  else next.delete(directoryId)
  expandedDirectoryIds.value = next
  hasSavedExpanded.value = true
  persistExpanded()
}

function isDirectoryFocused(directory: DirectoryEntry): boolean {
  if (current.value?.workspace_id != null) {
    return String(current.value.workspace_id) === directory.id
  }
  const activeDir = (directoryStore.currentDirectory || '').trim()
  return Boolean(activeDir && directory.path === activeDir)
}

function abortProjectRequests(workspaceId: string) {
  readControllers.get(workspaceId)?.abort()
  readControllers.delete(workspaceId)
  watchControllers.get(workspaceId)?.abort()
  watchControllers.delete(workspaceId)
  refreshPendingIds.delete(workspaceId)
}

function abortAllProjectRequests() {
  for (const controller of readControllers.values()) controller.abort()
  readControllers.clear()
  for (const controller of watchControllers.values()) controller.abort()
  watchControllers.clear()
  refreshPendingIds.clear()
}

function resetAllProjectStates() {
  abortAllProjectRequests()
  for (const key of Object.keys(projectStates)) {
    delete projectStates[key]
  }
}

function linkNativeSession(sessionId: string, nativeId: string) {
  const nextByNative = new Map(sessionIdByNativeId.value)
  nextByNative.set(nativeId, sessionId)
  sessionIdByNativeId.value = nextByNative

  const nextBySession = new Map(nativeIdBySessionId.value)
  nextBySession.set(sessionId, nativeId)
  nativeIdBySessionId.value = nextBySession
}

function unlinkSession(sessionId: string) {
  const linkedNative = nativeIdBySessionId.value.get(sessionId)
  if (linkedNative) {
    const nextByNative = new Map(sessionIdByNativeId.value)
    nextByNative.delete(linkedNative)
    sessionIdByNativeId.value = nextByNative

    const nextBySession = new Map(nativeIdBySessionId.value)
    nextBySession.delete(sessionId)
    nativeIdBySessionId.value = nextBySession
  }
  const nextStatus = new Map(statusBySessionId.value)
  nextStatus.delete(sessionId)
  statusBySessionId.value = nextStatus
  draftSessions.value = draftSessions.value.filter((item) => item.sessionId !== sessionId)
}

function upsertDraft(draft: AcpDraftSession) {
  const existingIndex = draftSessions.value.findIndex((item) => item.sessionId === draft.sessionId)
  if (existingIndex >= 0) {
    const next = draftSessions.value.slice()
    next[existingIndex] = { ...next[existingIndex], ...draft }
    draftSessions.value = next
  } else {
    draftSessions.value = [draft, ...draftSessions.value]
  }
}

function rowsForProject(workspaceId: string, filterQuery = query.value): AcpConversationRowModel[] {
  const st = projectStates[workspaceId]
  return mergeProjectConversations({
    workspaceId: Number(workspaceId),
    agent: agent.value,
    nativeSessions: st?.sessions ?? [],
    capabilities: st?.capabilities ?? null,
    drafts: draftSessions.value,
    sessionIdByNativeId: sessionIdByNativeId.value,
    nativeIdBySessionId: nativeIdBySessionId.value,
    query: filterQuery,
  })
}

const visibleProjects = computed(() =>
  filterAcpDirectories(projects.value, query.value, (dirId) => rowsForProject(dirId, query.value).length > 0),
)

function isRowSelected(row: AcpConversationRowModel): boolean {
  const activeId = currentId.value
  if (!activeId) return false
  if (row.sessionId && row.sessionId === activeId) return true
  if (row.nativeId && nativeIdBySessionId.value.get(activeId) === row.nativeId) return true
  return false
}

function refreshProjectWhenIdle(workspaceId: string) {
  const st = projectStates[workspaceId]
  if (refreshPendingIds.has(workspaceId) && st && !st.loading && !busyRowKey.value) {
    void loadProject(workspaceId, false)
  }
}

function followProject(workspaceId: string) {
  if (watchControllers.has(workspaceId)) return
  const st = projectStates[workspaceId]
  if (!st?.capabilities?.watch || isAgentUnavailable.value) return
  const controller = new AbortController()
  watchControllers.set(workspaceId, controller)
  const targetAgent = agent.value
  const numericWorkspace = Number(workspaceId)

  void (async () => {
    try {
      while (!controller.signal.aborted && !lifetime.signal.aborted && agent.value === targetAgent) {
        const change = await watchNativeAgentSessions(numericWorkspace, targetAgent, controller.signal)
        if (controller.signal.aborted || lifetime.signal.aborted || !change.supported) break
        if (change.changed) {
          refreshPendingIds.add(workspaceId)
          refreshProjectWhenIdle(workspaceId)
          await new Promise((resolve) => window.setTimeout(resolve, 300))
        }
      }
    } catch {
      // Manual refresh remains available if watch stops.
    } finally {
      if (watchControllers.get(workspaceId) === controller) {
        watchControllers.delete(workspaceId)
      }
    }
  })()
}

async function loadProject(workspaceId: string, append = false) {
  if (!agent.value || isAgentUnavailable.value) return
  if (!append) hydrateProjectFromCache(workspaceId)
  const st = ensureProjectState(workspaceId)
  readControllers.get(workspaceId)?.abort()
  const controller = new AbortController()
  readControllers.set(workspaceId, controller)
  const targetAgent = agent.value

  st.loading = true
  st.error = ''
  refreshPendingIds.delete(workspaceId)

  try {
    const page = await listNativeAgentSessions(
      Number(workspaceId),
      targetAgent,
      append ? st.cursor : null,
      st.showArchived ? 'all' : 'unarchived',
      controller.signal,
    )
    if (controller.signal.aborted || lifetime.signal.aborted || agent.value !== targetAgent) return
    if (append) {
      const known = new Set(st.sessions.map((item) => item.native_id))
      st.sessions = [...st.sessions, ...page.sessions.filter((item) => !known.has(item.native_id))]
    } else {
      st.sessions = page.sessions
    }
    st.cursor = page.next_cursor
    st.capabilities = page.capabilities
    st.loaded = true
    writeCachedAcpProject(targetAgent, workspaceId, {
      sessions: st.sessions,
      cursor: st.cursor,
      capabilities: st.capabilities,
      showArchived: st.showArchived,
    })
    if (page.capabilities.watch) followProject(workspaceId)
    else {
      watchControllers.get(workspaceId)?.abort()
      watchControllers.delete(workspaceId)
    }
  } catch (failure) {
    if (!controller.signal.aborted && !lifetime.signal.aborted && agent.value === targetAgent) {
      st.error = failure instanceof Error ? failure.message : String(failure)
    }
  } finally {
    if (readControllers.get(workspaceId) === controller) {
      readControllers.delete(workspaceId)
      st.loading = false
      refreshProjectWhenIdle(workspaceId)
    }
  }
}

function toggleDirectoryCollapse(directory: DirectoryEntry) {
  const expanded = !isDirectoryExpanded(directory.id)
  setDirectoryExpanded(directory.id, expanded)
  if (expanded) {
    const st = projectStates[directory.id]
    if (!st?.loaded && !st?.loading) {
      void loadProject(directory.id, false)
    }
  } else {
    abortProjectRequests(directory.id)
  }
}

function toggleProjectArchived(workspaceId: string) {
  const st = ensureProjectState(workspaceId)
  st.showArchived = !st.showArchived
  st.cursor = null
  void loadProject(workspaceId, false)
}

async function loadAgents() {
  try {
    const items = await listExternalAgents(lifetime.signal)
    if (lifetime.signal.aborted) return
    if (items.length && !items.some((entry) => entry.id === agent.value)) {
      agent.value = items[0].id
    }
  } catch {
    // Keep built-in defaults if catalog fetch fails.
  }
}

async function recheckActiveAgent() {
  if (recheckingAgent.value) return
  recheckingAgent.value = true
  try {
    await listExternalAgents(lifetime.signal)
    if (!lifetime.signal.aborted && !isAgentUnavailable.value) {
      for (const dir of projects.value) {
        if (isDirectoryExpanded(dir.id)) {
          void loadProject(dir.id, false)
        }
      }
    }
  } finally {
    if (!lifetime.signal.aborted) recheckingAgent.value = false
  }
}

async function loadProjects(options: { loadExpanded?: boolean } = {}) {
  const shouldLoadExpanded = options.loadExpanded ?? true
  projectsError.value = ''
  projectsLoading.value = true
  try {
    const entries: DirectoryEntry[] = []
    let page = 0
    let hasMore = true
    while (hasMore && !lifetime.signal.aborted) {
      const result = await fetchAgenaWorkspaces({ limit: 200, page: page++, signal: lifetime.signal })
      entries.push(...result.entries)
      hasMore = result.hasMore
    }
    if (lifetime.signal.aborted) return
    projects.value = entries

    // Ensure at least the active (or first) directory is expanded on initial visit.
    const validIds = new Set(entries.map((entry) => entry.id))
    const pruned = new Set([...expandedDirectoryIds.value].filter((id) => validIds.has(id)))
    if (!hasSavedExpanded.value || (pruned.size === 0 && entries.length > 0)) {
      const preferred =
        entries.find((entry) => current.value?.workspace_id != null && entry.id === String(current.value.workspace_id)) ||
        entries.find((entry) => entry.path === directoryStore.currentDirectory) ||
        entries[0]
      if (preferred) pruned.add(preferred.id)
    }
    expandedDirectoryIds.value = pruned

    if (shouldLoadExpanded && !isAgentUnavailable.value) {
      for (const id of expandedDirectoryIds.value) {
        if (!projectStates[id]?.loaded && !projectStates[id]?.loading) {
          void loadProject(id, false)
        }
      }
    }
  } catch (failure) {
    if (!lifetime.signal.aborted) {
      projectsError.value = failure instanceof Error ? failure.message : String(failure)
    }
  } finally {
    if (!lifetime.signal.aborted) projectsLoading.value = false
  }
}

async function refreshAll() {
  await Promise.all([loadAgents(), loadProjects({ loadExpanded: false })])
  if (!isAgentUnavailable.value) {
    for (const dir of projects.value) {
      if (isDirectoryExpanded(dir.id)) {
        void loadProject(dir.id, false)
      }
    }
  }
}

async function handleAddDirectory() {
  const path = newDirectoryPath.value.trim()
  if (!path) return
  try {
    const createdWorkspaceId = await chatApi.createWorkspace(path)
    newDirectoryPath.value = ''
    isAddDirectoryOpen.value = false
    await loadProjects()
    const createdId =
      Number.isFinite(createdWorkspaceId) && createdWorkspaceId > 0
        ? String(createdWorkspaceId)
        : projects.value.find((d) => d.path === path)?.id
    if (createdId) {
      setDirectoryExpanded(createdId, true)
      void loadProject(createdId, false)
    }
  } catch (failure) {
    toasts.push('error', failure instanceof Error ? failure.message : String(failure))
  }
}

function openAcpSettings() {
  const destination = acpAgentsSettingsDestination(route.query, agent.value)
  if (ui.isCompactLayout) {
    ui.setSessionSwitcherOpen(false)
    void router.push(destination)
  } else {
    void navigation.openMainTab('settings', destination)
  }
}

async function navigateToSession(sessionId?: string | number) {
  if (lifetime.signal.aborted) return
  await navigation.openMainTab('acp', {
    path: '/acp',
    query: sessionId ? { sessionId: String(sessionId) } : {},
    replace: true,
  })
  if (sessionId) ui.setSessionSwitcherOpen(false)
}

async function newSessionInProject(directory: DirectoryEntry) {
  if (creatingProjectId.value || !agent.value || isAgentUnavailable.value) return
  creatingProjectId.value = directory.id
  try {
    const created = await chatApi.createSession({
      workspaceId: Number(directory.id),
      externalAgent: agent.value,
      title: agentLabel(agent.value),
      autoTitle: true,
    })
    if (lifetime.signal.aborted) return
    const sid = String(created.id)
    upsertDraft({
      sessionId: sid,
      workspaceId: Number(directory.id),
      agent: agent.value,
      title: created.title || agentLabel(agent.value),
      updatedAtMs: Date.now(),
    })
    if (!isDirectoryExpanded(directory.id)) {
      setDirectoryExpanded(directory.id, true)
    }
    await navigateToSession(sid)
  } catch (failure) {
    toasts.push('error', failure instanceof Error ? failure.message : String(failure))
  } finally {
    creatingProjectId.value = ''
  }
}

async function openRow(row: AcpConversationRowModel) {
  if (openingRowKey.value || busyRowKey.value) return
  if (row.kind === 'draft' && row.sessionId) {
    await navigateToSession(row.sessionId)
    return
  }
  if (!row.nativeId) return
  openingRowKey.value = row.key
  try {
    const id = await openNativeAgentSession(row.workspaceId, row.agent, row.nativeId, lifetime.signal)
    if (lifetime.signal.aborted) return
    const sid = String(id)
    linkNativeSession(sid, row.nativeId)
    await navigateToSession(sid)
  } catch (failure) {
    if (!lifetime.signal.aborted) {
      toasts.push('error', failure instanceof Error ? failure.message : String(failure))
    }
  } finally {
    if (openingRowKey.value === row.key) openingRowKey.value = ''
  }
}

async function inspectSessionStatus(sessionId: string, force = false): Promise<NativeSessionStatus | null> {
  if (!sessionId || !isAcpSessionId(sessionId) || isAgentUnavailable.value) return null
  if (!force && statusBySessionId.value.has(sessionId)) {
    return statusBySessionId.value.get(sessionId) ?? null
  }
  if (statusLoadingIds.value.has(sessionId)) return null
  const nextLoading = new Set(statusLoadingIds.value)
  nextLoading.add(sessionId)
  statusLoadingIds.value = nextLoading
  try {
    const result = await manageNativeAgentSession(sessionId, { action: 'status' }, lifetime.signal)
    if (lifetime.signal.aborted || result.kind !== 'status') return null
    const nextStatus = new Map(statusBySessionId.value)
    nextStatus.set(sessionId, result)
    statusBySessionId.value = nextStatus
    if (result.session?.native_id) {
      linkNativeSession(sessionId, result.session.native_id)
      draftSessions.value = draftSessions.value.filter((item) => item.sessionId !== sessionId)
    }
    return result
  } catch {
    return null
  } finally {
    const next = new Set(statusLoadingIds.value)
    next.delete(sessionId)
    statusLoadingIds.value = next
  }
}

function canToggleArchive(row: AcpConversationRowModel): boolean {
  if (!row.nativeId || !row.capabilities?.archive) return false
  if (row.capabilities.index && (row.state === 'running' || row.state === 'requires_action')) return false
  return true
}

function statusForRow(row: AcpConversationRowModel): NativeSessionStatus | undefined {
  const sid = row.sessionId || (isRowSelected(row) ? currentId.value : '')
  return sid ? statusBySessionId.value.get(sid) : undefined
}

function canRenameRow(row: AcpConversationRowModel): boolean {
  if (row.kind === 'draft') return true
  if (row.capabilities?.title) return true
  const status = statusForRow(row)
  return Boolean(status?.session && status.capabilities.title)
}

function canDeleteRow(row: AcpConversationRowModel): boolean {
  if (row.kind === 'draft') return Boolean(row.sessionId)
  if (row.capabilities?.delete) return true
  const status = statusForRow(row)
  return Boolean(status?.session && status.capabilities.delete)
}

function canForkRow(row: AcpConversationRowModel): boolean {
  if (row.kind !== 'native' || !row.nativeId) return false
  if (row.capabilities?.fork) return true
  const status = statusForRow(row)
  return Boolean(status?.session && status.capabilities.fork)
}

function startRenameRow(row: AcpConversationRowModel) {
  if (!canRenameRow(row) || renameBusy.value) return
  renameTarget.value = row
  renameDraft.value = row.title
  if (ui.isCompactLayout) {
    renameInlineKey.value = ''
    renameOpen.value = true
  } else {
    renameOpen.value = false
    renameInlineKey.value = row.key
  }
}

function cancelRename() {
  if (renameBusy.value) return
  renameOpen.value = false
  renameInlineKey.value = ''
  renameTarget.value = null
}

async function deleteRow(row: AcpConversationRowModel) {
  if (busyRowKey.value || !canDeleteRow(row)) return
  const effectiveSessionId = row.sessionId || (isRowSelected(row) ? currentId.value : '')
  busyRowKey.value = row.key
  try {
    if (row.kind === 'draft' && effectiveSessionId) {
      await chat.deleteSession(effectiveSessionId)
      unlinkSession(effectiveSessionId)
      if (currentId.value === effectiveSessionId) {
        await navigateToSession()
      }
      return
    }
    if (row.nativeId) {
      await deleteNativeAgentSession(row.workspaceId, row.agent, row.nativeId)
      toasts.push('success', String(t('chat.nativeAgent.deletedToast')))
      if (effectiveSessionId) unlinkSession(effectiveSessionId)
      await loadProject(String(row.workspaceId), false)
      if (effectiveSessionId && currentId.value === effectiveSessionId) {
        await navigateToSession()
      }
    }
  } catch (failure) {
    toasts.push('error', failure instanceof Error ? failure.message : String(failure))
  } finally {
    if (busyRowKey.value === row.key) busyRowKey.value = ''
    refreshProjectWhenIdle(String(row.workspaceId))
  }
}

async function toggleRowArchive(row: AcpConversationRowModel) {
  if (!row.nativeId || busyRowKey.value || !canToggleArchive(row)) return
  busyRowKey.value = row.key
  try {
    await setNativeAgentArchived(row.workspaceId, row.agent, row.nativeId, !row.archived)
    await loadProject(String(row.workspaceId), false)
  } catch (failure) {
    toasts.push('error', failure instanceof Error ? failure.message : String(failure))
  } finally {
    if (busyRowKey.value === row.key) busyRowKey.value = ''
    refreshProjectWhenIdle(String(row.workspaceId))
  }
}

function openRowMenu(row: AcpConversationRowModel, event: MouseEvent | PointerEvent) {
  menuTarget.value = row
  menuAnchorEl.value = event.currentTarget instanceof HTMLElement ? event.currentTarget : null
  menuOpen.value = true
  const sid = row.sessionId || (isRowSelected(row) ? currentId.value : '')
  if (sid && isAcpSessionId(sid)) {
    void inspectSessionStatus(sid)
  }
}

function closeRowMenu() {
  menuOpen.value = false
  menuTarget.value = null
  menuAnchorEl.value = null
}

const menuGroups = computed<OptionMenuGroup[]>(() => {
  const row = menuTarget.value
  if (!row) return []
  const items: OptionMenuItem[] = []
  const isBusy = row.state === 'running' || row.state === 'requires_action'

  if (!isRowSelected(row)) {
    items.push({
      id: 'open',
      label: String(t('common.open')),
      icon: RiArrowRightUpLine,
    })
  }

  if (canRenameRow(row)) {
    items.push({
      id: 'rename',
      label: String(t('chat.sidebar.sessionActions.rename.label')),
      icon: RiEditLine,
      disabled: isBusy,
    })
  }

  if (canForkRow(row)) {
    items.push({
      id: 'fork',
      label: String(t('chat.sidebar.sessionActions.fork.label')),
      icon: RiGitBranchLine,
      disabled: isBusy,
    })
  }

  if (canToggleArchive(row)) {
    items.push({
      id: 'toggle-archive',
      label: String(t(row.archived ? 'chat.nativeAgent.unarchive' : 'chat.nativeAgent.archive')),
      icon: row.archived ? RiInboxUnarchiveLine : RiInboxArchiveLine,
    })
  }

  if (row.nativeId) {
    items.push({
      id: 'copy-id',
      label: String(t('chat.nativeAgent.copyId')),
      icon: RiFileCopyLine,
    })
  }

  if (canDeleteRow(row)) {
    items.push({
      id: 'delete',
      label: String(t('chat.sidebar.sessionActions.delete.label')),
      icon: RiDeleteBinLine,
      variant: 'destructive',
      disabled: isBusy,
      confirmTitle: String(
        t(row.kind === 'draft' ? 'chat.sidebar.sessionActions.delete.confirmTitle' : 'chat.nativeAgent.delete'),
      ),
      confirmDescription: String(
        t(
          row.kind === 'draft'
            ? 'chat.sidebar.sessionActions.delete.confirmDescription'
            : 'chat.nativeAgent.deleteDescription',
        ),
      ),
      confirmText: String(t('common.delete')),
      cancelText: String(t('common.cancel')),
    })
  }

  return items.length ? [{ id: 'acp-session-actions', items }] : []
})

async function runRowAction(item: OptionMenuItem) {
  const row = menuTarget.value
  closeRowMenu()
  if (!row) return

  if (item.id === 'open') {
    await openRow(row)
    return
  }

  if (item.id === 'rename') {
    startRenameRow(row)
    return
  }

  if (item.id === 'toggle-archive') {
    await toggleRowArchive(row)
    return
  }

  if (item.id === 'copy-id' && row.nativeId) {
    const ok = await copyTextToClipboard(row.nativeId)
    if (ok) toasts.push('success', String(t('common.copied')))
    return
  }

  if (item.id === 'fork' && row.nativeId) {
    busyRowKey.value = row.key
    try {
      let sid = row.sessionId || (isRowSelected(row) ? currentId.value : '')
      if (!sid) {
        const openedId = await openNativeAgentSession(row.workspaceId, row.agent, row.nativeId, lifetime.signal)
        sid = String(openedId)
        linkNativeSession(sid, row.nativeId)
      }
      const result = await manageNativeAgentSession(sid, { action: 'fork' })
      if (result.kind === 'forked') {
        toasts.push('success', String(t('chat.nativeAgent.forkedToast')))
        await loadProject(String(row.workspaceId), false)
        await navigateToSession(result.session_id)
      }
    } catch (failure) {
      toasts.push('error', failure instanceof Error ? failure.message : String(failure))
    } finally {
      if (busyRowKey.value === row.key) busyRowKey.value = ''
    }
    return
  }

  if (item.id === 'delete') {
    await deleteRow(row)
  }
}

async function saveRename() {
  const row = renameTarget.value
  const nextTitle = renameDraft.value.trim()
  if (!row || !nextTitle || renameBusy.value) return
  if (nextTitle === row.title) {
    cancelRename()
    return
  }
  const effectiveSessionId = row.sessionId || (isRowSelected(row) ? currentId.value : '')

  renameBusy.value = true
  try {
    if (row.kind === 'draft' && effectiveSessionId) {
      await chat.renameSession(effectiveSessionId, nextTitle)
      upsertDraft({
        sessionId: effectiveSessionId,
        workspaceId: row.workspaceId,
        agent: row.agent,
        title: nextTitle,
        updatedAtMs: Date.now(),
      })
    } else if (row.nativeId) {
      const saved = await renameNativeAgentSession(row.workspaceId, row.agent, row.nativeId, nextTitle)
      if (effectiveSessionId) {
        const cached = chat.getSessionById(effectiveSessionId)
        if (cached) cached.title = saved.title
      }
      await loadProject(String(row.workspaceId), false)
    }
    toasts.push('success', String(t('chat.nativeAgent.renamedToast')))
    renameOpen.value = false
    renameInlineKey.value = ''
    renameTarget.value = null
  } catch (failure) {
    toasts.push('error', failure instanceof Error ? failure.message : String(failure))
  } finally {
    renameBusy.value = false
  }
}

watch(agent, (nextAgent) => {
  setLocalString(localStorageKeys.ui.acpSidebarAgent, nextAgent)
  closeRowMenu()
  cancelRename()
  resetAllProjectStates()
  if (!isAgentUnavailable.value) {
    for (const dir of projects.value) {
      if (isDirectoryExpanded(dir.id)) {
        void loadProject(dir.id, false)
      }
    }
  }
})

// Keep active ACP session synchronized (expand its project, highlight row, promote draft once started).
watch(
  () => [currentId.value, current.value?.workspace_id, current.value?.external_agent, current.value?.title] as const,
  ([sid, workspaceId, extAgent, title]) => {
    if (!sid || !isAcpSessionId(sid)) return
    if (extAgent && agents.value.some((entry) => entry.id === extAgent) && agent.value !== extAgent) {
      agent.value = extAgent
    }
    if (workspaceId != null) {
      const wid = String(workspaceId)
      if (projects.value.some((dir) => dir.id === wid) && !isDirectoryExpanded(wid)) {
        setDirectoryExpanded(wid, true)
        if (!projectStates[wid]?.loaded && !projectStates[wid]?.loading && !isAgentUnavailable.value) {
          void loadProject(wid, false)
        }
      }
    }
    const existingDraft = draftSessions.value.find((item) => item.sessionId === sid)
    if (existingDraft && title && title !== existingDraft.title) {
      upsertDraft({ ...existingDraft, title, updatedAtMs: Date.now() })
    }
    if (!nativeIdBySessionId.value.has(sid) && extAgent === agent.value && !isAgentUnavailable.value) {
      void inspectSessionStatus(sid).then((status) => {
        if (!status && workspaceId != null && extAgent) {
          upsertDraft({
            sessionId: sid,
            workspaceId,
            agent: extAgent,
            title: title || agentLabel(extAgent),
            updatedAtMs: Date.now(),
          })
          return
        }
        if (status && !status.session && workspaceId != null && extAgent) {
          upsertDraft({
            sessionId: sid,
            workspaceId,
            agent: extAgent,
            title: title || agentLabel(extAgent),
            updatedAtMs: Date.now(),
          })
        } else if (status?.session && workspaceId != null && isDirectoryExpanded(String(workspaceId))) {
          const wid = String(workspaceId)
          const st = projectStates[wid]
          const alreadyListed = st?.sessions.some((item) => item.native_id === status.session?.native_id)
          if (!st?.loading && !alreadyListed) {
            void loadProject(wid, false)
          }
        }
      })
    }
  },
  { immediate: true },
)

// When a draft session starts running or finishes its first turn, promote it to its native entry.
watch(
  () => chat.selectedSessionExecution?.status,
  (nextStatus, prevStatus) => {
    const sid = currentId.value
    if (!sid || !isAcpSessionId(sid) || !prevStatus || nextStatus === prevStatus) return
    const wid = current.value?.workspace_id
    if (wid == null || isAgentUnavailable.value) return
    if (draftSessions.value.some((item) => item.sessionId === sid)) {
      void inspectSessionStatus(sid, true).then((status) => {
        if (status?.session && isDirectoryExpanded(String(wid))) {
          void loadProject(String(wid), false)
        }
      })
    }
  },
)

const unsubscribeAuth = subscribeServerAuthTokenVersion(() => {
  if (!lifetime.signal.aborted) {
    invalidateCachedAcpProject()
    void refreshAll()
  }
})

onMounted(() => {
  void loadAgents().then(() => loadProjects())
})

onBeforeUnmount(() => {
  unsubscribeAuth()
  abortAllProjectRequests()
  lifetime.abort()
})
</script>

<template>
  <aside
    data-selection-context
    class="oc-chat-sidebar flex h-full min-h-0 min-w-0 flex-col overflow-hidden bg-sidebar text-sidebar-foreground"
    :aria-label="t('nav.acp')"
  >
    <AddDirectoryDialog v-model:open="isAddDirectoryOpen" v-model:path="newDirectoryPath" @add="handleAddDirectory" />

    <RenameSessionDialog
      :open="renameOpen"
      :draft="renameDraft"
      :busy="renameBusy"
      @update:open="
        (v) => {
          if (!v) cancelRename()
        }
      "
      @update:draft="(v) => (renameDraft = v)"
      @save="saveRename"
    />

    <OptionMenu
      :open="menuOpen"
      :groups="menuGroups"
      :title="String(t('chat.sidebar.sessionActions.menuTitle'))"
      :mobile-title="String(t('chat.sidebar.sessionActions.menuTitle'))"
      :searchable="false"
      :is-compact-touch="ui.isCompactTouch"
      :desktop-anchor-el="menuAnchorEl"
      desktop-placement="bottom-start"
      desktop-class="w-56"
      @update:open="
        (v) => {
          if (!v) closeRowMenu()
        }
      "
      @select="runRowAction"
    />

    <AcpSidebarHeader
      :agents="agents"
      :agent="agent"
      :query="query"
      :loading="projectsLoading"
      :is-touch-pointer="ui.isTouchPointer"
      :show-search="!isAgentUnavailable"
      @update:agent="(v) => (agent = v)"
      @update:query="(v) => (query = v)"
      @add-directory="isAddDirectoryOpen = true"
      @refresh="refreshAll"
      @open-settings="openAcpSettings"
    />

    <div class="min-h-0 flex-1 overflow-x-hidden overflow-y-auto pb-2">
      <div v-if="isAgentUnavailable && activeAgentEntry" class="px-3 py-1">
        <AcpInstallGuideCard
          compact
          :agent-id="activeAgentEntry.id"
          :agent-name="activeAgentEntry.name"
          :command="activeAgentEntry.command"
          :show-retry="true"
          :retrying="recheckingAgent"
          @retry="recheckActiveAgent"
        />
      </div>

      <div
        v-else
        v-keyboard-collection="{ tree: true }"
        :aria-label="String(t('header.openSessions'))"
        class="min-h-0 overflow-x-hidden overflow-visible"
      >
        <div class="space-y-0.5 pb-0.5 pl-2 pr-1">
          <div v-if="projectsError" class="px-1.5 py-2 space-y-2">
            <p
              role="alert"
              class="break-words rounded-md border border-destructive/30 bg-destructive/10 px-2.5 py-2 text-xs text-destructive"
            >
              {{ projectsError }}
            </p>
            <SidebarTextButton class="w-full justify-center" @click="() => loadProjects()">
              {{ t('common.retry') }}
            </SidebarTextButton>
          </div>

          <SidebarSectionSkeleton v-else-if="projectsLoading && !projects.length" :rows="4" />

          <div v-else-if="!projects.length" class="px-2 py-6 text-center text-muted-foreground">
            <div class="typography-ui-label font-semibold">{{ t('chat.sidebar.directoriesList.empty.title') }}</div>
            <div class="typography-meta mt-1">{{ t('chat.sidebar.directoriesList.empty.description') }}</div>
          </div>

          <div v-else-if="!visibleProjects.length" class="px-2 py-6 text-center text-muted-foreground">
            <div class="typography-ui-label font-semibold">
              {{ t('chat.sidebar.directoriesList.noMatchingDirectories.title') }}
            </div>
            <div class="typography-meta mt-1">
              {{ t('chat.sidebar.directoriesList.noMatchingDirectories.description') }}
            </div>
          </div>

          <div v-else class="space-y-0.5">
            <div v-for="directory in visibleProjects" :key="directory.id" class="relative">
              <div
                class="sticky top-0 z-20 w-full border-b bg-sidebar pt-1.5 pb-1 select-none group"
                :class="
                  isDirectoryExpanded(directory.id)
                    ? 'border-sidebar-border/60'
                    : 'border-sidebar-border/30'
                "
              >
                <AcpProjectRow
                  :directory="directory"
                  :ui-is-compact-layout="ui.isCompactLayout"
                  :collapsed="!isDirectoryExpanded(directory.id)"
                  :focused="isDirectoryFocused(directory)"
                  :activity-state="acpDirectoryActivityState(rowsForProject(directory.id, ''))"
                  :loading="Boolean(projectStates[directory.id]?.loading)"
                  :creating-session="creatingProjectId === directory.id"
                  :can-create="Boolean(agent) && !isAgentUnavailable"
                  @toggle-collapse="toggleDirectoryCollapse(directory)"
                  @refresh="loadProject(directory.id, false)"
                  @new-session="newSessionInProject(directory)"
                />
              </div>

              <div v-if="isDirectoryExpanded(directory.id)" class="py-0.5 pl-1">
                <div class="space-y-0.5">
                  <AcpInstallGuideCard
                    v-if="projectStates[directory.id]?.error && isMissingAcpCommandError(projectStates[directory.id]?.error)"
                    compact
                    class="mx-1 my-1"
                    :agent-id="agent"
                    :agent-name="activeAgentEntry?.name"
                    :command="activeAgentEntry?.command"
                    :raw-error="projectStates[directory.id]?.error"
                    :show-retry="true"
                    :retrying="Boolean(projectStates[directory.id]?.loading)"
                    @retry="recheckActiveAgent"
                  />

                  <p
                    v-else-if="projectStates[directory.id]?.error"
                    role="alert"
                    class="mx-1.5 my-1 break-words rounded-md border border-destructive/30 bg-destructive/10 px-2.5 py-1.5 text-xs text-destructive"
                  >
                    {{ projectStates[directory.id]?.error }}
                  </p>

                  <SidebarSectionSkeleton
                    v-else-if="projectStates[directory.id]?.loading && !projectStates[directory.id]?.loaded"
                    :rows="3"
                  />

                  <template v-else>
                    <AcpConversationRow
                      v-for="row in rowsForProject(directory.id)"
                      :key="row.key"
                      :row="row"
                      :selected="isRowSelected(row)"
                      :ui-is-compact-layout="ui.isCompactLayout"
                      :opening="openingRowKey === row.key"
                      :busy="busyRowKey === row.key"
                      :can-rename="canRenameRow(row)"
                      :can-delete="canDeleteRow(row)"
                      :renaming="renameInlineKey === row.key"
                      :rename-draft="renameDraft"
                      :rename-busy="renameBusy"
                      @open="openRow(row)"
                      @open-menu="(event) => openRowMenu(row, event)"
                      @toggle-archive="toggleRowArchive(row)"
                      @delete="deleteRow(row)"
                      @request-rename="startRenameRow(row)"
                      @update:rename-draft="(v) => (renameDraft = v)"
                      @rename-save="saveRename"
                      @rename-cancel="cancelRename"
                    />

                    <p
                      v-if="!rowsForProject(directory.id).length && projectStates[directory.id]?.loaded"
                      class="px-3 py-1.5 text-xs text-muted-foreground/75"
                    >
                      {{ query.trim() ? t('chat.nativeAgent.noHistoryMatch') : t('chat.nativeAgent.noHistory') }}
                    </p>

                    <div
                      v-if="projectStates[directory.id]?.cursor || projectStates[directory.id]?.capabilities?.archive"
                      class="flex items-center justify-between gap-2 px-2 pt-0.5"
                    >
                      <SidebarTextButton
                        v-if="projectStates[directory.id]?.cursor"
                        class="text-[11px] text-muted-foreground hover:text-foreground"
                        :disabled="Boolean(projectStates[directory.id]?.loading)"
                        @click="loadProject(directory.id, true)"
                      >
                        {{ projectStates[directory.id]?.loading ? t('common.loading') : t('common.loadMore') }}
                      </SidebarTextButton>

                      <SidebarTextButton
                        v-if="projectStates[directory.id]?.capabilities?.archive"
                        class="ml-auto text-[11px] text-muted-foreground hover:text-foreground"
                        :disabled="Boolean(projectStates[directory.id]?.loading)"
                        @click="toggleProjectArchived(directory.id)"
                      >
                        {{
                          t(
                            projectStates[directory.id]?.showArchived
                              ? 'chat.nativeAgent.hideArchived'
                              : 'chat.nativeAgent.showArchived',
                          )
                        }}
                      </SidebarTextButton>
                    </div>
                  </template>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  </aside>
</template>
