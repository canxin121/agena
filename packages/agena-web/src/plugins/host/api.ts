import { ApiError, apiJson } from '@/lib/api'

import type { JsonValue as JsonLike } from '@/types/json'
import type { PluginActionResponse, PluginListItem, PluginListResponse, PluginManifestResponse } from '@/plugins/host/types'

// Agena plugin API (see crates/agena-api-server rest/plugins.rs):
//   GET  /api/v1/plugins                                   -> PluginStatus[]
//   GET  /api/v1/plugins/{plugin_id}                       -> { plugin: PluginInspect }
//   POST /api/v1/plugins/{plugin_id}/ui/actions/{action}   -> { plugin_id, action_id, action, result? }
// opencode's plugin manifest/action endpoints don't exist; the adapters below
// map between the opencode host shapes and these responses.

function asRecord(value: JsonLike | null | undefined): Record<string, JsonLike> | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  return value as Record<string, JsonLike>
}

export async function fetchPluginList(): Promise<PluginListResponse> {
  // Agena returns a bare PluginStatus[] (array) rather than an envelope.
  const payload = await apiJson<JsonLike>('/api/v1/plugins')
  const raw = Array.isArray(payload) ? payload : []
  const plugins: PluginListItem[] = []
  for (const entry of raw) {
    const record = asRecord(entry)
    if (!record) continue
    const id = String(record.plugin_id ?? '').trim()
    if (!id) continue
    const state = String(record.state ?? '').trim()
    plugins.push({
      id,
      spec: String(record.kind ?? '').trim(),
      status: state === 'running' ? 'ready' : 'resolve_error',
      capabilities: [],
      hasManifest: false,
    })
  }
  return { updatedAt: Date.now(), sourceSpecs: [], plugins }
}

export async function fetchPluginManifest(pluginId: string): Promise<PluginManifestResponse> {
  const id = String(pluginId || '').trim()
  if (!id) throw new Error('Plugin id is required')
  // Agena embeds the manifest inside the inspect payload ({ plugin: { status, manifest, ... } }).
  const payload = await apiJson<JsonLike>(`/api/v1/plugins/${encodeURIComponent(id)}`)
  const record = asRecord(payload)
  const plugin = asRecord(record?.plugin)
  const manifest = plugin?.manifest ?? null
  return { id, spec: '', manifestPath: '', manifest }
}

export async function invokePluginAction(
  pluginId: string,
  action: string,
  payload: JsonLike = null,
  context: JsonLike = null,
): Promise<PluginActionResponse> {
  const id = String(pluginId || '').trim()
  const actionName = String(action || '').trim()
  if (!id) throw new Error('Plugin id is required')
  if (!actionName) throw new Error('Plugin action is required')
  try {
    // Agena routes actions via ui/actions/{action_id}; the payload becomes the
    // request's input, with session_id pulled from the caller's context.
    const ctx = asRecord(context)
    const sessionId = ctx?.session_id
    const resp = await apiJson<JsonLike>(`/api/v1/plugins/${encodeURIComponent(id)}/ui/actions/${encodeURIComponent(actionName)}`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        input: payload !== null && payload !== undefined ? payload : null,
        session_id: typeof sessionId === 'number' ? sessionId : null,
      }),
    })
    const record = asRecord(resp)
    return { ok: true, data: (record?.result ?? null) as JsonLike }
  } catch (err) {
    if (err instanceof ApiError) {
      return { ok: false, error: { code: `http_${err.status ?? 'error'}`, message: err.message || err.bodyText || 'Plugin action failed' } }
    }
    return { ok: false, error: { code: 'error', message: err instanceof Error ? err.message : String(err) } }
  }
}
