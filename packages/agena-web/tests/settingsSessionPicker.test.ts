import { describe, expect, test } from 'bun:test'
import { createSettingsSessionCatalog, settingsSessionGroups } from '../src/components/settings/settingsSessionCatalog'
import type { Session } from '../src/types/chat'
import type { BotProfile } from '../src/lib/bots'

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (cause: unknown) => void
  const promise = new Promise<T>((yes, no) => {
    resolve = yes
    reject = no
  })
  return { promise, resolve, reject }
}
function session(id: string, title = 'Conversation ' + id): Session {
  return { id, title, workspace_id: 3 }
}
function bot(id: string, local: number | null, archived = false): BotProfile {
  return {
    id,
    name: id,
    local_session_id: local,
    archived,
    persona: { instructions: '' },
    created_at_ms: 1,
    updated_at_ms: 1,
    revision: 1,
  }
}
function harness(overrides: Partial<Parameters<typeof createSettingsSessionCatalog>[0]> = {}) {
  const cached: Session[][] = []
  const catalog = createSettingsSessionCatalog({
    listSessions: async () => ({ sessions: [], hasMore: false, total: 0 }),
    listBots: async () => ({ items: [] }),
    getSession: async (id) => session(id),
    cacheMetadata: (rows) => cached.push(rows),
    ...overrides,
  })
  return { catalog, cached }
}
const labels = { projects: 'Projects', bots: 'AI Bots', selected: 'Selected', session: (id: string) => 'Session ' + id }

describe('settings conversation catalog', () => {
  test('pages the server catalog beyond recents and preserves project ownership boundaries', async () => {
    const requests: any[] = []
    const { catalog, cached } = harness({
      listSessions: async (options) => {
        requests.push(options)
        return {
          sessions: [
            session('164'),
            { ...session('42'), owner_bot_id: 'atlas' },
            { ...session('165'), relation_kind: 'subagent' },
          ],
          total: 70,
          hasMore: false,
        }
      },
    })
    catalog.query.value = ' older '
    await catalog.loadPage(3)
    expect(requests[0]).toMatchObject({
      limit: 30,
      offset: 60,
      search: 'older',
      excludeSubagents: true,
      includeTotal: true,
      force: true,
    })
    expect(catalog.sessions.value.map((row) => row.id)).toEqual(['164'])
    expect(catalog.pageCount.value).toBe(3)
    expect(cached).toEqual([])
    catalog.dispose()
  })

  test('an obsolete page cannot replace the latest page even if abort is ignored', async () => {
    const first = deferred<{ sessions: Session[]; total: number; hasMore: boolean }>()
    const second = deferred<{ sessions: Session[]; total: number; hasMore: boolean }>()
    const signals: AbortSignal[] = []
    const { catalog } = harness({
      listSessions: (options) => {
        signals.push(options.signal)
        return signals.length === 1 ? first.promise : second.promise
      },
    })
    const old = catalog.loadPage(1)
    const current = catalog.loadPage(2)
    expect(signals[0]!.aborted).toBe(true)
    second.resolve({ sessions: [session('64')], total: 70, hasMore: true })
    await current
    first.resolve({ sessions: [session('7')], total: 30, hasMore: false })
    await old
    expect(catalog.page.value).toBe(2)
    expect(catalog.total.value).toBe(70)
    expect(catalog.sessions.value.map((row) => row.id)).toEqual(['64'])
    catalog.dispose()
  })

  test('search resets to page one and cancels stale results before its debounce fires', async () => {
    const first = deferred<{ sessions: Session[]; total: number; hasMore: boolean }>()
    const requests: any[] = []
    const { catalog } = harness({
      listSessions: async (options) => {
        requests.push(options)
        return requests.length === 1 ? first.promise : { sessions: [session('164')], total: 1, hasMore: false }
      },
    })
    const old = catalog.loadPage(3)
    catalog.search(' Older ')
    expect(requests[0].signal.aborted).toBe(true)
    expect(catalog.page.value).toBe(1)
    expect(catalog.sessions.value).toEqual([])
    await catalog.loadPage()
    expect(requests[1]).toMatchObject({ search: 'Older', offset: 0 })
    first.resolve({ sessions: [session('7')], total: 70, hasMore: false })
    await old
    expect(catalog.sessions.value.map((row) => row.id)).toEqual(['164'])
    expect(catalog.loading.value).toBe(false)
    catalog.dispose()
  })

  test('selected metadata is independent of browsing and late targets cannot enter the cache', async () => {
    const first = deferred<Session>()
    const second = deferred<Session>()
    const requests: Array<{ id: string; signal: AbortSignal }> = []
    const { catalog, cached } = harness({
      getSession: (id, signal) => {
        requests.push({ id, signal })
        return requests.length === 1 ? first.promise : second.promise
      },
    })
    const old = catalog.loadSelected('007')
    const current = catalog.loadSelected('42')
    expect(requests.map((item) => item.id)).toEqual(['7', '42'])
    expect(requests[0]!.signal.aborted).toBe(true)
    second.resolve({ id: '42', title: 'Bot remote conversation', owner_bot_id: 'atlas', owner_surface: 'group' })
    await current
    await catalog.loadPage(2)
    first.resolve(session('7'))
    await old
    expect(catalog.selected.value?.id).toBe('42')
    expect(cached.map((rows) => rows.map((row) => row.id))).toEqual([['42']])
    catalog.dispose()
  })

  test('a mismatched selected-session response stays out of the shared cache and supports retry', async () => {
    let count = 0
    const { catalog, cached } = harness({ getSession: async () => session(++count === 1 ? '8' : '7') })
    await catalog.loadSelected('7')
    expect(catalog.selected.value).toBeNull()
    expect(catalog.selectedError.value).toBeTruthy()
    expect(cached).toEqual([])
    await catalog.loadSelected('7')
    expect(catalog.selected.value?.id).toBe('7')
    expect(catalog.selectedError.value).toBe('')
    expect(cached).toHaveLength(1)
    catalog.dispose()
  })

  test('project request failures are visible while existing Bot choices remain usable', async () => {
    let fail = true
    const { catalog } = harness({
      listSessions: async () => {
        if (fail) throw new Error('Catalog unavailable')
        return { sessions: [session('7')], total: 1, hasMore: false }
      },
      listBots: async () => ({
        items: [bot('Atlas', 42), bot('No conversation yet', null), bot('Archived', 77, true)],
      }),
    })
    await Promise.all([catalog.loadPage(), catalog.loadBots()])
    expect(catalog.error.value).toBe('Catalog unavailable')
    expect(catalog.bots.value.map((entry) => entry.name)).toEqual(['Atlas'])
    fail = false
    await catalog.loadPage()
    expect(catalog.error.value).toBe('')
    expect(catalog.sessions.value.map((entry) => entry.id)).toEqual(['7'])
    catalog.dispose()
  })

  test('unmount cancels every resource and ignores responses that complete later', async () => {
    const pending = deferred<Session>()
    const requests: AbortSignal[] = []
    const { catalog, cached } = harness({
      getSession: (_id, signal) => {
        requests.push(signal)
        return pending.promise
      },
    })
    const work = catalog.loadSelected('7')
    catalog.dispose()
    expect(requests[0]!.aborted).toBe(true)
    pending.resolve(session('7'))
    await work
    expect(cached).toEqual([])
    expect(catalog.selected.value).toBeNull()
  })
})

