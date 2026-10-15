import { afterAll, expect, test } from 'bun:test'
import { createPinia, setActivePinia } from 'pinia'
import { createServer } from 'vite'
import { fileURLToPath } from 'node:url'
import { effectScope, nextTick, ref } from 'vue'
import { ensureBrowserTestRuntime } from './testRuntime'

ensureBrowserTestRuntime()
const vite = await createServer({
  root: fileURLToPath(new URL('..', import.meta.url)),
  server: { middlewareMode: true, watch: null, hmr: false },
})
afterAll(() => vite.close())
const { resourceDirectory } = await vite.ssrLoadModule('/src/app/workspace/resourceDirectory.ts')
const { useUiStore } = await vite.ssrLoadModule('/src/stores/ui.ts')
const { useDirectoryStoreUnscoped } = await vite.ssrLoadModule('/src/stores/directory.ts')
const { useGitReposStore } = await vite.ssrLoadModule('/src/stores/gitRepos.ts')

test('file links preserve their original folder while another project is selected', () => {
  expect(resourceDirectory({ directory: '/projects/a', filePath: '/projects/a/src/index.ts' }, '/projects/b')).toBe(
    '/projects/a',
  )
  expect(resourceDirectory({ filePath: '/projects/ab/index.ts' }, '/projects/a')).toBe('/projects/ab')
  expect(resourceDirectory({ filePath: 'D:\\work\\file.ts' }, 'C:/work')).toBe('D:/work')
  expect(resourceDirectory({ filePath: '/README.md' })).toBe('/')
  expect(resourceDirectory({ filePath: '/projects/a/file.ts' }, '/')).toBe('/')
  expect(resourceDirectory({ filePath: 'C:/work/file.ts' }, 'C:/')).toBe('C:/')
})

test('repository navigation uses fast discovery by default and opens the resolved repository root', async () => {
  const { useGitRepoSelection } = await vite.ssrLoadModule('/src/pages/git/useGitRepoSelection.ts')
  const scopes: unknown[] = []
  const opened: string[] = []
  const state = useGitRepoSelection({
    projectRoot: ref('/projects'),
    selectedRepoRelative: ref('.'),
    gitRepos: { getClosedRelatives: () => [], setSelectedRelative() {}, closeRepo() {}, reopenRepo() {} },
    toasts: { push() {} },
    load: async () => {},
    switchProjectRoot() {},
    openRepo: (path: string) => opened.push(path),
    gitJson: async (_endpoint: string, _directory: string, query: Record<string, unknown>) => {
      scopes.push(query.scope)
      return { repos: [{ relative: 'nested/repo', root: '/projects/nested/repo' }] }
    },
  })
  await state.loadRepos()
  await state.loadRepoPickerPage()
  await state.loadRepoPickerPage({ scan: true })
  expect(scopes).toEqual(['current', 'current', 'all'])
  state.selectRepo('nested/repo')
  expect(opened).toEqual(['/projects/nested/repo'])
})

test('a branch already checked out in another worktree opens that worktree without checkout', async () => {
  const { useGitBranches } = await vite.ssrLoadModule('/src/pages/git/useGitBranches.ts')
  const opened: string[] = []
  const checkouts: string[] = []
  const state = useGitBranches({
    root: ref('/repo'),
    toasts: { push() {} },
    withRepoBusy: (_op: string, run: () => Promise<void>) => run(),
    handleGitBusy: () => false,
    load: async () => {},
    openWorktree: (path: string) => opened.push(path),
    gitJson: async (endpoint: string, _dir: string, _query: unknown, init?: RequestInit) => {
      if (endpoint === 'checkout') checkouts.push(JSON.parse(String(init?.body)).branch)
      return { branches: { feature: { name: 'feature', worktreePath: '/sibling worktree' } } }
    },
  })
  await state.loadBranches()
  await state.checkoutBranch('feature')
  expect(opened).toEqual(['/sibling worktree'])
  expect(checkouts).toEqual([])
  await state.checkoutBranch('remotes/company/review')
  expect(checkouts).toEqual(['remotes/company/review'])
})

const deferred = <T>() => {
  let resolve!: (value: T) => void
  let reject!: (error: Error) => void
  const promise = new Promise<T>((done, fail) => {
    resolve = done
    reject = fail
  })
  return { promise, resolve, reject }
}

