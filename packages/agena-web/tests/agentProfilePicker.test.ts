import { afterEach, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { transformSync } from 'esbuild'
import * as vue from 'vue'
import * as agentProfiles from '../src/lib/agentProfiles'
import * as chatState from '../src/types/chat'
import type { Session, SessionState } from '../src/types/chat'

type Picker = ReturnType<typeof import('../src/pages/chat/useChatProfilePicker').useChatProfilePicker>
type Execution = {
  profile: string
  profile_is_default: boolean
  queued_messages?: unknown[]
  model_id?: string
}
type ServerSession = { session: Session; execution: Execution; queued_messages: unknown[] }
type PickerKind = 'profile' | 'model' | 'thinking' | 'speed' | null

const source = readFileSync(new URL('../src/pages/chat/useChatProfilePicker.ts', import.meta.url), 'utf8')
const compiled = transformSync(source, { loader: 'ts', format: 'cjs' }).code
const scopes: vue.EffectScope[] = []
afterEach(() => {
  for (const scope of scopes.splice(0)) scope.stop()
})

function copy<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T
}

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((yes, no) => {
    resolve = yes
    reject = no
  })
  return { promise, resolve, reject }
}

async function settle() {
  for (let index = 0; index < 4; index++) {
    await Promise.resolve()
    await vue.nextTick()
  }
}

