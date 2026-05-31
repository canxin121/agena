import { computed, ref, watch } from 'vue'
import type { RouteLocationNormalizedLoaded, Router } from 'vue-router'

import {
  createWorkspace,
  deleteWorkspace,
  getGitStatus,
  getVcsDiffRaw,
  initGitProject,
  listWorkspaceFileTree,
  listWorkspaces,
  resolveWorkspace,
  updateWorkspace,
  type GitStatusResource,
  type WorkspaceFileNode,
  type WorkspaceFileTreeResource,
  type WorkspaceResource,
} from '../lib/agenaApi'
import { workspaceShortcuts } from '../lib/runtimeWorkspaceShortcuts'
import { buildRuntimeSectionPath } from './runtimePageStateModel'

export type WorkspaceTreeRow = {
  node: WorkspaceFileNode
  depth: number
}

export type WorkspaceConfigCard = {
  id: string
  title: string
  description: string
  relativePath: string
  active: boolean
  entryCount: number
}

export type WorkspacePageStateDeps = {
  createWorkspace: typeof createWorkspace
  deleteWorkspace: typeof deleteWorkspace
  getGitStatus: typeof getGitStatus
  getVcsDiffRaw: typeof getVcsDiffRaw
  initGitProject: typeof initGitProject
  listWorkspaceFileTree: typeof listWorkspaceFileTree
  listWorkspaces: typeof listWorkspaces
  resolveWorkspace: typeof resolveWorkspace
  updateWorkspace: typeof updateWorkspace
}

const defaultDeps: WorkspacePageStateDeps = {
  createWorkspace,
  deleteWorkspace,
  getGitStatus,
  getVcsDiffRaw,
  initGitProject,
  listWorkspaceFileTree,
  listWorkspaces,
  resolveWorkspace,
  updateWorkspace,
}

function flattenTree(nodes: WorkspaceFileNode[], depth = 0): WorkspaceTreeRow[] {
  return nodes.flatMap((node) => [{ node, depth }, ...flattenTree(node.children || [], depth + 1)])
}

function readRouteWorkspaceId(value: unknown): number | null {
  if (typeof value !== 'string') return null
  const parsed = Number(value)
  return Number.isFinite(parsed) ? parsed : null
}

function readRoutePath(value: unknown): string {
  return typeof value === 'string' ? value.trim().replace(/^\/+/, '') : ''
}

export function formatWorkspaceNodeSize(node: WorkspaceFileNode): string {
  if (node.kind === 'directory') return 'dir'
  if (node.size === null || node.size === undefined) return node.kind
  if (node.size < 1024) return `${node.size} B`
  if (node.size < 1024 * 1024) return `${(node.size / 1024).toFixed(1)} KB`
  return `${(node.size / 1024 / 1024).toFixed(1)} MB`
}