test('comparison responses cannot cross repository or request boundaries', async () => {
  const { useGitCompareOps } = await vite.ssrLoadModule('/src/pages/git/useGitCompareOps.ts')
  const scope = effectScope()
  try {
    const root = ref('/repo')
    const requests: Array<ReturnType<typeof deferred<{ diff: string }>>> = []
    const state = scope.run(() =>
      useGitCompareOps({
        repoRoot: root,
        toasts: { push() {} },
        gitJson: () => {
          const request = deferred<{ diff: string }>()
          requests.push(request)
          return request.promise
        },
      }),
    )!
    const old = state.runCompare({ base: 'main', head: 'feature' })
    const newer = state.runCompare({ base: 'main', head: 'review' })
    requests[1]!.resolve({ diff: 'review diff' })
    await newer
    requests[0]!.resolve({ diff: 'obsolete feature diff' })
    await old
    expect(state.compareDiff.value).toBe('review diff')
    const pending = state.runCompare()
    root.value = '/other-worktree'
    await nextTick()
    requests[2]!.reject(new Error('old repository failed'))
    await pending
    expect(state.compareDiff.value).toBe('')
    expect(state.compareError.value).toBeNull()
    expect(state.compareLoading.value).toBe(false)
  } finally {
    scope.stop()
  }
})

test('history can inspect a remote branch without checkout and ignores the previous branch response', async () => {
  const { useGitHistoryLog } = await vite.ssrLoadModule('/src/pages/git/useGitHistoryLog.ts')
  const scope = effectScope()
  try {
    const root = ref('/repo')
    const requests: Array<{ reference: unknown; request: ReturnType<typeof deferred<unknown>> }> = []
    const state = scope.run(() =>
      useGitHistoryLog({
        repoRoot: root,
        gitJson: (endpoint: string, _dir: string, query: Record<string, unknown>) => {
          expect(endpoint).toBe('log')
          const request = deferred<unknown>()
          requests.push({ reference: query.ref, request })
          return request.promise
        },
      }),
    )!
    const head = state.loadHistoryPage(1)
    state.historyRef.value = 'refs/remotes/company/review'
    await nextTick()
    expect(requests.map((item) => item.reference)).toEqual(['HEAD', 'refs/remotes/company/review'])
    requests[1]!.request.resolve({ commits: [{ hash: 'remote-commit' }], hasMore: false })
    await nextTick()
    requests[0]!.request.resolve({ commits: [{ hash: 'old-head' }], hasMore: false })
    await head
    expect(state.historyCommits.value.map((item: { hash: string }) => item.hash)).toEqual(['remote-commit'])
    root.value = '/other-repo'
    await nextTick()
    expect(state.historyRef.value).toBe('HEAD')
    expect(state.historyCommits.value).toEqual([])
    requests.at(-1)!.request.resolve({ commits: [], hasMore: false })
    await nextTick()
  } finally {
    scope.stop()
  }
})

test('activating file and diff tabs restores their folder before router/page hydration', () => {
  setActivePinia(createPinia())
  const ui = useUiStore()
  ui.closeAllWorkspaceWindows()
  const directories = useDirectoryStoreUnscoped()
  const a = ui.openWorkspaceWindow('files', { query: { directory: '/a', filePath: '/a/src/file.ts' } })
  expect(directories.currentDirectory).toBe('/a')
  const b = ui.openWorkspaceWindow('git', {
    query: { directory: '/b', repoRoot: '/b/worktree', gitFile: 'src/file.ts', gitSource: 'staged' },
  })
  expect(directories.currentDirectory).toBe('/b')
  directories.setDirectory('/manually-selected')
  expect(directories.currentDirectory).toBe('/manually-selected')
  ui.selectWorkspaceWindow(a)
  expect(directories.currentDirectory).toBe('/a')
  ui.selectWorkspaceWindow(b)
  expect(directories.currentDirectory).toBe('/b')
  const browser = ui.openWorkspaceWindow('git', { query: { directory: '/b' } })
  expect(browser).not.toBe(b)
  expect(ui.getWorkspaceWindowById(b).routeQuery.gitFile).toBe('src/file.ts')
})

test('two Git tabs for the same folder retain independent repository and diff selections', () => {
  setActivePinia(createPinia())
  const repos = useGitReposStore()
  repos.setSelectedRelative('/projects', 'a', 'view-a')
  repos.setSelectedRelative('/projects', 'b', 'view-b')
  expect(repos.getSelectedRelative('/projects', 'view-a')).toBe('a')
  expect(repos.getSelectedRelative('/projects', 'view-b')).toBe('b')
  repos.setOpenItem('/repo', 'one.ts', 'working', 'view-a')
  repos.setOpenItem('/repo', 'two.ts', 'staged', 'view-b')
  expect(repos.getOpenItem('/repo', 'view-a')).toEqual({ path: 'one.ts', source: 'working' })
  expect(repos.getOpenItem('/repo', 'view-b')).toEqual({ path: 'two.ts', source: 'staged' })
})
