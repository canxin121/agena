type UnknownRecord = Record<string, unknown>

export type ProviderAdapterModelsRecord<TModel> = {
  adapter_id: string
  enabled: boolean
  resolved_base_url?: string | null
  models: TModel[]
  failure?: Record<string, any> | null
}

function record(value: unknown): UnknownRecord | null {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as UnknownRecord) : null
}

/**
 * Normalize both configured-model array responses and live-listing response
 * envelopes. Older Agena servers omitted `models` when an adapter had no
 * configured routes, so browser consumers must treat a missing field as an
 * empty list rather than calling Array methods on `undefined`.
 */
export function normalizeProviderAdapterModels<TModel>(value: unknown): ProviderAdapterModelsRecord<TModel>[] {
  const response = record(value)
  const entries = Array.isArray(value) ? value : Array.isArray(response?.adapters) ? response.adapters : []

  return entries.flatMap((entry) => {
    const item = record(entry)
    if (!item) return []
    const adapterId = String(item.adapter_id || '').trim()
    if (!adapterId) return []

    return [
      {
        ...item,
        adapter_id: adapterId,
        enabled: item.enabled === true,
        models: Array.isArray(item.models) ? (item.models as TModel[]) : [],
      } as ProviderAdapterModelsRecord<TModel>,
    ]
  })
}
