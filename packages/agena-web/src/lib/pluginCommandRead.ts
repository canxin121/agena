import { apiJson } from './api'
import { captureResourceObservation } from './resourceSync'
import type { PluginCommandCatalogItem } from './pluginOperations'

type Surface = { catalog?: { commands?: PluginCommandCatalogItem[] } }
let cached: { scope: number; at: number; promise: Promise<Surface> } | undefined

/** Share catalog I/O; each pane still resolves its own locale and UI state. */
export async function readPluginCommandCatalog() {
  const scope = captureResourceObservation('sessions').scope
  if (cached?.scope === scope && Date.now() - cached.at < 30_000) return cached.promise
  const entry = {
    scope,
    at: Date.now(),
    promise: apiJson<Surface>('/api/v1/plugins/surface', { signal: AbortSignal.timeout(15_000) }),
  }
  cached = entry
  try {
    return await entry.promise
  } catch (error) {
    if (cached === entry) cached = undefined
    throw error
  }
}
