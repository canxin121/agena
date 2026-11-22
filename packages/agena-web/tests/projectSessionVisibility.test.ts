import { afterEach, beforeEach, describe, expect, mock, spyOn, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia, type Pinia } from 'pinia'

import { useChatStoreUnscoped } from '../src/stores/chat'
import * as chatApi from '../src/stores/chat/api'
import { fetchAgenaWorkspaces, useDirectorySessionStore } from '../src/stores/directorySessionStore'
import { noteResourceVersion } from '../src/lib/resourceSync'
import type { SessionOwnerSurface } from '../src/types/chat'

const BOT_ID = '8f2c0000-0000-4000-8000-000000000001'
const surfaces = ['local', 'direct', 'group', 'broadcast'] as const
const browserGlobals = ['window', 'document', 'localStorage', 'sessionStorage'] as const
const originalGlobals = new Map(browserGlobals.map((name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)]))
const originalFetch = globalThis.fetch
let pinia: Pinia

function memoryStorage() {
  const values = new Map<string, string>()
  return {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => values.set(key, value),
    removeItem: (key: string) => values.delete(key),
  }
}

beforeEach(() => {
  globalThis.fetch = (async (url) => {
    throw new Error(`Unexpected fixture request: ${String(url)}`)
  }) as typeof fetch
  const values = {
    window: Object.assign(new EventTarget(), {
      location: {
        origin: 'https://agena.test',
        href: 'https://agena.test/chat',
        pathname: '/chat',
        search: '',
        hash: '',
      },
      setTimeout,
      clearTimeout,
    }),
    document: undefined,
    localStorage: memoryStorage(),
    sessionStorage: memoryStorage(),
  }
  for (const name of browserGlobals)
    Object.defineProperty(globalThis, name, { configurable: true, value: values[name] })
  pinia = createPinia()
  setActivePinia(pinia)
})

afterEach(() => {
  disposePinia(pinia)
  setActivePinia(undefined)
  mock.restore()
  globalThis.fetch = originalFetch
  for (const name of browserGlobals) {
    const descriptor = originalGlobals.get(name)
    if (descriptor) Object.defineProperty(globalThis, name, descriptor)
    else Reflect.deleteProperty(globalThis, name)
  }
})

test('workspace pages use matching cursors and recover when the catalog changes', async () => {
  const catalog = 'workspaces:catalog'
  let revision = 'workspace-pages:1'
  noteResourceVersion(catalog, revision)
  const requests: URLSearchParams[] = []
  globalThis.fetch = (async (input) => {
    const url = new URL(String(input), 'https://agena.test')
    expect(url.pathname).toBe('/api/v1/workspaces')
    const params = url.searchParams
    requests.push(params)
    const offset = params.get('offset')
    const cursor = params.get('cursor')
    const id = cursor ? Number(cursor.slice('after-'.length)) : Number(offset) + 1
    return Response.json(
      { items: [{ id, path: `/project/${id}` }], page: { has_more: true, next_cursor: `after-${id + 1}` } },
      { headers: { 'x-resource-revision': revision } },
    )
  }) as typeof fetch

  const search = 'page-anchor'
  await fetchAgenaWorkspaces({ limit: 1, page: 0, search })
  await fetchAgenaWorkspaces({ limit: 1, page: 1, search })
  await fetchAgenaWorkspaces({ limit: 1, page: 1, search: 'other-search' })
  await fetchAgenaWorkspaces({ limit: 2, page: 1, search })
  await fetchAgenaWorkspaces({ limit: 1, page: 19, search })
  revision = 'workspace-pages:2'
  const page = await fetchAgenaWorkspaces({ limit: 1, page: 2, search })
  expect(page.entries.map((entry) => entry.id)).toEqual(['3'])
  expect(requests.map((params) => [params.get('cursor'), params.get('offset')])).toEqual([
    [null, '0'],
    ['after-2', null],
    [null, '1'],
    [null, '2'],
    [null, '19'],
    ['after-3', null],
    [null, '2'],
  ])
})

function session(id: number, surface?: SessionOwnerSurface, parentId?: number) {
  return {
    id,
    workspace_id: 7,
    title: surface ? `${surface} Bot conversation` : 'Project conversation',
    cwd: '/fixture/project',
    owner_bot_id: surface ? BOT_ID : null,
    owner_surface: surface ?? null,
    parent_id: parentId ?? null,
    root_id: parentId ?? id,
    depth: parentId ? 1 : 0,
    child_session_count: 0,
    relation_kind: parentId ? 'fork' : 'root',
    lifecycle_state: 'ready',
    state: { kind: 'ready', data: {} },
    version: 1,
    pinned: true,
    favorite: true,
    message_count: 1,
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
  }
}

function execution(value: ReturnType<typeof session>): chatApi.AgenaExecutionState {
  return {
    session: value as chatApi.AgenaExecutionState['session'],
    latest_event_seq: 1,
    parts: [],
    part_page: null,
    execution: { agent_status: 'completed' },
    queued_messages: [],
    queue_paused: false,
    background_activities: [],
  }
}

