<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import { apiErrorBodyRecord, ApiError } from '@/lib/api'
import { gitJson, gitWatchUrl } from '@/lib/gitApi'
import { copyTextToClipboard } from '@/lib/clipboard'
import { useI18n } from 'vue-i18n'
import { useGitDiffSelection } from '@/composables/git/useGitDiffSelection'
import { useGitStatusPaged } from '@/composables/git/useGitStatusPaged'
import { useGitWatchSse } from '@/composables/git/useGitWatchSse'
import { createRevalidator } from '@/lib/revalidation'
import { isDocumentVisible, limitBackgroundReads } from '@/lib/backgroundReads'
import { useGitPageAuth } from './git/useGitPageAuth'
import { useGitCommitState } from './git/useGitCommitState'
import { useGitCommitOps } from './git/useGitCommitOps'
import { useGitRemoteOps } from './git/useGitRemoteOps'
import { useGitRemoteBranchPicker } from './git/useGitRemoteBranchPicker'
import { useGitRemoteTargetOps } from './git/useGitRemoteTargetOps'
import { useGitRemoteTargetState } from './git/useGitRemoteTargetState'
import { useGitRepoSelection } from './git/useGitRepoSelection'
import { useGitBranches } from './git/useGitBranches'
import { useGitCheckoutOps } from './git/useGitCheckoutOps'
import { useGitPathOps } from './git/useGitPathOps'
import { useGitHistoryOps } from './git/useGitHistoryOps'
import { useGitHistoryLog } from './git/useGitHistoryLog'
import { useGitMergeRebaseOps } from './git/useGitMergeRebaseOps'
import { useGitCompareOps } from './git/useGitCompareOps'
import { useGitSubmoduleOps } from './git/useGitSubmoduleOps'
import { useGitLfsOps } from './git/useGitLfsOps'
import { useGitAutoFetch } from './git/useGitAutoFetch'
import { useGitRemotesOps } from './git/useGitRemotesOps'
import { useGitSequencerOps } from './git/useGitSequencerOps'
import { useGitStashOps } from './git/useGitStashOps'
import { useGitTags } from './git/useGitTags'
import { useGitWorkingTreeOps } from './git/useGitWorkingTreeOps'
import { useGitWorktrees } from './git/useGitWorktrees'
import { useGitPatchOps } from './git/useGitPatchOps'
import GitPageView from './git/GitPageView.vue'
import { composeGitPageViewContext } from './git/composeGitPageViewContext'
import { createEmptyStatusSummary, joinFs } from './git/gitPageUtils'
import type {
  GitLogCommit,
  GitRemoteInfoResponse,
  GitSigningInfoResponse,
  GitStateResponse,
  GitStatusResponse,
  GitWatchStatusPayload,
} from '@/types/git'
import { useDirectoryStore } from '@/stores/directory'
import { useGitReposStore } from '@/stores/gitRepos'
import { useSettingsStore } from '@/stores/settings'
import { useUiStore } from '@/stores/ui'
import { useToastsStore } from '@/stores/toasts'
import { gitRepoScopedStorageKey, localStorageKeys } from '@/lib/persistence/storageKeys'
import { isEmbeddedWorkspacePaneContext } from '@/app/windowScope'
import { useWorkspaceNavigation } from '@/app/navigation/useWorkspaceNavigation'

const props = withDefaults(
  defineProps<{
    embedded?: boolean
  }>(),
  {
    embedded: false,
  },
)

// Stores & State
const settings = useSettingsStore()
const toasts = useToastsStore()
const { t } = useI18n()
const directoryStore = useDirectoryStore()
const gitRepos = useGitReposStore()
const ui = useUiStore()
const route = useRoute()
const router = useRouter()
const workspaceNavigation = useWorkspaceNavigation()
const isEmbeddedWorkspacePane = computed(() => props.embedded || isEmbeddedWorkspacePaneContext(route.query))

const projectRoot = computed(() => directoryStore.currentDirectory)

// Unified SCM actions menu.
const actionsOpen = ref(false)
const autoFetchDialogOpen = ref(false)

const autoFetchEnabled = computed<boolean>({
  get() {
    return Boolean(settings.data?.gitAutoFetchEnabled)
  },
  set(value) {
    void settings.save({ gitAutoFetchEnabled: Boolean(value) })
  },
})

const autoFetchIntervalMinutes = computed<number>({
  get() {
    const raw = settings.data?.gitAutoFetchIntervalMinutes
    const n = typeof raw === 'number' && Number.isFinite(raw) ? raw : 10
    return Math.max(1, n)
  },
  set(value) {
    const n = Math.max(1, Number(value) || 1)
    void settings.save({ gitAutoFetchIntervalMinutes: n })
  },
})

const autoSyncEnabled = computed<boolean>({
  get() {
    return Boolean(settings.data?.gitAutoSyncEnabled)
  },
  set(value) {
    void settings.save({ gitAutoSyncEnabled: Boolean(value) })
  },
})

const autoSyncIntervalMinutes = computed<number>({
  get() {
    const raw = settings.data?.gitAutoSyncIntervalMinutes
    const n = typeof raw === 'number' && Number.isFinite(raw) ? raw : 30
    return Math.max(1, n)
  },
  set(value) {
    const n = Math.max(1, Number(value) || 1)
    void settings.save({ gitAutoSyncIntervalMinutes: n })
  },
})

type BranchProtectionPrompt = 'alwaysCommit' | 'alwaysCommitToNewBranch' | 'alwaysPrompt'
type PostCommitCommand = 'none' | 'push' | 'sync'

const gitAllowForcePush = computed<boolean>(() => Boolean(settings.data?.gitAllowForcePush))
const gitAllowNoVerifyCommit = computed<boolean>(() => Boolean(settings.data?.gitAllowNoVerifyCommit))
const gitBranchProtection = computed<string[]>(() => {
  const raw = settings.data?.gitBranchProtection
  if (!Array.isArray(raw)) return []
  return raw.map((item) => (typeof item === 'string' ? item.trim() : '')).filter((item) => !!item)
})
const gitBranchProtectionPrompt = computed<BranchProtectionPrompt>(() => {
  const raw = settings.data?.gitBranchProtectionPrompt
  const mode = typeof raw === 'string' ? raw.trim() : ''
  if (mode === 'alwaysCommit' || mode === 'alwaysCommitToNewBranch' || mode === 'alwaysPrompt') {
    return mode
  }
  return 'alwaysPrompt'
})
const gitPostCommitCommand = computed<PostCommitCommand>({
  get() {
    const raw = settings.data?.gitPostCommitCommand
    const command = typeof raw === 'string' ? raw.trim() : ''
    if (command === 'push' || command === 'sync' || command === 'none') return command
    return 'none'
  },
  set(value) {
    const command: PostCommitCommand = value === 'push' || value === 'sync' ? value : 'none'
    void settings.save({ gitPostCommitCommand: command })
  },
})

