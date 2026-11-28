import { apiJson } from './api'
import { captureResourceObservation } from './resourceSync'
import type { PluginCommandCatalogItem } from './pluginOperations'

type Surface = { catalog?: { commands?: PluginCommandCatalogItem[] } }
let cached: { key: string; at: number; promise: Promise<Surface> } | undefined

/** Share catalog I/O; each pane still resolves its own locale and UI state. */
export async function readPluginCommandCatalog(sessionId?: string | null) {
  const scope = captureResourceObservation('sessions').scope
  const id = Number(sessionId)
  const session = Number.isSafeInteger(id) && id > 0 ? String(id) : ''
  const key = `${scope}:${session}`
  if (cached?.key === key && Date.now() - cached.at < 30_000) return cached.promise
  const path = session ? `/api/v1/plugins/surface?session_id=${session}` : '/api/v1/plugins/surface'
  const entry = {
    key,
    at: Date.now(),
    promise: apiJson<Surface>(path, { signal: AbortSignal.timeout(15_000) }),
  }
  cached = entry
  try {
    return await entry.promise
  } catch (error) {
    if (cached === entry) cached = undefined
    throw error
  } finally {
    if (session && cached === entry) cached = undefined
  }
}