function collapsedProject() {
  const sidebar = useDirectorySessionStore(pinia)
  sidebar.uiPrefs.collapsedDirectoryIds = ['7']
  sidebar.uiPrefs.sessionRootPageByDirectoryId = { '7': 3 }
  sidebar.uiPrefs.expandedParentSessionIds = []
  return sidebar
}

describe('project sidebar ownership', () => {
  test.each(surfaces)('%s Bot forks keep project expansion and paging unchanged', async (surface) => {
    const sidebar = collapsedProject()
    const before = JSON.parse(JSON.stringify(sidebar.uiPrefs))
    const chat = useChatStoreUnscoped(pinia)
    spyOn(chatApi, 'forkSession').mockResolvedValue(execution(session(100, surface, 42)))

    const fork = await chat.forkSession('42')

    expect(fork?.id).toBe('100')
    expect(sidebar.uiPrefs).toEqual(before)
    expect(chat.getSessionById('100')?.owner_bot_id).toBe(BOT_ID)
    expect(chat.getSessionById('100')?.workspace_id).toBe(7)
  })

  test('reverting a Bot conversation preserves project chrome and retains the Bot session', async () => {
    const sidebar = collapsedProject()
    const before = JSON.parse(JSON.stringify(sidebar.uiPrefs))
    const chat = useChatStoreUnscoped(pinia)
    spyOn(chatApi, 'listSessionMessages').mockResolvedValue([
      { info: { id: '10', sessionID: '42', role: 'user', runState: 'completed' }, parts: [] },
    ])
    spyOn(chatApi, 'rewindSession').mockResolvedValue(execution(session(101, 'local', 42)))

    const rewound = await chat.revertToMessage('42', '10')

    expect(rewound?.session.id).toBe('101')
    expect(sidebar.uiPrefs).toEqual(before)
    expect(chat.getSessionById('101')?.owner_bot_id).toBe(BOT_ID)
  })

  test('ordinary project forks still reveal their parent and workspace', async () => {
    const sidebar = collapsedProject()
    const chat = useChatStoreUnscoped(pinia)
    spyOn(chatApi, 'forkSession').mockResolvedValue(execution(session(102, undefined, 42)))

    await chat.forkSession('42')

    expect(sidebar.uiPrefs.collapsedDirectoryIds).toEqual([])
    expect(sidebar.uiPrefs.expandedParentSessionIds).toEqual(['42'])
  })

  test('retained Bot snapshots cannot populate project rows, footers or session state', async () => {
    const project = session(10)
    const bots = surfaces.map((surface, index) => ({
      ...session(20 + index, surface),
      state: { kind: 'running', data: {} },
    }))
    globalThis.fetch = (async (input) => {
      const url = new URL(String(input), 'https://agena.test')
      if (url.pathname === '/api/v1/changes/revisions') {
        const resources = JSON.parse(url.searchParams.get('resources')!) as string[]
        return Response.json(Object.fromEntries(resources.map((key) => [key, 'project-visibility:1'])))
      }
      if (url.pathname === '/api/v1/workspaces')
        return Response.json({ items: [{ id: 7, path: '/fixture/project' }], page: { has_more: false } })
      if (url.pathname === '/api/v1/workspaces/session-stats')
        return Response.json({
          items: [
            {
              workspace_id: 7,
              revision: 'project-visibility:1',
              stats: { total: 1, roots: 1, pinned: 1, running: 0, attention: 0 },
            },
          ],
        })
      if (url.pathname === '/api/v1/sessions') {
        const running = url.searchParams.get('bucket') === 'running'
        return Response.json({
          items: [...(running ? [] : [project]), ...bots],
          total: running ? 0 : 1,
          page: { has_more: false },
        })
      }
      throw new Error(`Unexpected fixture request: ${url}`)
    }) as typeof fetch
    const chat = useChatStoreUnscoped(pinia)
    chat.cacheSessions(bots.map((item) => chatApi.toSession(item)!))
    const sidebar = useDirectorySessionStore(pinia)
    sidebar.uiPrefs.pinnedSessionsOpen = true
    sidebar.uiPrefs.favoriteSessionsOpen = true
    sidebar.uiPrefs.recentSessionsOpen = true
    sidebar.uiPrefs.runningSessionsOpen = true

    expect(await sidebar.revalidateFromApi()).toBe(true)

    const directory = sidebar.directorySidebarById['7']!
    expect(directory.recentRows.map((row) => row.id)).toEqual(['10'])
    expect(directory.pinnedRows.map((row) => row.id)).toEqual(['10'])
    expect(directory.recentRootIds).toEqual(['10'])
    expect(directory.hasRunningSessions).toBe(false)
    for (const footer of [sidebar.pinnedFooterView, sidebar.favoriteFooterView, sidebar.recentFooterView])
      expect(footer.rows.map((row) => row.id)).toEqual(['10'])
    expect(sidebar.runningFooterView.rows).toEqual([])
    expect(Object.keys(sidebar.stateBySessionId)).toEqual(['10'])
    expect(sidebar.uiPrefs.pinnedSessionIds).toEqual(['10'])
    for (const bot of bots) expect(chat.getSessionById(String(bot.id))?.owner_bot_id).toBe(BOT_ID)
  })
})