function fixture(options: { global?: boolean; subagent?: boolean; profile?: string | null } = {}) {
  const builtins = {
    main: { title: 'Main', description: 'Coordinate work', mode: 'primary' },
    worker: { title: 'Worker', mode: 'subagent' },
    explore: { title: 'Explore', description: 'Find evidence', mode: 'subagent' },
    reviewer: { title: 'Reviewer', mode: 'all' },
  }
  const documents: Record<'global' | 'workspace', Record<string, any>> = {
    global: {
      main: { model: { provider: 'provider', model: 'main-model' } },
      globalOnly: { title: 'Global only', mode: 'all' },
      shared: { title: 'Global shared', description: 'Global instructions', mode: 'all' },
      hiddenReview: { title: 'Hidden reviewer', mode: 'all', hidden: true },
      inheritedExplore: { title: 'Inherited explorer', extends: 'explore' },
    },
    workspace: {
      project: { title: 'Project work', mode: 'primary' },
      shared: { title: 'Workspace shared', mode: 'primary' },
    },
  }
  const ready = (): SessionState => ({ kind: 'ready', data: {} })
  function record(id: string, input = options): ServerSession {
    const fallback = input.subagent ? 'worker' : 'main'
    const profile = input.profile === undefined ? fallback : input.profile
    return {
      session: {
        id,
        workspace_id: input.global ? null : 1,
        relation_kind: input.subagent ? 'subagent' : 'root',
        state: ready(),
      } as Session,
      execution: { profile: profile || fallback, profile_is_default: !profile, model_id: 'original-model' },
      queued_messages: [],
    }
  }
  const records: Record<string, ServerSession> = {
    '9': record('9'),
    '10': record('10', { profile: 'reviewer' }),
  }
  const backend = vue.ref(1)
  const directory = vue.ref('/project')
  const locallyBusy = vue.ref(false)
  const open = vue.ref<PickerKind>(null)
  const reads: Array<{ path: string; backend: number; signal?: AbortSignal }> = []
  const writes: Array<{ id: string; profile: string; backend: number }> = []
  const refreshes: Array<{ id: string; forceFresh?: boolean; backend: number }> = []
  const defaults: Array<{ scope: string; id: string }> = []
  const settingsListeners = new Set<() => void>()
  let closes = 0

  function executionFor(id: string): Execution {
    return { ...copy(records[id]!.execution), queued_messages: copy(records[id]!.queued_messages) }
  }
  const chat = vue.reactive({
    selectedSessionId: '9' as string | null,
    selectedSession: copy(records['9']!.session) as Session | null,
    selectedSessionState: copy(records['9']!.session.state!) as SessionState,
    selectedSessionExecution: executionFor('9') as Execution | null,
    async refreshExecutionStatus(id: string, forceFresh?: boolean) {
      const requestBackend = backend.value
      refreshes.push({ id, forceFresh, backend: requestBackend })
      await hooks.refresh(id, requestBackend)
    },
  })
  const hooks = {
    async sources(path: string, _signal: AbortSignal | undefined, _backend: number): Promise<any> {
      if (path === 'builtin_profiles') return { effective: { value: copy(builtins) } }
      if (path !== 'profiles') throw new Error('Unexpected settings path: ' + path)
      return {
        global: { value: copy(documents.global) },
        workspace: { value: copy(documents.workspace) },
        effective: { value: { ...copy(documents.global), ...copy(documents.workspace) } },
      }
    },
    async summary(id: string, _signal: AbortSignal | undefined, _backend: number): Promise<Session> {
      return copy(records[id]!.session)
    },
    async execution(id: string, _backend: number): Promise<any> {
      return copy(records[id]!)
    },
    async patch(id: string, profile: string, _backend: number): Promise<unknown> {
      const current = records[id]!
      current.execution = {
        ...current.execution,
        profile: profile || (current.session.relation_kind === 'subagent' ? 'worker' : 'main'),
        profile_is_default: !profile,
        model_id: profile === 'reviewer' ? 'review-model' : 'role-model',
      }
      return copy(current.session)
    },
    async refresh(id: string, _backend: number): Promise<void> {
      if (chat.selectedSessionId === id) {
        chat.selectedSessionExecution = executionFor(id)
        chat.selectedSessionState = copy(records[id]!.session.state!)
      }
    },
  }
  const modules: Record<string, unknown> = {
    vue,
    '@/lib/agentProfiles': agentProfiles,
    '@/lib/runtimeSettings': {
      readRuntimeSettingSources(path: string, signal?: AbortSignal) {
        const requestBackend = backend.value
        reads.push({ path, signal, backend: requestBackend })
        return hooks.sources(path, signal, requestBackend)
      },
      subscribeRuntimeSettingsChanges(listener: () => void) {
        settingsListeners.add(listener)
        return () => settingsListeners.delete(listener)
      },
    },
    '@/lib/resourceSync': { captureResourceObservation: () => ({ scope: backend.value }) },
    '@/stores/chat/api': {
      getSession: (id: string, signal?: AbortSignal) => hooks.summary(id, signal, backend.value),
      getSessionExecution: (id: string) => hooks.execution(id, backend.value),
      patchSessionMetadata(id: string, patch: { profile: string }) {
        const requestBackend = backend.value
        writes.push({ id, profile: patch.profile, backend: requestBackend })
        return hooks.patch(id, patch.profile, requestBackend)
      },
    },
    '@/types/chat': chatState,
    '@/i18n/settingsText': { settingsText: (text: string) => text },
  }
  const module = { exports: {} as { useChatProfilePicker: (options: any) => Picker } }
  new Function('require', 'module', 'exports', compiled)(
    (id: string) => {
      if (!(id in modules)) throw new Error('Unexpected dependency: ' + id)
      return modules[id]
    },
    module,
    module.exports,
  )
  const scope = vue.effectScope()
  scopes.push(scope)
  const picker = scope.run(() =>
    module.exports.useChatProfilePicker({
      chat,
      open,
      directory: () => directory.value,
      locallyBusy: () => locallyBusy.value,
      close: () => {
        closes++
        open.value = null
      },
      openDefaults: (scope: string, id: string) => defaults.push({ scope, id }),
    }),
  )!

  return {
    picker,
    chat,
    hooks,
    records,
    documents,
    backend,
    directory,
    locallyBusy,
    open,
    reads,
    writes,
    refreshes,
    defaults,
    scope,
    closes: () => closes,
    async show() {
      // Navigation closes the previous menu before a later user event reopens it.
      await vue.nextTick()
      open.value = 'profile'
      await settle()
    },
    switchSession(id: string) {
      chat.selectedSession = copy(records[id]!.session)
      chat.selectedSessionState = copy(records[id]!.session.state!)
      chat.selectedSessionExecution = executionFor(id)
      chat.selectedSessionId = id
    },
    items() {
      return picker.groups.value.flatMap((group) => group.items)
    },
    roles() {
      return picker.groups.value.find((group) => group.id === 'profile-roles')?.items || []
    },
    item(id: string) {
      return picker.groups.value.flatMap((group) => group.items).find((item) => item.id === id)
    },
    settingsChanged() {
      for (const listener of settingsListeners) listener()
    },
    listenerCount: () => settingsListeners.size,
  }
}