const selectedRepoRelative = computed(() => {
  const base = (projectRoot.value || '').trim()
  if (!base) return null
  return gitRepos.getSelectedRelative(base)
})

const selectedRepoLabel = computed(() => {
  if (!projectRoot.value) return '(no project)'
  return selectedRepoRelative.value || '(none)'
})

async function switchProjectRoot(path: string) {
  const target = (path || '').trim()
  if (!target) return
  const existing = settings.data?.projects || []
  if (!existing.some((project) => project.path.trim() === target)) {
    await settings.addProject(target)
  }
  directoryStore.setDirectory(target)
}

const repoSelection = useGitRepoSelection({
  projectRoot,
  selectedRepoRelative,
  gitRepos,
  toasts,
  gitJson,
  load,
  switchProjectRoot,
})

const repoRoot = computed(() => {
  const base = projectRoot.value || ''
  const rel = selectedRepoRelative.value || '.'
  const out = joinFs(base, rel)
  return out || null
})

const root = repoRoot

const status = ref<GitStatusResponse | null>(null)
const loading = ref(false)
const error = ref<string | null>(null)

const gitCheckLoading = ref(false)
const isGitRepository = ref<boolean | null>(null)
const gitReady = computed(() => isGitRepository.value === true)
const unsafeRepoPath = ref('')
const unsafeRepoHint = ref('')
const unsafeRepoBusy = ref(false)
const unsafeRepoDetected = computed(() => !!unsafeRepoPath.value)

const remoteInfo = ref<GitRemoteInfoResponse | null>(null)
const signingInfo = ref<GitSigningInfoResponse | null>(null)
const gitState = ref<GitStateResponse | null>(null)
const hasRemotes = computed(() => (remoteInfo.value?.remotes || []).length > 0)

// Stash panel logic extracted to ./git/useGitStashOps

// Push/Pull target selection state (VS Code-like "... to...")
const remoteTargetState = useGitRemoteTargetState()
const { pushToOpen, pullFromOpen, fetchFromOpen, targetRemote, targetBranch, targetRef, targetSetUpstream } =
  remoteTargetState

// Tags logic extracted to ./git/useGitTags

// Detached checkout / branch-from-ref, rename/delete, and history/template helpers are extracted below.

const remoteBranchPicker = useGitRemoteBranchPicker({
  gitJson,
  repoRoot,
  pushToOpen,
  pullFromOpen,
  fetchFromOpen,
  targetRemote,
  targetBranch,
})

// VS Code Style Sections
const isMergeExpanded = ref(true)
const isStagedExpanded = ref(true)
const isChangesExpanded = ref(true)
const isUntrackedExpanded = ref(true)
const isHistoryExpanded = ref(true)

// File list paging (large repos can have thousands of changed files).
// This is backed by server pagination to keep both network + UI responsive.
const FILE_LIST_PAGE_SIZE = 200
let repoReadController = new AbortController()
let watchReadController: AbortController | null = null
let watchGeneration = 0
let gitStatusGeneration = 0

async function readRepoJson<T>(
  path: string,
  directory: string,
  query?: Record<string, string | number | boolean | null | undefined>,
  signal?: AbortSignal,
): Promise<T> {
  const owner = repoReadController
  const requestSignal = AbortSignal.any([owner.signal, ...(signal ? [signal] : []), AbortSignal.timeout(30_000)])
  requestSignal.throwIfAborted()
  if (root.value !== directory) throw new DOMException('Repository changed before read', 'AbortError')
  const response = await gitJson<T>(path, directory, query, { signal: requestSignal })
  requestSignal.throwIfAborted()
  if (owner !== repoReadController || root.value !== directory)
    throw new DOMException('Repository changed during read', 'AbortError')
  return response
}
const conflictPaths = ref<string[]>([])
const conflictReaders = new WeakMap<AbortController, number>()
let conflictGeneration = 0

const {
  mergeList,
  stagedList,
  changesList,
  untrackedList,
  mergeListLoading,
  stagedListLoading,
  changesListLoading,
  untrackedListLoading,
  mergeCount,
  stagedCount,
  changesCount,
  untrackedCount,
  hasMoreMerge,
  hasMoreStaged,
  hasMoreUnstaged,
  hasMoreUntracked,
  loadMore,
  reloadScopeFirstPage,
  cancelRequests: cancelStatusRequests,
  clearScope: clearStatusScope,
  resetAll: resetStatusPages,
} = useGitStatusPaged({
  gitReady,
  status,
  pageSize: FILE_LIST_PAGE_SIZE,
  loadStatusPage: async ({ directory, scope, offset, limit, signal }) => {
    if (root.value === directory) dirtyWatchScopes.delete(scope)
    return await readRepoJson<GitStatusResponse>('status', directory, { scope, offset, limit, includeDiffStats: true }, signal)
  },
})

async function loadMoreMerge(directory: string) {
  await loadMore(directory, 'merge')
}

async function loadMoreStaged(directory: string) {
  await loadMore(directory, 'staged')
}

async function loadMoreChanges(directory: string) {
  await loadMore(directory, 'unstaged')
}

async function loadMoreUntracked(directory: string) {
  await loadMore(directory, 'untracked')
}

// Selection + Diff
const diffPaneRef = ref<{ refreshDiff: () => Promise<void> } | null>(null)
function refreshDiff() {
  return diffPaneRef.value?.refreshDiff?.() ?? Promise.resolve()
}
const { selectedFile, diffSource, selectedIsConflict, selectFile } = useGitDiffSelection({
  conflictPaths,
  refresh: refreshDiff,
})

const restoringOpenItem = ref(false)

watch(
  () => repoRoot.value,
  (nextRepo, prevRepo) => {
    const nextKey = (nextRepo || '').trim()
    const prevKey = (prevRepo || '').trim()
    if (nextKey === prevKey || !nextKey) return

    const saved = gitRepos.getOpenItem(nextKey)
    restoringOpenItem.value = true
    selectedFile.value = saved?.path || null
    diffSource.value = saved?.source || 'working'
    restoringOpenItem.value = false
  },
  { immediate: true },
)

