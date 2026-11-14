import { afterEach, beforeEach, describe, expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia, type Pinia } from 'pinia'
import { bindBotSession, getBotSession } from '../src/lib/botSession'
import { MAIN_TABS, mainTabFromPath, mainTabPath, WORKSPACE_MAIN_TABS } from '../src/app/navigation/mainTabs'
import { useUiStore } from '../src/stores/ui'
import { clientCommandFromCatalog } from '../src/pages/chat/chatCommandsCatalog'
import type { PluginCommandCatalogItem } from '../src/lib/pluginOperations'

const originalFetch = globalThis.fetch
let pinia: Pinia
beforeEach(() => {
  pinia = createPinia()
})
afterEach(() => {
  globalThis.fetch = originalFetch
  disposePinia(pinia)
  setActivePinia(undefined)
})
function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => {
    resolve = done
  })
  return { promise, resolve }
}

describe('canonical Bot conversation', () => {
  test('resolves exclusively through the bodyless canonical endpoint and validates its numeric ID', async () => {
    const calls: Array<{ url: string; method?: string; body?: BodyInit | null }> = []
    let id: unknown = 42
    globalThis.fetch = (async (input, init) => {
      calls.push({ url: String(input), method: init?.method, body: init?.body })
      return Response.json({ session_id: id, bot_id: '11111111-1111-4111-8111-111111111111', name: 'Atlas' })
    }) as typeof fetch
    expect(await getBotSession()).toBe('42')
    expect(await getBotSession()).toBe('42')
    expect(
      calls.every((call) => call.url.endsWith('/api/v1/bot/session') && call.method === 'POST' && !call.body),
    ).toBe(true)
    for (id of [null, '42', 0, -1, 1.5, Number.MAX_SAFE_INTEGER + 1]) {
      await expect(getBotSession()).rejects.toThrow('Invalid Bot session response')
    }
  })

  test('leaving during canonical lookup never hydrates or binds the old pane', async () => {
    const response = deferred<string>()
    let current = true
    const events: string[] = []
    const pending = bindBotSession({
      resolve: () => response.promise,
      isCurrent: () => current,
      hydrate: async (id) => {
        events.push(`hydrate:${id}`)
      },
      bind: async (id) => {
        events.push(`bind:${id}`)
      },
    })
    current = false
    response.resolve('42')
    expect(await pending).toBeNull()
    expect(events).toEqual([])
  })

  test('leaving during hydration never replaces the project session route', async () => {
    const hydrated = deferred<void>()
    let current = true
    let directoryOwner: (() => boolean) | undefined
    const bound: string[] = []
    const pending = bindBotSession({
      resolve: async () => '42',
      isCurrent: () => current,
      hydrate: (_id, ownsRoute) => {
        directoryOwner = ownsRoute
        return hydrated.promise
      },
      bind: async (id) => {
        bound.push(id)
      },
    })
    await Promise.resolve()
    current = false
    hydrated.resolve()
    expect(await pending).toBeNull()
    expect(directoryOwner?.()).toBe(false)
    expect(bound).toEqual([])
  })

  test('a newer load wins even if an aborted request still resolves', async () => {
    const oldResponse = deferred<string>()
    let generation = 1
    const bound: string[] = []
    const load = (owner: number, resolve: () => Promise<string>) =>
      bindBotSession({
        resolve,
        isCurrent: () => generation === owner,
        hydrate: async () => {},
        bind: async (id) => {
          bound.push(id)
        },
      })
    const old = load(1, () => oldResponse.promise)
    generation = 2
    expect(await load(2, async () => '42')).toBe('42')
    oldResponse.resolve('77')
    expect(await old).toBeNull()
    expect(bound).toEqual(['42'])
  })

  test('lookup and hydration failures can be retried without binding a different conversation', async () => {
    let attempts = 0
    const bound: string[] = []
    const load = () =>
      bindBotSession({
        resolve: async () => {
          if (++attempts === 1) throw new Error('offline')
          return '42'
        },
        isCurrent: () => true,
        hydrate: async () => {},
        bind: async (id) => {
          bound.push(id)
        },
      })
    await expect(load()).rejects.toThrow('offline')
    expect(bound).toEqual([])
    expect(await load()).toBe('42')
    await expect(
      bindBotSession({
        resolve: async () => '42',
        isCurrent: () => true,
        hydrate: async () => {
          throw new Error('unavailable')
        },
        bind: async (id) => {
          bound.push(id)
        },
      }),
    ).rejects.toThrow('unavailable')
    expect(bound).toEqual(['42'])
  })

  test('Bot remains a primary desktop/mobile destination and Task is absent', () => {
    expect(MAIN_TABS.map((tab) => tab.id)).toEqual(['bot', 'chat', 'files', 'preview', 'terminal', 'git', 'plugins'])
    expect(mainTabFromPath('/bot')).toBe('bot')
    expect(mainTabPath('bot')).toBe('/bot')
    expect(WORKSPACE_MAIN_TABS.some((tab) => String(tab.id) === 'tasks')).toBe(false)
  })

  test('clean /bot entry reuses only its unresolved tab and named entries match botId', () => {
    setActivePinia(pinia)
    const ui = useUiStore()
    const first = ui.openWorkspaceWindow('bot', { path: '/bot', query: {} })
    expect(first).toBeTruthy()
    ui.setWorkspaceWindowRouteQuery(first, { sessionId: '42' })
    const second = ui.openWorkspaceWindow('bot', { path: '/bot', query: {} })
    expect(second).toBe(first)
    expect(ui.workspaceWindows).toHaveLength(1)
    expect(ui.workspaceWindows[0]?.mainTab).toBe('bot')
  })

  test('accepts the shared bot command action independently of its slash spelling', () => {
    const command = clientCommandFromCatalog({
      id: 'bot',
      slash: '/operator',
      aliases: ['/bot'],
      target: { kind: 'client', action: 'bot' },
      docs: {},
    } as PluginCommandCatalogItem)
    expect(command?.action).toBe('bot')
    expect(command?.name).toBe('operator')
    expect(command?.aliases).toEqual(['bot'])
  })
})
