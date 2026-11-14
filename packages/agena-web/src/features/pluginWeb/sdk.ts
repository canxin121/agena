import type { PluginCommandResult, PluginSettingsState, PluginSettingsUpdateResponse } from '../../lib/pluginOperations'
import type { JsonValue } from '../../types/json'

export type { PluginCommandResult, PluginSettingsState, PluginSettingsUpdateResponse, JsonValue }

/** Version 1 of the browser entry exported by an installed plugin. */
export interface PluginPageModule {
  mount(container: HTMLElement, sdk: AgenaPluginUI): PluginPageHandle | Promise<PluginPageHandle>
}

/** Release listeners, application roots, and other resources owned by this mount. */
export interface PluginPageHandle {
  dispose(): void | Promise<void>
}

export type PluginUIEnvironment = Readonly<{
  locale: string
  theme: 'light' | 'dark'
}>

export type PluginPageCatalogItem = {
  plugin_id: string
  page_id: string
  title: string
  api_version: number
  scope: 'instance' | 'session'
  entry_url: string
  style_urls: string[]
  commands: string[]
  settings: boolean
  revision: string
  generation: string
}

/** Host services bound to one plugin page and its current runtime generation.
 * Native modules share the host's browser privileges. This SDK is a versioned
 * application API; it does not sandbox the module.
 */
export interface AgenaPluginUI {
  readonly pluginId: string
  readonly pageId: string
  readonly signal: AbortSignal
  readonly context: Readonly<{ sessionId: number | null; path: string }>
  readonly environment: PluginUIEnvironment
  /** Immediately receive the current environment, then changes until disposed. */
  subscribeEnvironment(listener: (environment: PluginUIEnvironment) => void): () => void
  readonly commands: {
    invoke(commandId: string, input?: JsonValue): Promise<PluginCommandResult>
  }
  readonly settings: {
    read(): Promise<PluginSettingsState>
    update(value: JsonValue): Promise<PluginSettingsUpdateResponse>
  }
  readonly navigation: {
    openPage(pageId: string): Promise<void>
  }
  readonly ui: {
    setTitle(title: string): void
    notify(message: string, tone?: 'success' | 'error' | 'info'): void
  }
}