watch(
  () => [repoRoot.value, selectedFile.value, diffSource.value] as const,
  ([repo, path, source]) => {
    if (restoringOpenItem.value) return
    gitRepos.setOpenItem(repo, path, source)
  },
)

function repoKey(suffix: string): string {
  return gitRepoScopedStorageKey(suffix, repoRoot.value)
}

const commitState = useGitCommitState({ repoRoot, repoKey })

const {
  commitMessage,
  committing,
  commitNoVerify,
  commitSignoff,
  commitAmend,
  commitNoGpgSign,
  commitErrorOpen,
  commitErrorTitle,
  commitErrorOutput,
  postCommitOpen,
  postCommitTitle,
  postCommitExplain,
  postCommitRememberChoice,
  pushCommitHistory,
} = commitState

function onCommitSuccess(msg: string) {
  pushCommitHistory(msg)
  commitMessage.value = ''
}

// Credentials / passphrase dialogs.
const auth = useGitPageAuth({
  router,
  toasts,
  repoRoot,
  repoKey,
  gitJson,
  status,
  remoteInfo,
  signingInfo,
  committing,
  onCommitSuccess,
  commitNoVerify,
  commitSignoff,
  commitAmend,
  commitNoGpgSign,
  load,
})

// Auth/credentials/GPG helpers extracted to './git/useGitPageAuth'.

const remoteTargetOps = useGitRemoteTargetOps({
  repoRoot,
  status,
  preferredRemote: auth.preferredRemote,
  pushToOpen,
  pullFromOpen,
  fetchFromOpen,
  targetRemote,
  targetBranch,
  targetRef,
  targetSetUpstream,
  clearRemoteBranchOptions: remoteBranchPicker.clearRemoteBranchOptions,
  prefetchRemoteBranches: remoteBranchPicker.prefetchRemoteBranches,
  toasts,
  gitJson,
  withRepoBusy: auth.withRepoBusy,
  handleGitBusy: auth.handleGitBusy,
  handleGitSso: auth.handleGitSso,
  load,
  isGitAuthError: auth.isGitAuthError,
  openCredentialsDialog: auth.openCredentialsDialog,
  openTerminalHelp: auth.openTerminalHelp,
})

const remoteOps = useGitRemoteOps({
  repoRoot,
  allowForcePush: gitAllowForcePush,
  preferredRemote: auth.preferredRemote,
  isGithubRemote: auth.isGithubRemote,
  githubToken: auth.githubToken,
  toasts,
  gitJson,
  withRepoBusy: auth.withRepoBusy,
  handleGitBusy: auth.handleGitBusy,
  handleGitSso: auth.handleGitSso,
  load,
  isGitAuthError: auth.isGitAuthError,
  openCredentialsDialog: auth.openCredentialsDialog,
  openTerminalHelp: auth.openTerminalHelp,
  terminalCommandForRemoteAction: auth.terminalCommandForRemoteAction,
})

const commitOps = useGitCommitOps({
  repoRoot,
  repoKey,
  status,
  toasts,
  gitJson,
  withRepoBusy: auth.withRepoBusy,
  handleGitBusy: auth.handleGitBusy,
  load,
  commitMessage,
  committing,
  commitNoVerify,
  commitSignoff,
  commitAmend,
  commitNoGpgSign,
  allowNoVerifyCommit: gitAllowNoVerifyCommit,
  branchProtectionRules: gitBranchProtection,
  branchProtectionPrompt: gitBranchProtectionPrompt,
  postCommitCommand: gitPostCommitCommand,
  postCommitRememberChoice,
  runPostCommitPush: remoteOps.push,
  runPostCommitSync: remoteOps.sync,
  applyGitmoji,
  commitErrorOpen,
  commitErrorTitle,
  commitErrorOutput,
  postCommitOpen,
  postCommitTitle,
  postCommitExplain,
  isGpgNoKeyError: auth.isGpgNoKeyError,
  isGpgPassphraseError: auth.isGpgPassphraseError,
  gpgDialogOpen: auth.gpgDialogOpen,
  gpgExplain: auth.gpgExplain,
  pendingCommitMessage: auth.pendingCommitMessage,
  gpgMissingDialogOpen: auth.gpgMissingDialogOpen,
  gpgMissingExplain: auth.gpgMissingExplain,
  openTerminalHelp: auth.openTerminalHelp,
  terminalCommandForCommit: auth.terminalCommandForCommit,
  onCommitSuccess,
})

useGitAutoFetch({
  repoRoot,
  gitReady,
  repoBusy: auth.repoBusy,
  hasRemotes,
  autoFetchEnabled,
  autoFetchIntervalMinutes,
  autoSyncEnabled,
  autoSyncIntervalMinutes,
  fetchRemote: remoteOps.fetchRemote,
  sync: remoteOps.sync,
})

const sequencerOps = useGitSequencerOps({
  repoRoot,
  toasts,
  gitJson,
  withRepoBusy: auth.withRepoBusy,
  handleGitBusy: auth.handleGitBusy,
  load,
  openTerminalHelp: auth.openTerminalHelp,
})

const tags = useGitTags({
  repoRoot,
  preferredRemote: auth.preferredRemote,
  gitJson,
  toasts,
  withRepoBusy: auth.withRepoBusy,
  handleGitBusy: auth.handleGitBusy,
})

const workingTreeOps = useGitWorkingTreeOps({
  root,
  selectedFile,
  toasts,
  gitJson,
  withRepoBusy: auth.withRepoBusy,
  handleGitBusy: auth.handleGitBusy,
  load,
  refreshAfterStageOp: refreshAfterWorkingTreeChange,
  refreshDiff,
})

const branchesOps = useGitBranches({
  root,
  toasts,
  gitJson,
  withRepoBusy: auth.withRepoBusy,
  handleGitBusy: auth.handleGitBusy,
  load,
})

const { branches, loadBranches } = branchesOps

const historyBranchOptions = computed(() => {
  const list = Object.values(branches.value?.branches ?? {})
    .map((b) => (b?.name || '').trim())
    .filter((name) => !!name && !name.startsWith('remotes/') && !name.endsWith('/HEAD'))
  list.sort((a, b) => a.localeCompare(b))
  return list
})

const historyTagOptions = computed(() => {
  const tagList = Array.isArray(tags.tagsList.value) ? tags.tagsList.value : []
  const list = tagList.map((t) => (t?.name || '').trim()).filter(Boolean)
  list.sort((a, b) => a.localeCompare(b))
  return list
})

const checkoutOps = useGitCheckoutOps({
  repoRoot,
  toasts,
  gitJson,
  withRepoBusy: auth.withRepoBusy,
  handleGitBusy: auth.handleGitBusy,
  load,
  loadBranches,
})