export function useWorkspacePageState(
  input: {
    route: RouteLocationNormalizedLoaded
    router: Router
  },
  deps: WorkspacePageStateDeps = defaultDeps,
) {
  const workspaces = ref<WorkspaceResource[]>([])
  const selectedWorkspaceId = ref<number | null>(null)
  const pathInput = ref('')
  const workspacePath = ref('')
  const tree = ref<WorkspaceFileTreeResource | null>(null)
  const gitStatus = ref<GitStatusResource | null>(null)
  const rawDiff = ref('')
  const rawDiffLoaded = ref(false)
  const rawDiffLoading = ref(false)
  const loading = ref(false)
  const actionError = ref('')
  const actionMessage = ref('')
  const selectedWorkspacePathDraft = ref('')

  const selectedWorkspace = computed(
    () => workspaces.value.find((workspace) => workspace.id === selectedWorkspaceId.value) || null,
  )
  const rows = computed<WorkspaceTreeRow[]>(() => flattenTree(tree.value?.entries || []))
  const selectedShortcutId = computed(
    () => workspaceShortcuts.find((shortcut) => shortcut.relativePath === pathInput.value)?.id || '',
  )
  const workspaceSummaryFacts = computed(() => {
    if (!selectedWorkspace.value) return [] as string[]
    return [
      `id=${selectedWorkspace.value.id}`,
      `sessions=${selectedWorkspace.value.session_count ?? 0}`,
      `updated=${selectedWorkspace.value.updated_at}`,
    ]
  })
  const workspaceConfigCards = computed<WorkspaceConfigCard[]>(() =>
    workspaceShortcuts.map((shortcut) => ({
      ...shortcut,
      active: shortcut.id === selectedShortcutId.value,
      entryCount: rows.value.filter((row) => row.depth === 0 && row.node.path.startsWith(shortcut.relativePath)).length,
    })),
  )
  const configSummaryFacts = computed(() => {
    const facts = [`entry_points=${workspaceShortcuts.length}`]
    if (selectedShortcutId.value) facts.push(`active=${selectedShortcutId.value}`)
    if (tree.value?.path) facts.push(`path=/${tree.value.path}`)
    return facts
  })
  const pageTitle = computed(() => 'Workspace')
  const pageDescription = computed(() => 'Browse workspace files and inspect model-produced patch diffs in Chat.')

  async function updateRouteQuery() {
    const query: Record<string, string> = {}
    if (selectedWorkspaceId.value) {
      query.workspace = String(selectedWorkspaceId.value)
    }
    const normalizedPath = pathInput.value.trim().replace(/^\/+/, '')
    if (normalizedPath) {
      query.path = normalizedPath
    }
    await input.router.replace({ path: '/workspace', query })
  }

  function syncFromRoute() {
    const routeWorkspaceId = readRouteWorkspaceId(input.route.query.workspace)
    if (routeWorkspaceId && workspaces.value.some((workspace) => workspace.id === routeWorkspaceId)) {
      selectedWorkspaceId.value = routeWorkspaceId
    }
    pathInput.value = readRoutePath(input.route.query.path)
  }

  async function loadGitStatus() {
    try {
      gitStatus.value = await deps.getGitStatus()
    } catch {
      gitStatus.value = null
    }
  }

  async function initGitProjectAction() {
    loading.value = true
    actionError.value = ''
    actionMessage.value = ''
    try {
      gitStatus.value = await deps.initGitProject()
      actionMessage.value = `Initialized git repository at ${gitStatus.value.workspace_root}.`
    } catch (err) {
      actionError.value = err instanceof Error ? err.message : String(err)
    } finally {
      loading.value = false
    }
  }

  async function loadVcsDiffRawAction() {
    rawDiffLoading.value = true
    actionError.value = ''
    actionMessage.value = ''
    try {
      rawDiff.value = await deps.getVcsDiffRaw()
      rawDiffLoaded.value = true
      actionMessage.value = rawDiff.value
        ? 'Loaded raw git diff.'
        : 'No raw git diff is available for the current workspace.'
    } catch (err) {
      actionError.value = err instanceof Error ? err.message : String(err)
    } finally {
      rawDiffLoading.value = false
    }
  }

  async function loadTree() {
    const workspaceId = selectedWorkspaceId.value
    if (!workspaceId) return
    loading.value = true
    actionError.value = ''
    try {
      tree.value = await deps.listWorkspaceFileTree({
        workspaceId,
        path: pathInput.value,
        depth: 4,
        limit: 1000,
      })
      await updateRouteQuery()
    } catch (err) {
      actionError.value = err instanceof Error ? err.message : String(err)
    } finally {
      loading.value = false
    }
  }

  async function selectWorkspace(workspaceId: number | string) {
    const parsed = Number(workspaceId)
    if (!Number.isFinite(parsed)) return
    if (!workspaces.value.some((workspace) => workspace.id === parsed)) return
    selectedWorkspaceId.value = parsed
    pathInput.value = ''
    await loadTree()
  }

  async function load() {
    loading.value = true
    actionError.value = ''
    try {
      const [workspaceItems] = await Promise.all([deps.listWorkspaces(), loadGitStatus()])
      workspaces.value = workspaceItems
      syncFromRoute()
      if (
        selectedWorkspaceId.value &&
        !workspaces.value.some((workspace) => workspace.id === selectedWorkspaceId.value)
      ) {
        selectedWorkspaceId.value = null
      }
      if (!selectedWorkspaceId.value && workspaces.value.length) {
        selectedWorkspaceId.value = workspaces.value[0]?.id ?? null
      }
      if (selectedWorkspaceId.value) {
        await loadTree()
      }
    } catch (err) {
      actionError.value = err instanceof Error ? err.message : String(err)
    } finally {
      loading.value = false
    }
  }

  function openDirectory(node: WorkspaceFileNode) {
    if (node.kind !== 'directory') return
    pathInput.value = node.path
    void loadTree()
  }

  function goRoot() {
    pathInput.value = ''
    void loadTree()
  }

  function openShortcut(relativePath: string) {
    pathInput.value = relativePath
    void loadTree()
  }

  function openSettingsForShortcut(_shortcutId: string) {
    const workspace = selectedWorkspaceId.value ? String(selectedWorkspaceId.value) : undefined
    void input.router.push({ path: buildRuntimeSectionPath('runtime', 'workflow'), query: { workspace } })
  }

  function openChatForWorkspace() {
    if (!selectedWorkspaceId.value) return
    void input.router.push({ path: '/chat', query: { workspace: String(selectedWorkspaceId.value) } })
  }

  function openRuntimeForWorkspace() {
    if (!selectedWorkspaceId.value) return
    void input.router.push({
      path: buildRuntimeSectionPath('runtime', 'workflow'),
      query: { workspace: String(selectedWorkspaceId.value) },
    })
  }

  async function resolveWorkspaceAction(createIfMissing: boolean) {
    const path = workspacePath.value.trim()
    if (!path) return
    loading.value = true
    actionError.value = ''
    actionMessage.value = ''
    try {
      const workspace = createIfMissing ? await deps.resolveWorkspace(path, true) : await deps.createWorkspace(path)
      workspacePath.value = workspace.path
      await load()
      selectedWorkspaceId.value = workspace.id
      pathInput.value = ''
      await loadTree()
      actionMessage.value = `Opened workspace ${workspace.path}.`
    } catch (err) {
      actionError.value = err instanceof Error ? err.message : String(err)
    } finally {
      loading.value = false
    }
  }

  async function updateWorkspacePathAction(workspace: WorkspaceResource, nextPath: string) {
    const normalizedPath = nextPath.trim()
    if (!normalizedPath || normalizedPath === workspace.path) return

    loading.value = true
    actionError.value = ''
    actionMessage.value = ''
    try {
      const updated = await deps.updateWorkspace({
        workspaceId: workspace.id,
        path: normalizedPath,
      })
      await load()
      selectedWorkspaceId.value = updated.id
      selectedWorkspacePathDraft.value = updated.path
      actionMessage.value = `Renamed workspace to ${updated.path}.`
    } catch (err) {
      actionError.value = err instanceof Error ? err.message : String(err)
    } finally {
      loading.value = false
    }
  }

  async function saveSelectedWorkspacePath() {
    const workspace = selectedWorkspace.value
    if (!workspace) return
    await updateWorkspacePathAction(workspace, selectedWorkspacePathDraft.value)
  }

  function resetSelectedWorkspacePathDraft() {
    selectedWorkspacePathDraft.value = selectedWorkspace.value?.path || ''
  }

  function useSelectedWorkspaceAsResolverPath() {
    workspacePath.value = selectedWorkspace.value?.path || ''
  }

  async function renameSelectedWorkspace() {
    const workspace = selectedWorkspace.value
    if (!workspace) return
    const nextPath =
      typeof window !== 'undefined' ? (window.prompt('Rename workspace path', workspace.path)?.trim() ?? '') : ''
    await updateWorkspacePathAction(workspace, nextPath)
  }

  async function deleteWorkspaceAction(workspace: WorkspaceResource) {
    if (typeof window !== 'undefined' && !window.confirm(`Delete workspace #${workspace.id} (${workspace.path})?`)) {
      return
    }

    loading.value = true
    actionError.value = ''
    actionMessage.value = ''
    try {
      const removed = await deps.deleteWorkspace(workspace.id)
      await load()
      if (!workspaces.value.length) {
        selectedWorkspaceId.value = null
        tree.value = null
      }
      actionMessage.value = `Deleted workspace ${removed.path}.`
    } catch (err) {
      actionError.value = err instanceof Error ? err.message : String(err)
    } finally {
      loading.value = false
    }
  }

  async function deleteSelectedWorkspace() {
    const workspace = selectedWorkspace.value
    if (!workspace) return
    await deleteWorkspaceAction(workspace)
  }

  watch(
    () => [input.route.query.workspace, input.route.query.path],
    () => {
      syncFromRoute()
      if (selectedWorkspaceId.value) {
        void loadTree()
      }
    },
  )

  watch(
    () => selectedWorkspace.value?.path,
    (path) => {
      selectedWorkspacePathDraft.value = path || ''
    },
    { immediate: true },
  )

  return {
    actionError,
    actionMessage,
    configSummaryFacts,
    formatSize: formatWorkspaceNodeSize,
    gitStatus,
    initGitProjectAction,
    loadVcsDiffRawAction,
    goRoot,
    load,
    loadTree,
    loading,
    deleteWorkspaceAction,
    openChatForWorkspace,
    openDirectory,
    openRuntimeForWorkspace,
    openSettingsForShortcut,
    openShortcut,
    pageDescription,
    pageTitle,
    pathInput,
    rawDiff,
    rawDiffLoaded,
    rawDiffLoading,
    renameSelectedWorkspace,
    resetSelectedWorkspacePathDraft,
    resolveWorkspaceAction,
    rows,
    selectedShortcutId,
    selectedWorkspace,
    selectedWorkspaceId,
    selectedWorkspacePathDraft,
    selectWorkspace,
    saveSelectedWorkspacePath,
    deleteSelectedWorkspace,
    tree,
    useSelectedWorkspaceAsResolverPath,
    workspaceConfigCards,
    workspacePath,
    workspaces,
    workspaceSummaryFacts,
    syncFromRoute,
  }
}