test('global sessions use global and built-in profiles, excluding workspace replacements', async () => {
  const f = fixture({ global: true })
  await f.show()
  expect(f.picker.loading.value).toBe(false)
  expect(f.roles().map((item) => item.id)).toContain('profile:select:globalOnly')
  expect(f.roles().map((item) => item.id)).not.toContain('profile:select:project')
  expect(f.item('profile:select:shared')?.label).toBe('Global shared')
  expect(f.item('profile:select:main')?.label).toBe('Main')
  expect(f.roles().map((item) => item.id)).not.toContain('profile:select:explore')
})

test('workspace profiles replace whole global entries and inherit built-in role metadata', async () => {
  const f = fixture()
  await f.show()
  expect(f.item('profile:select:project')?.label).toBe('Project work')
  expect(f.item('profile:select:shared')?.label).toBe('Workspace shared')
  expect(f.item('profile:select:shared')?.description).not.toContain('Global instructions')
  expect(f.item('profile:select:main')?.label).toBe('Main')
})

test('subagents discover compatible inherited modes and allow manual hidden-profile selection', async () => {
  const f = fixture({ subagent: true })
  await f.show()
  const ids = f.roles().map((item) => item.id)
  expect(ids).toContain('profile:select:worker')
  expect(ids).toContain('profile:select:explore')
  expect(ids).toContain('profile:select:inheritedExplore')
  expect(ids).toContain('profile:select:hiddenReview')
  expect(ids).not.toContain('profile:select:main')
  expect(ids).not.toContain('profile:select:project')
  await f.picker.select('profile:select:main')
  expect(f.writes).toHaveLength(0)
  await f.picker.select('profile:select:hiddenReview')
  expect(f.writes).toEqual([{ id: '9', profile: 'hiddenReview', backend: 1 }])
})

test('saving writes the captured profile, forces full refresh, and closes after refresh completes', async () => {
  const f = fixture()
  await f.show()
  const refreshed = deferred<void>()
  const original = f.hooks.refresh
  f.hooks.refresh = async (id, backend) => {
    await refreshed.promise
    await original(id, backend)
  }
  const saving = f.picker.select('profile:select:reviewer')
  await settle()
  expect(f.writes).toEqual([{ id: '9', profile: 'reviewer', backend: 1 }])
  expect(f.refreshes).toEqual([{ id: '9', forceFresh: true, backend: 1 }])
  expect(f.picker.saving.value).toBe(true)
  expect(f.open.value).toBe('profile')
  expect(f.closes()).toBe(0)
  expect(f.roles().every((item) => item.disabled)).toBe(true)
  await f.picker.select('profile:select:main')
  expect(f.writes).toHaveLength(1)
  refreshed.resolve()
  await saving
  expect(f.chat.selectedSessionExecution?.profile).toBe('reviewer')
  expect(f.chat.selectedSessionExecution?.model_id).toBe('review-model')
  expect(f.picker.saving.value).toBe(false)
  expect(f.open.value).toBeNull()
  expect(f.closes()).toBe(1)
})

for (const subagent of [false, true]) {
  test('restore default sends an empty profile for ' + (subagent ? 'subagents' : 'primary sessions'), async () => {
    const f = fixture({ subagent, profile: 'reviewer' })
    await f.show()
    const fallback = subagent ? 'worker' : 'main'
    expect(f.item('profile:restore')?.label).toContain(fallback)
    expect(f.item('profile:restore')?.checked).toBe(false)
    await f.picker.select('profile:restore')
    expect(f.writes).toEqual([{ id: '9', profile: '', backend: 1 }])
    expect(f.chat.selectedSessionExecution).toMatchObject({ profile: fallback, profile_is_default: true })
  })
}