const pathOps = useGitPathOps({
  root,
  selectedFile,
  toasts,
  gitJson,
  withRepoBusy: auth.withRepoBusy,
  handleGitBusy: auth.handleGitBusy,
  load,
  refreshDiff,
})

const stashOps = useGitStashOps({
  repoRoot,
  toasts,
  gitJson,
  readJson: <T,>(endpoint: string, directory: string, query?: Record<string, string | number | boolean | null | undefined>, init?: RequestInit) => {
    const signal = AbortSignal.any([repoReadController.signal, ...(init?.signal ? [init.signal] : [])])
    return limitBackgroundReads(() => readRepoJson<T>(endpoint, directory, query, signal), signal)
  },
  withRepoBusy: auth.withRepoBusy,
  handleGitBusy: auth.handleGitBusy,
  load,
  loadBranches,
  openTerminalHelp: auth.openTerminalHelp,
})

const { stashList, loadStash } = stashOps

const historyOps = useGitHistoryOps({
  repoRoot,
  toasts,
  gitJson,
  withRepoBusy: auth.withRepoBusy,
  handleGitBusy: auth.handleGitBusy,
  load,
  openTerminalHelp: auth.openTerminalHelp,
  commitMessage,
  commitErrorOpen,
  commitErrorTitle,
  commitErrorOutput,
})

const historyLog = useGitHistoryLog({
  repoRoot,
  gitJson,
})

const remotesOps = useGitRemotesOps({
  repoRoot,
  remoteInfo,
  gitJson,
  toasts,
  withRepoBusy: auth.withRepoBusy,
  handleGitBusy: auth.handleGitBusy,
})

const worktreesOps = useGitWorktrees({
  repoRoot,
  gitJson,
  toasts,
  withRepoBusy: auth.withRepoBusy,
  handleGitBusy: auth.handleGitBusy,
})

const mergeRebaseOps = useGitMergeRebaseOps({
  repoRoot,
  toasts,
  gitJson,
  withRepoBusy: auth.withRepoBusy,
  handleGitBusy: auth.handleGitBusy,
  load,
  openTerminalHelp: auth.openTerminalHelp,
})

const compareOps = useGitCompareOps({
  repoRoot,
  gitJson,
  toasts,
})

const historyCompareHash = ref('')

function selectHistoryCompareCommit(commit: GitLogCommit) {
  const hash = (commit?.hash || '').trim()
  if (!hash) return
  historyCompareHash.value = hash
  toasts.push('info', t('git.toasts.selectedCommitForCompare', { hash: hash.slice(0, 7) }), 1500)
}

function clearHistoryCompareCommit() {
  historyCompareHash.value = ''
}

async function compareHistoryWithParent(commit: GitLogCommit) {
  const head = (commit?.hash || '').trim()
  const parent = (commit?.parents?.[0] || '').trim()
  if (!head || !parent) {
    toasts.push('info', t('git.toasts.selectedCommitHasNoParent'))
    return
  }
  const path = (historyLog.historyFilterPath.value || '').trim()
  await compareOps.compareCommitWithParent(head, parent, path)
}

async function compareHistoryWithSelected(commit: GitLogCommit) {
  const base = (historyCompareHash.value || '').trim()
  const head = (commit?.hash || '').trim()
  if (!base || !head || base === head) return
  const path = (historyLog.historyFilterPath.value || '').trim()
  compareOps.compareBase.value = base
  compareOps.compareHead.value = head
  compareOps.comparePath.value = path
  compareOps.compareOpen.value = true
  await compareOps.runCompare()
}

async function compareWithUpstream() {
  const current = (status.value?.current || '').trim()
  const upstream = (status.value?.tracking || '').trim()
  if (!current || !upstream) {
    toasts.push('info', t('git.toasts.noUpstreamTrackingBranch'))
    return
  }
  compareOps.compareBase.value = upstream
  compareOps.compareHead.value = current
  compareOps.comparePath.value = ''
  compareOps.compareOpen.value = true
  await compareOps.runCompare()
}

const submoduleOps = useGitSubmoduleOps({
  repoRoot,
  gitJson,
  toasts,
  withRepoBusy: auth.withRepoBusy,
  handleGitBusy: auth.handleGitBusy,
})

const lfsOps = useGitLfsOps({
  repoRoot,
  gitJson,
  toasts,
  withRepoBusy: auth.withRepoBusy,
  handleGitBusy: auth.handleGitBusy,
})

const patchOps = useGitPatchOps({
  repoRoot,
  gitJson,
  toasts,
  withRepoBusy: auth.withRepoBusy,
  handleGitBusy: auth.handleGitBusy,
  load,
  refreshAfterPatchOp: refreshAfterWorkingTreeChange,
  refreshDiff,
})

const gitmojiEnabled = computed(() => Boolean(settings.data?.gitmojiEnabled))
const selectedGitmoji = ref(localStorage.getItem(localStorageKeys.git.gitmoji) || '')
watch(selectedGitmoji, (v) => localStorage.setItem(localStorageKeys.git.gitmoji, v))

const gitmojis = [
  { emoji: '✨', label: 'feat' },
  { emoji: '🐛', label: 'fix' },
  { emoji: '📝', label: 'docs' },
  { emoji: '♻️', label: 'refactor' },
  { emoji: '✅', label: 'test' },
  { emoji: '⚡️', label: 'perf' },
  { emoji: '🔧', label: 'chore' },
  { emoji: '🚀', label: 'deploy' },
]

// Methods
function applyGitmoji(message: string): string {
  const base = (message || '').trim()
  if (!gitmojiEnabled.value) return base
  const e = (selectedGitmoji.value || '').trim()
  if (!e) return base

  const known = gitmojis.map((g) => g.emoji)
  for (const k of known) {
    if (base.startsWith(`${k} `)) {
      return `${e} ${base.slice((k + ' ').length).trim()}`
    }
  }
  return `${e} ${base}`
}

function insertGitmoji() {
  if (!gitmojiEnabled.value) return
  const e = (selectedGitmoji.value || '').trim()
  if (!e) return
  commitMessage.value = applyGitmoji(commitMessage.value)
}

let loadSeq = 0

