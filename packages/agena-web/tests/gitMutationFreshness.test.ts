import { afterAll, expect, test } from 'bun:test'
import { createServer } from 'vite'
import { fileURLToPath } from 'node:url'
import { effectScope, ref } from 'vue'
import { ensureBrowserTestRuntime } from './testRuntime'

ensureBrowserTestRuntime()
const vite = await createServer({
  root: fileURLToPath(new URL('..', import.meta.url)),
  server: { middlewareMode: true, watch: null, hmr: false },
})
afterAll(() => vite.close())
const deferred = <T>() => {
  let resolve!: (value: T) => void, reject!: (error: Error) => void
  const promise = new Promise<T>((done, fail) => {
    resolve = done
    reject = fail
  })
  return { promise, resolve, reject }
}
const toasts = {
  push(kind: string, message: string) {
    if (kind === 'error') throw new Error(message)
  },
}

for (const [file, factory, load, field, oldValue, newValue] of [
  [
    'useGitBranches',
    'useGitBranches',
    'loadBranches',
    'branches',
    { current: 'old', branches: {} },
    { current: 'new', branches: {} },
  ],
  ['useGitTags', 'useGitTags', 'loadTags', 'tagsList', { tags: [{ name: 'old' }] }, { tags: [{ name: 'new' }] }],
  ['useGitWorktrees', 'useGitWorktrees', 'loadWorktrees', 'worktrees', [{ path: '/old' }], [{ path: '/new' }]],
  [
    'useGitRemotesOps',
    'useGitRemotesOps',
    'loadRemotes',
    'remoteInfo',
    { remotes: [{ name: 'old' }] },
    { remotes: [{ name: 'new' }] },
  ],
  [
    'useGitSubmoduleOps',
    'useGitSubmoduleOps',
    'loadSubmodules',
    'submodules',
    { submodules: [{ path: 'old' }] },
    { submodules: [{ path: 'new' }] },
  ],
] as const) {
  test(`${load} retains the post-mutation response when an earlier read or failure arrives last`, async () => {
    const module = await vite.ssrLoadModule(`/src/pages/git/${file}.ts`)
    const root = ref<string | null>('/repo')
    const requests: Array<ReturnType<typeof deferred<unknown>>> = []
    const remoteInfo = ref(null)
    const state = module[factory]({
      root,
      repoRoot: root,
      remoteInfo,
      toasts,
      gitJson: () => {
        const request = deferred<unknown>()
        requests.push(request)
        return request.promise
      },
      withRepoBusy: (_op: string, run: () => Promise<void>) => run(),
      handleGitBusy: () => false,
      load: async () => {},
      refreshWorkingTree: async () => {},
      preferredRemote: () => 'origin',
    })
    const snapshot = () => (field === 'remoteInfo' ? remoteInfo.value : state[field].value)
    const old = state[load]()
    const current = state[load]()
    requests[1]!.resolve(newValue)
    await current
    const expected = snapshot()
    requests[0]!.resolve(oldValue)
    await old
    expect(snapshot()).toBe(expected)
    const failingOld = state[load]()
    const latest = state[load]()
    requests[3]!.resolve(newValue)
    await latest
    requests[2]!.reject(new Error('obsolete failure'))
    await failingOld
    expect(snapshot()).toEqual(expected)
    // A repository change also invalidates a reader without a replacement read.
    const retired = state[load]()
    root.value = '/other-repo'
    requests[4]!.resolve(oldValue)
    await retired
    expect(snapshot()).toEqual(expected)
  })
}

test('creating a tag refreshes the list and a pre-create read cannot undo it', async () => {
  const { useGitTags } = await vite.ssrLoadModule('/src/pages/git/useGitTags.ts')
  const requests: Array<ReturnType<typeof deferred<unknown>>> = []
  const state = useGitTags({
    repoRoot: ref('/repo'),
    toasts,
    preferredRemote: () => 'origin',
    withRepoBusy: (_op: string, run: () => Promise<void>) => run(),
    handleGitBusy: () => false,
    gitJson: (_endpoint: string, _directory: string, _query: unknown, init?: RequestInit) => {
      if (init?.method === 'POST') return Promise.resolve({})
      const request = deferred<unknown>()
      requests.push(request)
      return request.promise
    },
  })
  const old = state.loadTags()
  state.newTagName.value = 'v-new'
  const create = state.createTag()
  for (let i = 0; i < 10; i++) await Promise.resolve()
  expect(requests.length).toBe(2)
  requests[1]!.resolve({ tags: [{ name: 'v-new' }] })
  await create
  requests[0]!.resolve({ tags: [] })
  await old
  expect(state.tagsList.value.map((row: { name: string }) => row.name)).toEqual(['v-new'])
})

test('a repository discovery response preserves a newer search page', async () => {
  const { useGitRepoSelection } = await vite.ssrLoadModule('/src/pages/git/useGitRepoSelection.ts')
  const requests: Array<ReturnType<typeof deferred<unknown>>> = []
  const state = useGitRepoSelection({
    projectRoot: ref('/repo'),
    selectedRepoRelative: ref('.'),
    toasts,
    gitRepos: { getClosedRelatives: () => [], setSelectedRelative() {}, closeRepo() {}, reopenRepo() {} },
    gitJson: () => {
      const request = deferred<unknown>()
      requests.push(request)
      return request.promise
    },
    load: async () => {},
    switchProjectRoot() {},
  })
  const discovery = state.loadRepos()
  const search = state.loadRepoPickerPage({ page: 3, search: 'fresh' })
  requests[1]!.resolve({
    repos: [{ relative: 'fresh-result' }],
    page: 3,
    pageSize: 30,
    total: 100,
    totalPages: 4,
    search: 'fresh',
  })
  await search
  requests[0]!.resolve({ repos: [{ relative: '.' }], page: 1, total: 1, totalPages: 1 })
  await discovery
  expect(state.repos.value.map((row: { relative: string }) => row.relative)).toEqual(['.'])
  expect(state.repoPickerRepos.value.map((row: { relative: string }) => row.relative)).toEqual(['fresh-result'])
  expect(state.repoPickerPage.value).toBe(3)
  expect(state.repoPickerSearch.value).toBe('fresh')
})