test('explicit main and automatic main have distinct checked states', async () => {
  const explicit = fixture({ profile: 'main' })
  const automatic = fixture({ profile: null })
  await explicit.show()
  await automatic.show()
  expect(explicit.item('profile:restore')?.checked).toBe(false)
  expect(explicit.item('profile:select:main')?.checked).toBe(true)
  expect(automatic.item('profile:restore')?.checked).toBe(true)
  expect(automatic.item('profile:select:main')?.checked).toBe(false)
})

for (const kind of ['creating', 'running', 'awaiting_interaction'] as const) {
  test('profile mutation is disabled while session is ' + kind, async () => {
    const f = fixture()
    f.records['9']!.session.state = { kind, data: {} } as SessionState
    f.chat.selectedSessionState = copy(f.records['9']!.session.state!)
    await f.show()
    expect(f.roles().every((item) => item.disabled)).toBe(true)
    await f.picker.select('profile:select:reviewer')
    expect(f.writes).toHaveLength(0)
    expect(f.picker.helperText.value).toContain('Finish or stop')
  })
}

test('local sends and queued messages disable changes, while a failed idle session can change profile', async () => {
  const f = fixture()
  await f.show()
  f.locallyBusy.value = true
  await f.picker.select('profile:select:reviewer')
  expect(f.writes).toHaveLength(0)
  f.locallyBusy.value = false
  f.chat.selectedSessionExecution = {
    ...f.chat.selectedSessionExecution!,
    queued_messages: [{ execution_id: 'queued', message_id: 1 }],
  }
  expect(f.item('profile:restore')?.disabled).toBe(true)
  await f.picker.select('profile:restore')
  expect(f.writes).toHaveLength(0)
  f.chat.selectedSessionExecution = { ...f.chat.selectedSessionExecution!, queued_messages: [] }
  f.chat.selectedSessionState = { kind: 'failed', data: { failure: 'previous run failed' } }
  expect(f.item('profile:restore')?.disabled).toBe(false)
  await f.picker.select('profile:restore')
  expect(f.writes).toHaveLength(1)
})

test('fresh top-level queue snapshot blocks stale idle store, then live queue retirement unlocks the menu', async () => {
  const f = fixture()
  f.records['9']!.queued_messages = [{ execution_id: 'paused', message_id: 2 }]
  await f.show()
  expect(f.chat.selectedSessionExecution?.queued_messages).toEqual([])
  expect(f.item('profile:select:reviewer')?.disabled).toBe(true)
  await f.picker.select('profile:select:reviewer')
  expect(f.writes).toHaveLength(0)
  f.records['9']!.queued_messages = []
  f.chat.selectedSessionExecution = { ...f.chat.selectedSessionExecution!, queued_messages: [] }
  expect(f.item('profile:select:reviewer')?.disabled).toBe(false)
  await f.picker.select('profile:select:reviewer')
  expect(f.writes).toHaveLength(1)
})

test('a live store update during load wins over a delayed execution snapshot', async () => {
  const f = fixture()
  const stale = copy(f.records['9']!)
  const delayed = deferred<ServerSession>()
  f.hooks.execution = async () => delayed.promise
  await f.show()
  expect(f.picker.loading.value).toBe(true)
  f.chat.selectedSessionExecution = {
    profile: 'reviewer',
    profile_is_default: false,
    queued_messages: [{ execution_id: 'new', message_id: 3 }],
  }
  delayed.resolve(stale)
  await settle()
  expect(f.item('profile:select:reviewer')?.checked).toBe(true)
  expect(f.item('profile:select:reviewer')?.disabled).toBe(true)
})

test('a missing selected profile can be repaired using the independent global session summary', async () => {
  const f = fixture({ global: true, profile: 'removed' })
  f.chat.selectedSession = { ...f.chat.selectedSession!, workspace_id: 1 }
  const original = f.hooks.execution
  f.hooks.execution = async (id, backend) => {
    if (f.records[id]!.execution.profile === 'removed') throw new Error('Unknown profile: removed')
    return original(id, backend)
  }
  await f.show()
  expect(f.picker.loading.value).toBe(false)
  expect(f.item('profile:select:reviewer')?.disabled).toBe(false)
  expect(f.item('profile:select:project')).toBeUndefined()
  await f.picker.select('profile:select:reviewer')
  expect(f.chat.selectedSessionExecution?.profile).toBe('reviewer')
})

