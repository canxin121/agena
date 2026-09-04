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
 * Normalize the two current provider-model response envelopes: configured
 * models return an adapter array, while live listing returns `{ adapters }`.
 * Every adapter must carry its `models` array.
 */
export function normalizeProviderAdapterModels<TModel>(value: unknown): ProviderAdapterModelsRecord<TModel>[] {
  const response = record(value)
  const entries = Array.isArray(value) ? value : Array.isArray(response?.adapters) ? response.adapters : []

  return entries.flatMap((entry) => {
    const item = record(entry)
    if (!item) return []
    const adapterId = String(item.adapter_id || '').trim()
    if (!adapterId) return []
    if (!Array.isArray(item.models)) {
      throw new TypeError(`Provider adapter ${adapterId} is missing its models array`)
    }

    return [
      {
        ...item,
        adapter_id: adapterId,
        enabled: item.enabled === true,
        models: item.models as TModel[],
      } as ProviderAdapterModelsRecord<TModel>,
    ]
  })
}
