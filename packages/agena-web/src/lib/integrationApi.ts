import { apiJson } from './api'
import type {
  IntegrationsSnapshot,
  IntegrationDetail,
  IntegrationBinding,
  IntegrationControl,
  CreateIntegration,
  ChannelPolicyUpdate,
  PlatformInventory,
} from '@/generated/agenaState'
export type {
  IntegrationsSnapshot,
  IntegrationDetail,
  IntegrationBinding,
  IntegrationControl,
  CreateIntegration,
  PlatformInventory,
}
const base = (source: number) => `/api/v1/sessions/${source}/integrations`
async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
  const controller = new AbortController(),
    abort = () => controller.abort()
  if (init.signal?.aborted) controller.abort()
  else init.signal?.addEventListener('abort', abort, { once: true })
  const timeout = setTimeout(abort, 30000)
  try {
    return await apiJson<T>(path, { ...init, signal: controller.signal })
  } finally {
    clearTimeout(timeout)
    init.signal?.removeEventListener('abort', abort)
  }
}
const post = (body: unknown): RequestInit => ({
  method: 'POST',
  headers: { 'content-type': 'application/json' },
  body: JSON.stringify(body),
})
export const integrationApi = {
  access: (source: number, id: string, body: ChannelPolicyUpdate) =>
    request<IntegrationBinding>(`${base(source)}/${encodeURIComponent(id)}/access`, { ...post(body), method: 'PUT' }),
  list: (source: number, signal?: AbortSignal) => request<IntegrationsSnapshot>(base(source), { signal }),
  platforms: (source: number, signal?: AbortSignal) =>
    request<PlatformInventory>(`/api/v1/sessions/${source}/platforms`, { signal }),
  create: (source: number, body: CreateIntegration) => request<IntegrationBinding>(base(source), post(body)),
  detail: (source: number, id: string, signal?: AbortSignal) =>
    request<IntegrationDetail>(`${base(source)}/${encodeURIComponent(id)}`, { signal }),
  control: (source: number, id: string, body: IntegrationControl) =>
    request<IntegrationBinding>(`${base(source)}/${encodeURIComponent(id)}/control`, post(body)),
}