test('submodule init/update and worktree migration refresh their working-tree projections after success', async () => {
  const { useGitSubmoduleOps } = await vite.ssrLoadModule('/src/pages/git/useGitSubmoduleOps.ts')
  const { useGitWorktrees } = await vite.ssrLoadModule('/src/pages/git/useGitWorktrees.ts')
  const calls: string[] = []
  let refreshed = 0
  const opts = {
    repoRoot: ref('/repo'),
    toasts,
    withRepoBusy: (_op: string, run: () => Promise<void>) => run(),
    handleGitBusy: () => false,
    refreshWorkingTree: async () => {
      refreshed++
    },
    gitJson: async (endpoint: string) => {
      calls.push(endpoint)
      return endpoint === 'submodules' ? { submodules: [{ path: 'module', initialized: true }] } : []
    },
  }
  const submodules = useGitSubmoduleOps(opts)
  await submodules.initSubmodule('module')
  await submodules.updateSubmodule('module')
  const worktrees = useGitWorktrees(opts)
  await worktrees.migrateWorktreeChanges('/source')
  expect(calls).toEqual([
    'submodules/init',
    'submodules',
    'submodules/update',
    'submodules',
    'worktrees/migrate',
    'worktrees',
  ])
  expect(refreshed).toBe(3)
})

test('LFS locks cannot regress when refreshes finish out of order', async () => {
  const { useGitLfsOps } = await vite.ssrLoadModule('/src/pages/git/useGitLfsOps.ts')
  const requests: Array<ReturnType<typeof deferred<unknown>>> = []
  const state = useGitLfsOps({
    repoRoot: ref('/repo'),
    toasts,
    withRepoBusy: (_op: string, run: () => Promise<void>) => run(),
    handleGitBusy: () => false,
    gitJson: (endpoint: string) => {
      if (endpoint === 'lfs') return Promise.resolve({ installed: true, tracked: [] })
      const request = deferred<unknown>()
      requests.push(request)
      return request.promise
    },
  })
  const old = state.refreshLfs()
  for (let i = 0; i < 10; i++) await Promise.resolve()
  const current = state.refreshLfs()
  for (let i = 0; i < 10; i++) await Promise.resolve()
  requests[1]!.resolve({ locks: [{ path: 'fresh' }] })
  await current
  requests[0]!.resolve({ locks: [{ path: 'old' }] })
  await old
  expect(state.lfsLocks.value.map((row: { path: string }) => row.path)).toEqual(['fresh'])
})

test('remote branch caches expire after successful fetch and old in-flight reads cannot refill them', async () => {
  const { useGitRemoteBranchPicker } = await vite.ssrLoadModule('/src/pages/git/useGitRemoteBranchPicker.ts')
  const { gitJson } = await vite.ssrLoadModule('/src/lib/gitApi.ts')
  const originalFetch = globalThis.fetch
  const scope = effectScope()
  const requests: Array<ReturnType<typeof deferred<Response>>> = []
  globalThis.fetch = ((_input, init) => {
    if (init?.method === 'POST') return Promise.resolve(Response.json({}))
    const request = deferred<Response>()
    requests.push(request)
    return request.promise
  }) as typeof fetch
  const settle = async () => {
    for (let i = 0; i < 6; i++) await new Promise<void>((resolve) => setImmediate(resolve))
  }
  try {
    const picker = scope.run(() =>
      useGitRemoteBranchPicker({
        gitJson,
        repoRoot: ref('/repo'),
        pushToOpen: ref(false),
        pullFromOpen: ref(false),
        fetchFromOpen: ref(false),
        targetRemote: ref('origin'),
        targetBranch: ref(''),
      }),
    )!
    picker.prefetchRemoteBranches('origin')
    requests[0]!.resolve(Response.json({ branches: ['original'] }))
    await settle()
    picker.prefetchRemoteBranches('origin')
    await settle()
    expect(requests).toHaveLength(1)
    await gitJson('fetch', '/repo', undefined, { method: 'POST' })
    picker.prefetchRemoteBranches('origin')
    expect(requests).toHaveLength(2)
    requests[1]!.resolve(Response.json({ branches: ['after-fetch'] }))
    await settle()
    expect(picker.filteredRemoteBranchOptions.value).toEqual(['after-fetch'])
    await gitJson('fetch', '/repo', undefined, { method: 'POST' })
    picker.prefetchRemoteBranches('origin')
    await gitJson('fetch', '/repo', undefined, { method: 'POST' })
    picker.prefetchRemoteBranches('origin')
    requests[3]!.resolve(Response.json({ branches: ['latest'] }))
    await settle()
    requests[2]!.resolve(Response.json({ branches: ['obsolete'] }))
    await settle()
    expect(picker.filteredRemoteBranchOptions.value).toEqual(['latest'])
    picker.prefetchRemoteBranches('origin')
    await settle()
    expect(requests).toHaveLength(4)
    expect(picker.filteredRemoteBranchOptions.value).toEqual(['latest'])
  } finally {
    scope.stop()
    globalThis.fetch = originalFetch
  }
})