let pendingWatchDiff = false
let pendingWatchConflicts = false
type WatchScope = 'merge' | 'staged' | 'unstaged' | 'untracked'
const dirtyWatchScopes = new Set<WatchScope>()
// Selection changes restart the path stream, but do not discard the
// repository baseline used to validate already displayed file lists.
let repoWatchBaseline: GitWatchStatusPayload | null = null
const visibleWatchScopes = () =>
  [
    ['merge', isMergeExpanded.value, status.value?.mergeCount ?? 0, mergeList],
    ['staged', isStagedExpanded.value, status.value?.stagedCount ?? 0, stagedList],
    ['unstaged', isChangesExpanded.value, status.value?.unstagedCount ?? 0, changesList],
    ['untracked', isUntrackedExpanded.value, status.value?.untrackedCount ?? 0, untrackedList],
  ] as const
const watchRefreshQueue = createRevalidator(
  async () => {
    const directory = root.value
    if (!directory || !gitReady.value) return
    const owner = watchGeneration
    const controller = new AbortController()
    watchReadController = controller
    try {
      await refreshOpenDiffFromWatch(directory, owner, controller.signal)
      for (const [scope, expanded, count] of visibleWatchScopes()) {
        controller.signal.throwIfAborted()
        if (owner !== watchGeneration || root.value !== directory || !isDocumentVisible()) return
        if (!dirtyWatchScopes.has(scope) || !expanded) continue
        dirtyWatchScopes.delete(scope)
        try {
          if (count === 0) clearStatusScope(scope)
          else await reloadScopeFirstPage(directory, scope)
        } catch (error) {
          if (owner === watchGeneration && root.value === directory) dirtyWatchScopes.add(scope)
          throw error
        }
      }
    } finally {
      if (watchReadController === controller) watchReadController = null
    }
  },
  { intervalMs: 1500, retryMs: 5000, enabled: () => isDocumentVisible() && gitReady.value },
)

async function refreshOpenDiffFromWatch(directory: string, owner: number, signal: AbortSignal) {
  let conflicts = pendingWatchConflicts
  let diff = pendingWatchDiff
  const path = selectedFile.value
  pendingWatchConflicts = false
  pendingWatchDiff = false
  try {
    signal.throwIfAborted()
    if (conflicts) {
      if ((status.value?.mergeCount ?? 0) > 0) await loadConflicts(directory, signal)
      else conflictPaths.value = []
      conflicts = false
    }
    signal.throwIfAborted()
    if (diff && owner === watchGeneration && root.value === directory && selectedFile.value === path && isDocumentVisible()) {
      await refreshDiff()
      diff = false
    }
  } catch (error) {
    if (owner === watchGeneration && root.value === directory) {
      pendingWatchConflicts ||= conflicts
      pendingWatchDiff ||= diff && selectedFile.value === path
    }
    throw error
  }
}

async function refreshAfterWorkingTreeChange() {
  const directory = root.value
  if (!directory || !gitReady.value) return
  await loadStatusSummary(directory)
  if (root.value !== directory) return
  await Promise.all([
    loadGitState(directory).catch((error) => {
      if (root.value === directory && !(error instanceof DOMException && error.name === 'AbortError')) gitState.value = null
    }),
    ((status.value?.mergeCount ?? 0) > 0 ? loadConflicts(directory) : Promise.resolve().then(() => { conflictPaths.value = [] })).catch((error) => {
      if (root.value === directory && !(error instanceof DOMException && error.name === 'AbortError')) conflictPaths.value = []
    }),
    ...visibleWatchScopes().map(([scope, expanded, count]) => {
      if (count === 0) clearStatusScope(scope)
      else if (expanded) return reloadScopeFirstPage(directory, scope)
      else dirtyWatchScopes.add(scope)
      return Promise.resolve()
    }),
  ])
  if (root.value === directory) void refreshDiff()
}

const { startWatch: startWatchInner, stopWatch } = useGitWatchSse<GitWatchStatusPayload>({
  buildUrl: (directory) => gitWatchUrl(directory, 1500, selectedFile.value),
  onPayload: (payload, prev) => {
    const baseline = repoWatchBaseline ?? prev
    if (baseline?.updatedAtMs != null && payload.updatedAtMs != null && payload.updatedAtMs < baseline.updatedAtMs) return
    repoWatchBaseline = payload
    if (
      prev &&
      payload.updatedAtMs === prev.updatedAtMs &&
      payload.worktreeSignature === prev.worktreeSignature &&
      payload.selectedPathSignature === prev.selectedPathSignature
    )
      return
    // Update the summary immediately; an older HTTP snapshot cannot replace it.
    gitStatusGeneration++
    const baseStatus = status.value ?? createEmptyStatusSummary()
    const nextStatus = {
      ...baseStatus,
      current: payload.current,
      tracking: payload.tracking ?? null,
      ahead: payload.ahead,
      behind: payload.behind,
      stagedCount: payload.stagedCount,
      unstagedCount: payload.unstagedCount,
      untrackedCount: payload.untrackedCount,
      mergeCount: payload.mergeCount,
      totalFiles: payload.totalFiles ?? baseStatus.totalFiles,
    }
    if (!status.value || nextStatus.current !== baseStatus.current || nextStatus.tracking !== baseStatus.tracking ||
      nextStatus.ahead !== baseStatus.ahead || nextStatus.behind !== baseStatus.behind ||
      nextStatus.stagedCount !== baseStatus.stagedCount || nextStatus.unstagedCount !== baseStatus.unstagedCount ||
      nextStatus.untrackedCount !== baseStatus.untrackedCount || nextStatus.mergeCount !== baseStatus.mergeCount ||
      nextStatus.totalFiles !== baseStatus.totalFiles) status.value = nextStatus

    if (payload.isClean) {
      // If the repo is clean, clear selection + lists to match VS Code behavior.
      selectedFile.value = null
    }

    // Repository clocks update summaries; independent path/scope signatures
    // restrict expensive reads to the visible representations that changed.
    const prevSignature = typeof baseline?.worktreeSignature === 'string' ? baseline.worktreeSignature : ''
    const nextSignature = typeof payload.worktreeSignature === 'string' ? payload.worktreeSignature : ''
    const changedCounts =
      !baseline ||
      baseline.current !== payload.current ||
      baseline.tracking !== payload.tracking ||
      baseline.ahead !== payload.ahead ||
      baseline.behind !== payload.behind ||
      baseline.stagedCount !== payload.stagedCount ||
      baseline.unstagedCount !== payload.unstagedCount ||
      baseline.untrackedCount !== payload.untrackedCount ||
      baseline.mergeCount !== payload.mergeCount ||
      baseline.isClean !== payload.isClean ||
      prevSignature !== nextSignature

    if (changedCounts) {
      pendingWatchDiff ||=
        Boolean(selectedFile.value) &&
        (!baseline || (prev != null && (payload.selectedPathSignature == null || payload.selectedPathSignature !== prev.selectedPathSignature)))
      const conflictsChanged = baseline
        ? payload.mergeCount !== baseline.mergeCount || payload.scopeSignatures?.merge !== baseline.scopeSignatures?.merge
        : conflictPaths.value.length > 0 || (conflictReaders.get(repoReadController) ?? 0) > 0
      if (conflictsChanged) {
        conflictGeneration++
        pendingWatchConflicts = true
      }
      const loadingByScope = { merge: mergeListLoading, staged: stagedListLoading, unstaged: changesListLoading, untracked: untrackedListLoading }
      for (const [scope, , count, list] of visibleWatchScopes()) {
        if (count === 0) {
          clearStatusScope(scope)
          dirtyWatchScopes.delete(scope)
        } else if (baseline
          ? !payload.scopeSignatures || payload.scopeSignatures[scope] !== baseline.scopeSignatures?.[scope]
          : list.value.length > 0 || loadingByScope[scope].value) {
          // The first stream snapshot may be newer than an earlier HTTP
          // representation. Validate only rows already read or in flight;
          // initial reads dispatched after this baseline cover it themselves.
          dirtyWatchScopes.add(scope)
        }
      }
      if (pendingWatchDiff || pendingWatchConflicts || dirtyWatchScopes.size) watchRefreshQueue.invalidate(0)
    }
  },
})

