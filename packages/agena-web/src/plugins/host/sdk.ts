import { invokePluginAction } from '@/plugins/host/api'
import type { JsonValue as JsonLike } from '@/types/json'
import { connectSse } from '@/lib/sse'

export type PluginHostEvent = {
  type: string
  data: JsonLike
  lastEventId?: string
}

export async function invokeHostPluginAction(
  pluginId: string,
  action: string,
  payload: JsonLike = null,
  context: JsonLike = null,
): Promise<JsonLike> {
  const resp = await invokePluginAction(pluginId, action, payload, context)
  if (!resp.ok) {
    throw new Error(resp.error?.message || `Plugin action failed: ${action}`)
  }
  return resp.data ?? null
}

export function subscribeHostPluginEvents(
  pluginId: string,
  handlers: {
    onEvent?: (evt: PluginHostEvent) => void
    onError?: (err: Event) => void
  },
): () => void {
  const id = String(pluginId || '').trim()
  if (!id) return () => {}

  const client = connectSse({
    // Agena has no per-plugin event feed; subscribe to the shared change stream
    // and filter to this plugin's events on receipt.
    endpoint: `/api/v1/changes/stream?scope_kind=global`,
    debugLabel: `sse:plugin:${id}`,
    onEvent: (evt) => {
      const type = String(evt?.type || '').trim()
      if (!type) return

      // plugin_runtime now wraps event payloads as { type, data }.
      const raw = (evt as unknown as { data?: unknown }).data
      handlers.onEvent?.({
        type,
        data: raw !== undefined ? (raw as JsonLike) : (evt as unknown as JsonLike),
        lastEventId:
          typeof (evt as unknown as { lastEventId?: unknown }).lastEventId === 'string'
            ? String((evt as unknown as { lastEventId?: string }).lastEventId || '')
            : undefined,
      })
    },
    onError: () => {
      try {
        handlers.onError?.(new Event('error'))
      } catch {
        // ignore
      }
    },
  })

  return () => {
    client.close()
  }
}