describe('conversation picker groups', () => {
  test('keeps an off-page remote Bot target and separates existing canonical Bots from projects', () => {
    const groups = settingsSessionGroups({
      sessions: [session('7'), session('7'), { ...session('42'), owner_bot_id: 'Atlas' }],
      bots: [bot('Atlas', 42), bot('New Bot', null)],
      selectedId: '99',
      selected: { id: '99', title: 'Remote Bot room', owner_bot_id: 'Atlas', owner_surface: 'group' },
      query: '',
      labels,
    })
    expect(groups.map((group) => group.id)).toEqual([
      'selected-conversation',
      'project-conversations',
      'bot-conversations',
    ])
    expect(groups.map((group) => group.items.map((item) => item.id))).toEqual([['99'], ['7'], ['42']])
    expect(groups[0]!.items[0]!.label).toBe('Remote Bot room')
    expect(groups[2]!.items[0]!.label).toBe('Atlas')
  })

  test('Bot search matches names and IDs while selected context remains available', () => {
    const groups = settingsSessionGroups({
      sessions: [session('164', 'Older project conversation')],
      bots: [bot('Atlas', 42), bot('Muse', 43)],
      selectedId: '007',
      selected: session('7'),
      query: 'muse',
      labels,
    })
    expect(groups.flatMap((group) => group.items.map((item) => item.id))).toEqual(['7', '164', '43'])
    const exact = settingsSessionGroups({
      sessions: [],
      bots: [bot('Atlas', 42)],
      selectedId: '42',
      selected: null,
      query: '42',
      labels,
    })
    expect(exact).toHaveLength(1)
    expect(exact[0]!.id).toBe('bot-conversations')
    expect(exact[0]!.items[0]!.id).toBe('42')
  })
})