test('failed profile writes keep the menu open and permit retry without refreshing a failed mutation', async () => {
  const f = fixture()
  await f.show()
  const original = f.hooks.patch
  let fail = true
  f.hooks.patch = async (id, profile, backend) => {
    if (fail) throw new Error('Profile changed while a run was starting')
    return original(id, profile, backend)
  }
  await f.picker.select('profile:select:reviewer')
  expect(f.picker.helperText.value).toBe('Profile changed while a run was starting')
  expect(f.picker.saving.value).toBe(false)
  expect(f.open.value).toBe('profile')
  expect(f.closes()).toBe(0)
  expect(f.refreshes).toHaveLength(0)
  fail = false
  await f.picker.select('profile:select:reviewer')
  expect(f.writes).toHaveLength(2)
  expect(f.refreshes).toHaveLength(1)
  expect(f.open.value).toBeNull()
})

for (const change of ['session', 'backend'] as const) {
  test('a delayed load cannot overwrite the catalogue after ' + change + ' changes', async () => {
    const f = fixture()
    const original = f.hooks.sources
    const delayed = deferred<any>()
    let first = true
    f.hooks.sources = async (path, signal, backend) => {
      if (path === 'profiles' && first) {
        first = false
        return delayed.promise
      }
      return original(path, signal, backend)
    }
    const oldSources = await original('profiles', undefined, 1)
    await f.show()
    expect(f.picker.loading.value).toBe(true)
    const oldSignal = f.reads[0]!.signal
    if (change === 'session') f.switchSession('10')
    else f.backend.value = 2
    f.documents.workspace.shared = { title: 'New context role', mode: 'primary' }
    if (f.open.value !== 'profile') await f.show()
    else await f.picker.load()
    expect(f.item('profile:select:shared')?.label).toBe('New context role')
    expect(oldSignal?.aborted).toBe(true)
    delayed.resolve(oldSources)
    await settle()
    expect(f.item('profile:select:shared')?.label).toBe('New context role')
    expect(f.picker.loading.value).toBe(false)
    expect(f.open.value).toBe('profile')
  })

  for (const rejects of [false, true]) {
    test(
      'a stale ' + (rejects ? 'failed' : 'successful') + ' save ignores the new ' + change + ' context',
      async () => {
        const f = fixture()
        await f.show()
        const delayed = deferred<unknown>()
        f.hooks.patch = async () => delayed.promise
        const saving = f.picker.select('profile:select:reviewer')
        if (change === 'session') f.switchSession('10')
        else f.backend.value = 2
        if (f.open.value !== 'profile') await f.show()
        else await f.picker.load()
        const closesBeforeReply = f.closes()
        if (rejects) delayed.reject(new Error('Old backend write failed'))
        else delayed.resolve({})
        await saving
        expect(f.refreshes).toHaveLength(0)
        expect(f.open.value).toBe('profile')
        expect(f.closes()).toBe(closesBeforeReply)
        expect(f.picker.helperText.value).not.toContain('Old backend write failed')
        expect(f.chat.selectedSessionId).toBe(change === 'session' ? '10' : '9')
      },
    )
  }
}

test('disposing a picker prevents post-save refresh and removes settings subscription', async () => {
  const f = fixture()
  await f.show()
  const delayed = deferred<unknown>()
  f.hooks.patch = async () => delayed.promise
  const saving = f.picker.select('profile:select:reviewer')
  expect(f.listenerCount()).toBe(1)
  f.scope.stop()
  delayed.resolve({})
  await saving
  expect(f.refreshes).toHaveLength(0)
  expect(f.closes()).toBe(0)
  expect(f.listenerCount()).toBe(0)
})

for (const global of [false, true]) {
  test(
    'default settings open the ' + (global ? 'global' : 'workspace') + ' layer for the current pane session',
    async () => {
      const f = fixture({ global })
      await f.show()
      f.locallyBusy.value = true
      expect(f.item('profile:settings')?.disabled).toBe(false)
      await f.picker.select('profile:settings')
      expect(f.defaults).toEqual([{ scope: global ? 'global' : 'workspace', id: '9' }])
      expect(f.open.value).toBeNull()
      expect(f.writes).toHaveLength(0)
    },
  )
}