watch([isMergeExpanded, isStagedExpanded, isChangesExpanded, isUntrackedExpanded], () => {
  if (dirtyWatchScopes.size) watchRefreshQueue.invalidate(0)
})

function startWatch(directory: string) {
  startWatchInner(directory, `${directory}:${selectedFile.value ?? ''}`)
}

async function loadStatusSummary(directory: string) {
  // summary=true keeps the payload small even for huge repos.
  const generation = gitStatusGeneration
  const response = await limitBackgroundReads(() => readRepoJson<GitStatusResponse>('status', directory, { summary: true }), repoReadController.signal)
  if (generation === gitStatusGeneration) status.value = response
}

async function loadRemoteInfo(directory: string) {
  remoteInfo.value = await limitBackgroundReads(() => readRepoJson<GitRemoteInfoResponse>('remote-info', directory), repoReadController.signal)
}

async function loadSigningInfo(directory: string) {
  signingInfo.value = await limitBackgroundReads(() => readRepoJson<GitSigningInfoResponse>('signing-info', directory), repoReadController.signal)
}

async function loadGitState(directory: string) {
  gitState.value = await limitBackgroundReads(() => readRepoJson<GitStateResponse>('state', directory), repoReadController.signal)
}

async function loadConflicts(directory: string, signal?: AbortSignal) {
  const owner = repoReadController
  const generation = conflictGeneration
  const readSignal = signal ? AbortSignal.any([owner.signal, signal]) : owner.signal
  conflictReaders.set(owner, (conflictReaders.get(owner) ?? 0) + 1)
  try {
    const resp = await limitBackgroundReads(() => readRepoJson<{ files: string[] }>('conflicts', directory, undefined, readSignal), readSignal)
    if (root.value === directory && generation === conflictGeneration) conflictPaths.value = Array.isArray(resp?.files) ? resp.files : []
  } catch (error) {
    if (owner === repoReadController && root.value === directory && generation !== conflictGeneration) return
    throw error
  } finally {
    const remaining = (conflictReaders.get(owner) ?? 1) - 1
    if (remaining) conflictReaders.set(owner, remaining)
    else conflictReaders.delete(owner)
  }
}

function openFirstConflict() {
  const first = (conflictPaths.value || [])[0]
  if (!first) {
    toasts.push('error', t('git.toasts.noConflictsFound'))
    return
  }
  isMergeExpanded.value = true
  selectFile(first, 'working')
}

function resetRepoState() {
  resetStatusPages()
  status.value = null
  selectedFile.value = null
  mergeList.value = []
  stagedList.value = []
  changesList.value = []
  untrackedList.value = []
  remoteInfo.value = null
  signingInfo.value = null
  gitState.value = null
  stashList.value = []
  conflictPaths.value = []
}

function clearUnsafeRepo() {
  unsafeRepoPath.value = ''
  unsafeRepoHint.value = ''
}

async function trustUnsafeRepo() {
  const target = (unsafeRepoPath.value || repoRoot.value || '').trim()
  if (!target) return
  unsafeRepoBusy.value = true
  try {
    await gitJson<{ success: boolean; path: string; alreadyPresent?: boolean }>('safe-directory', target, undefined, {
      method: 'POST',
    })
    clearUnsafeRepo()
    toasts.push('success', t('git.toasts.repositoryMarkedSafe'))
    await repoSelection.loadRepos()
    await load()
  } catch (err) {
    toasts.push('error', err instanceof Error ? err.message : String(err))
  } finally {
    unsafeRepoBusy.value = false
  }
}

async function load() {
  const seq = ++loadSeq
  const dir = root.value
  error.value = null
  loading.value = true
  try {
    await settings.refresh()
    if (seq !== loadSeq || dir !== root.value) return

    if (!dir) {
      resetRepoState()
      clearUnsafeRepo()
      return
    }

    gitCheckLoading.value = true
    try {
      const chk = await readRepoJson<{ isGitRepository: boolean }>('check', dir)
      if (seq !== loadSeq || dir !== root.value) return
      isGitRepository.value = Boolean(chk?.isGitRepository)
    } finally {
      if (seq === loadSeq) gitCheckLoading.value = false
    }

    if (!isGitRepository.value) {
      resetRepoState()
      return
    }

    // Reset the cursor as well as the rows before loading this repository.
    resetStatusPages()
    await loadStatusSummary(dir)
    if (seq !== loadSeq || dir !== root.value) return
    const applyFallback = (error: unknown, fallback: () => void) => {
      if (seq === loadSeq && dir === root.value && !(error instanceof DOMException && error.name === 'AbortError')) fallback()
    }
    await Promise.all([
      loadRemoteInfo(dir).catch((error) => applyFallback(error, () => { remoteInfo.value = { remotes: [] } })),
      loadSigningInfo(dir).catch((error) => applyFallback(error, () => { signingInfo.value = null })),
      loadGitState(dir).catch((error) => applyFallback(error, () => { gitState.value = null })),
      (status.value?.mergeCount ? loadConflicts(dir) : Promise.resolve()).catch((error) => applyFallback(error, () => { conflictPaths.value = [] })),
      (stashOps.isStashExpanded.value ? loadStash(dir, true) : Promise.resolve()).catch((error) => applyFallback(error, () => { stashList.value = [] })),
      ...visibleWatchScopes().map(([scope, expanded, count]) => {
        if (expanded && count > 0) return reloadScopeFirstPage(dir, scope)
        if (count === 0) clearStatusScope(scope)
        else dirtyWatchScopes.add(scope)
        return Promise.resolve()
      }),
    ])
    if (seq === loadSeq && dir === root.value) clearUnsafeRepo()
  } catch (err) {
    if (seq !== loadSeq || dir !== root.value || (err instanceof DOMException && err.name === 'AbortError')) return
    if (err instanceof ApiError && err.code === 'git_unsafe_repository') {
      const body = apiErrorBodyRecord(err)
      const unsafePath = typeof body?.path === 'string' ? body.path.trim() : ''
      unsafeRepoPath.value = unsafePath || (root.value || '').trim()
      unsafeRepoHint.value = (err.hint || '').trim()
      isGitRepository.value = false
      resetRepoState()
      error.value = null
      return
    }
    if (err instanceof ApiError && err.status === 409) {
      isGitRepository.value = false
      resetRepoState()
      error.value = null
      return
    }
    error.value = err instanceof Error ? err.message : String(err)
    status.value = null
    isGitRepository.value = null
  } finally {
    if (seq === loadSeq) loading.value = false
  }
}

