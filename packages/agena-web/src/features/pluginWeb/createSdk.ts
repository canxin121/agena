import type {
  AgenaPluginUI,
  PluginCommandResult,
  PluginPageCatalogItem,
  PluginSettingsState,
  PluginSettingsUpdateResponse,
  PluginUIEnvironment,
} from './sdk'

export type PluginUIRequest = <T>(path: string, init?: RequestInit) => Promise<T>

export type PluginUIHost = {
  page: PluginPageCatalogItem
  signal: AbortSignal
  sessionId: number | null
  path: string
  request: PluginUIRequest
  environment: () => PluginUIEnvironment
  subscribeEnvironment: (listener: (environment: PluginUIEnvironment) => void) => () => void
  openPage: (pageId: string) => Promise<void>
  setTitle: (title: string) => void
  notify: (message: string, tone: 'success' | 'error' | 'info') => void
  refresh: () => void
  messages: { inactive: string; unavailable: string; sessionRequired: string }
}

/** Build a fresh API for every mount; callbacks never look up a global selection. */
export function createPluginUI(host: PluginUIHost): AgenaPluginUI {
  const { page, signal } = host
  const commands = new Set(page.commands)
  const base = `/api/v1/plugins/${encodeURIComponent(page.plugin_id)}/web/${encodeURIComponent(page.page_id)}`
  const active = () => {
    if (signal.aborted) throw new DOMException(host.messages.inactive, 'AbortError')
  }
  const available = (allowed: boolean) => {
    active()
    if (!allowed) throw new Error(host.messages.unavailable)
  }
  async function request<T>(path: string, init?: RequestInit): Promise<T> {
    active()
    try {
      const value = await host.request<T>(path, { ...init, signal })
      active()
      return value
    } catch (error) {
      if (!signal.aborted && error && typeof error === 'object' && 'status' in error) {
        if (error.status === 404 || error.status === 409) host.refresh()
      }
      throw error
    }
  }
  const sdk: AgenaPluginUI = {
    pluginId: page.plugin_id,
    pageId: page.page_id,
    signal,
    context: Object.freeze({ sessionId: host.sessionId, path: host.path }),
    get environment() {
      return host.environment()
    },
    subscribeEnvironment(listener) {
      active()
      let unsubscribe = () => {}
      const stop = () => {
        unsubscribe()
        signal.removeEventListener('abort', stop)
      }
      unsubscribe = host.subscribeEnvironment((environment) => {
        if (!signal.aborted) listener(environment)
      })
      signal.addEventListener('abort', stop, { once: true })
      if (signal.aborted) stop()
      return stop
    },
    commands: Object.freeze({
      async invoke(commandId, input = {}) {
        available(commands.has(commandId))
        if (page.scope === 'session' && host.sessionId === null) throw new Error(host.messages.sessionRequired)
        const response = await request<{ result: PluginCommandResult }>(
          `${base}/commands/${encodeURIComponent(commandId)}`,
          {
            method: 'POST',
            headers: { 'content-type': 'application/json' },
            body: JSON.stringify({ generation: page.generation, input, session_id: host.sessionId }),
          },
        )
        return response.result
      },
    }),
    settings: Object.freeze({
      read() {
        available(page.settings)
        return request<PluginSettingsState>(`${base}/settings?generation=${encodeURIComponent(page.generation)}`)
      },
      async update(value) {
        available(page.settings)
        const response = await request<PluginSettingsUpdateResponse>(`${base}/settings`, {
          method: 'PUT',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({ generation: page.generation, value }),
        })
        if (response.reload_required) host.refresh()
        return response
      },
    }),
    navigation: Object.freeze({
      async openPage(pageId) {
        active()
        await host.openPage(pageId)
      },
    }),
    ui: Object.freeze({
      setTitle(title) {
        active()
        host.setTitle(String(title).trim().slice(0, 200))
      },
      notify(message, tone = 'info') {
        active()
        if (!['success', 'error', 'info'].includes(tone)) throw new Error(host.messages.unavailable)
        host.notify(String(message).trim().slice(0, 2000), tone)
      },
    }),
  }
  return Object.freeze(sdk)
}
