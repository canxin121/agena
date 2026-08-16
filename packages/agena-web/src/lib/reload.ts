import { apiJson } from './api'

export type RuntimeReloadTaskResponse = {
  started: boolean
  task: {
    id: string
    kind: string
    status: string
  }
}

type RuntimeGeneration = { generation?: number }

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => globalThis.setTimeout(resolve, ms))
}

export async function reloadAgenaRuntime(): Promise<RuntimeReloadTaskResponse> {
  const before = await apiJson<RuntimeGeneration>('/api/v1/runtime').catch(() => null)
  const response = await apiJson<RuntimeReloadTaskResponse>('/api/v1/runtime/reload', { method: 'POST' })
  const previousGeneration = typeof before?.generation === 'number' ? before.generation : null

  // Reload runs as a background task. Wait briefly for the new generation so
  // callers do not immediately reload the browser against the old runtime.
  if (previousGeneration !== null) {
    for (let attempt = 0; attempt < 20; attempt += 1) {
      await delay(250)
      const current = await apiJson<RuntimeGeneration>('/api/v1/runtime').catch(() => null)
      if (typeof current?.generation === 'number' && current.generation > previousGeneration) break
    }
  }
  return response
}