async function copyCommitHash(hash: string) {
  const value = (hash || '').trim()
  if (!value) return
  const ok = await copyTextToClipboard(value)
  if (ok) {
    toasts.push('success', t('git.toasts.copiedCommitHash'))
  } else {
    toasts.push('error', t('common.failedToCopyToClipboard'))
  }
}

async function copyRemoteUrl(url: string) {
  const value = (url || '').trim()
  if (!value) return
  const ok = await copyTextToClipboard(value)
  if (ok) {
    toasts.push('success', t('git.toasts.copiedRemoteUrl'))
  } else {
    toasts.push('error', t('common.failedToCopyToClipboard'))
  }
}

async function copyWorktreePath(path: string) {
  const value = (path || '').trim()
  if (!value) return
  const ok = await copyTextToClipboard(value)
  if (ok) {
    toasts.push('success', t('git.toasts.copiedWorktreePath'))
  } else {
    toasts.push('error', t('common.failedToCopyToClipboard'))
  }
}

function resolveRepoFilePath(path: string): string | null {
  const repo = (repoRoot.value || '').trim().replace(/\/+$/g, '')
  if (!repo) return null
  const rel = (path || '').trim().replace(/^\/+/, '')
  if (!rel) return null
  return `${repo}/${rel}`
}

function openFileInFiles(path: string) {
  const abs = resolveRepoFilePath(path)
  if (!abs) return
  if (isEmbeddedWorkspacePane.value) {
    ui.requestWorkspaceDockFile(abs, 'open')
    return
  }

  const query = {
    filePath: abs,
  }
  const fileName = abs.split('/').filter(Boolean).pop() || String(t('nav.files'))
  void workspaceNavigation.openWorkspaceLocation('files', {
    query,
    title: fileName,
    matchKeys: ['filePath'],
  })
}

function revealFileInFiles(path: string) {
  const abs = resolveRepoFilePath(path)
  if (!abs) return
  if (isEmbeddedWorkspacePane.value) {
    ui.requestWorkspaceDockFile(abs, 'reveal')
    return
  }

  const query = {
    filePath: abs,
  }
  const fileName = abs.split('/').filter(Boolean).pop() || String(t('nav.files'))
  void workspaceNavigation.openWorkspaceLocation('files', {
    query,
    title: fileName,
    matchKeys: ['filePath'],
  })
}

function openWorktree(path: string) {
  const base = (projectRoot.value || '').trim().replace(/\/+$/g, '')
  const p = (path || '').trim()
  if (!base || !p) return
  if (!p.startsWith(base)) {
    toasts.push('error', t('git.errors.worktreeOutsideProjectRoot'))
    return
  }
  const rel = p.slice(base.length).replace(/^\/+/, '') || '.'
  gitRepos.setSelectedRelative(base, rel)
}

watch(
  () => projectRoot.value,
  (next, prev) => {
    const n = (next || '').trim()
    const p = (prev || '').trim()
    if (n === p) return
    status.value = null
    selectedFile.value = null
    branches.value = null
    mergeList.value = []
    stagedList.value = []
    changesList.value = []
    untrackedList.value = []
    repoSelection.repos.value = []
    repoSelection.reposError.value = null
    void repoSelection.loadRepos().then(() => load())
  },
)

watch(
  () => selectedRepoRelative.value,
  (next, prev) => {
    const n = (next || '').trim()
    const p = (prev || '').trim()
    if (n === p) return
    status.value = null
    selectedFile.value = null
    branches.value = null
    mergeList.value = []
    stagedList.value = []
    changesList.value = []
    untrackedList.value = []
    void load()
  },
)

/**
 * Start or stop the diff watch for the current selection. A hidden tab shows
 * nothing, so the watch closes and the visibility listener restarts it.
 */
function syncWatch() {
  const nextDir = (root.value || '').trim()
  const visible = isDocumentVisible()
  if (visible && repoReadController.signal.aborted) repoReadController = new AbortController()
  if (!gitReady.value || !nextDir || !visible) {
    stopWatch()
    watchRefreshQueue.pause()
    watchReadController?.abort()
    if (!visible) repoReadController.abort()
    cancelStatusRequests()
    return
  }
  startWatch(nextDir)
  watchRefreshQueue.resume()
}

watch(
  () => [root.value, gitReady.value, selectedFile.value] as const,
  (next, old) => {
    const [dir, ready, selected] = next
    const [prevDir, prevReady, prevSelected] = old ?? [null, false, null]
    const nextDir = (dir || '').trim()
    const prevDirTrimmed = (prevDir || '').trim()
    if (nextDir === prevDirTrimmed && ready === prevReady && selected === prevSelected) return
    if (nextDir !== prevDirTrimmed) {
      watchGeneration++
      watchReadController?.abort()
      repoReadController.abort()
      repoReadController = new AbortController()
      resetStatusPages()
      repoWatchBaseline = null
      dirtyWatchScopes.clear()
      pendingWatchConflicts = false
      pendingWatchDiff = false
    } else if (selected !== prevSelected) pendingWatchDiff = false
    syncWatch()
  },
  { immediate: true, flush: 'sync' },
)

watch(
  () => repoRoot.value,
  (next, prev) => {
    const n = (next || '').trim()
    const p = (prev || '').trim()
    if (n === p) return
    historyCompareHash.value = ''
    auth.loadGithubTokenForRepo()
  },
  { immediate: true },
)

async function showMoreStaged() {
  const dir = root.value
  if (!dir) return
  await loadMoreStaged(dir)
}

async function showMoreChanges() {
  const dir = root.value
  if (!dir) return
  await loadMoreChanges(dir)
}

async function showMoreMerge() {
  const dir = root.value
  if (!dir) return
  await loadMoreMerge(dir)
}

async function showMoreUntracked() {
  const dir = root.value
  if (!dir) return
  await loadMoreUntracked(dir)
}

const headline = computed(() => {
  if (!status.value) return 'No repository'
  const parts = [status.value.current || '(detached)']
  if (status.value.ahead) parts.push(`↑ ${status.value.ahead}`)
  if (status.value.behind) parts.push(`↓ ${status.value.behind}`)
  return parts.join('  ')
})

function gitDiffFileName(path: string): string {
  const normalized = String(path || '')
    .trim()
    .replace(/\\/g, '/')
  if (!normalized) return ''
  const parts = normalized.split('/').filter(Boolean)
  return parts[parts.length - 1] || normalized
}

const gitDiffSourceTitle = computed(() =>
  diffSource.value === 'staged'
    ? String(t('git.ui.diffViewer.labels.index'))
    : String(t('git.ui.diffViewer.labels.workingTree')),
)

function syncGitWindowTitle() {
  const base = String(t('nav.git'))
  const selectedDiffFile = gitDiffFileName(selectedFile.value || '')
  if (selectedDiffFile) {
    const sourceTitle = String(gitDiffSourceTitle.value || '').trim()
    const detail = sourceTitle ? `${selectedDiffFile} · ${sourceTitle}` : selectedDiffFile
    ui.setWorkspaceWindowTitleFromRoute(route.query, `${base} · ${detail}`)
    return
  }

  const branch = String(status.value?.current || '').trim()
  const repo = String(selectedRepoRelative.value || '').trim()
  const detail = branch || (repo && repo !== '.' ? repo : '')
  const title = detail ? `${base} · ${detail}` : base

  ui.setWorkspaceWindowTitleFromRoute(route.query, title)
}

watch(
  () => [status.value?.current, selectedRepoRelative.value, selectedFile.value, diffSource.value, route.query],
  () => {
    syncGitWindowTitle()
  },
  { immediate: true },
)

// Template is rendered by ./git/GitPageView.vue. Keep this file under 1000 LOC by
// passing a context bag (refs + handlers) to the view.
const viewCtx = composeGitPageViewContext([
  {
    label: 'core',
    values: {
      ui,
      toasts,
      projectRoot,
      selectedRepoRelative,
      selectedRepoLabel,
      repoRoot,
      root,
      gitReady,
      status,
      loading,
      error,
      headline,
      gitState,
      signingInfo,
      remoteInfo,

      // Local UI state/handlers used in the view.
      actionsOpen,
      autoFetchDialogOpen,
      autoFetchEnabled,
      autoFetchIntervalMinutes,
      autoSyncEnabled,
      autoSyncIntervalMinutes,
      gitPostCommitCommand,
      gitmojiEnabled,
      gitmojis,
      selectedGitmoji,
      insertGitmoji,

      isMergeExpanded,
      isStagedExpanded,
      isChangesExpanded,
      isUntrackedExpanded,
      isHistoryExpanded,

      mergeList,
      stagedList,
      changesList,
      untrackedList,
      mergeListLoading,
      stagedListLoading,
      changesListLoading,
      untrackedListLoading,
      mergeCount,
      stagedCount,
      changesCount,
      untrackedCount,
      hasMoreMerge,
      hasMoreStaged,
      hasMoreUnstaged,
      hasMoreUntracked,
      showMoreMerge,
      showMoreStaged,
      showMoreChanges,
      showMoreUntracked,
      conflictPaths,

      diffPaneRef,
      selectedFile,
      diffSource,
      selectedIsConflict,
      selectFile,
      openFirstConflict,
      load,
      refreshRepository: load,
      copyCommitHash,
      copyRemoteUrl,
      copyWorktreePath,
      openWorktree,
      openFileInFiles,
      revealFileInFiles,
      historyBranchOptions,
      historyTagOptions,
      historyCompareHash,
      selectHistoryCompareCommit,
      clearHistoryCompareCommit,
      compareHistoryWithParent,
      compareHistoryWithSelected,
      compareWithUpstream,
      unsafeRepoDetected,
      unsafeRepoPath,
      unsafeRepoHint,
      trustUnsafeRepo,
      unsafeRepoBusy,
    },
  },
  { label: 'repoSelection', values: repoSelection },
  { label: 'remoteTargetState', values: remoteTargetState },
  { label: 'remoteBranchPicker', values: remoteBranchPicker },
  { label: 'remoteTargetOps', values: remoteTargetOps },
  { label: 'commitState', values: commitState },
  { label: 'auth', values: auth },
  { label: 'commitOps', values: commitOps },
  { label: 'historyOps', values: historyOps },
  { label: 'historyLog', values: historyLog },
  { label: 'mergeRebaseOps', values: mergeRebaseOps },
  { label: 'compareOps', values: compareOps },
  { label: 'submoduleOps', values: submoduleOps },
  { label: 'lfsOps', values: lfsOps },
  { label: 'remotesOps', values: remotesOps },
  { label: 'worktreesOps', values: worktreesOps },
  { label: 'patchOps', values: patchOps },
  { label: 'remoteOps', values: remoteOps },
  { label: 'sequencerOps', values: sequencerOps },
  { label: 'tags', values: tags },
  { label: 'workingTreeOps', values: workingTreeOps },
  { label: 'checkoutOps', values: checkoutOps },
  { label: 'branchesOps', values: branchesOps },
  { label: 'pathOps', values: pathOps },
  { label: 'stashOps', values: stashOps },
] as const)

defineExpose({
  refresh: load,
})

onMounted(() => {
  void repoSelection.loadRepos().then(() => load())
  document.addEventListener('visibilitychange', syncWatch)
})

onBeforeUnmount(() => {
  document.removeEventListener('visibilitychange', syncWatch)
  stopWatch()
  watchReadController?.abort()
  repoReadController.abort()
  cancelStatusRequests()
  watchRefreshQueue.dispose()
})
</script>

<template>
  <GitPageView :ctx="viewCtx" :embedded="isEmbeddedWorkspacePane" />
</template>
