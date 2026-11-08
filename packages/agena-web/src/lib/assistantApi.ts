import { apiJson } from './api'
import type {
  AssistantSnapshot,
  AssistantProfile,
  AssistantUpdate,
  AssistantMemory,
  AssistantMemoryUpdate,
  Reminder,
  ReminderControl,
} from '@/generated/agenaState'
export type {
  AssistantSnapshot,
  AssistantProfile,
  AssistantUpdate,
  AssistantMemory,
  AssistantMemoryUpdate,
  Reminder,
  ReminderControl,
}

async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
  const controller = new AbortController()
  const abort = () => controller.abort()
  if (init.signal?.aborted) controller.abort()
  else init.signal?.addEventListener('abort', abort, { once: true })
  const timer = setTimeout(abort, 20000)
  try {
    return await apiJson<T>(path, { ...init, signal: controller.signal })
  } finally {
    clearTimeout(timer)
    init.signal?.removeEventListener('abort', abort)
  }
}
const base = (source: number) => `/api/v1/sessions/${source}/assistant`
const body = (method: string, value: unknown) => ({
  method,
  headers: { 'content-type': 'application/json' },
  body: JSON.stringify(value),
})
export const assistantApi = {
  get: (source: number, signal?: AbortSignal) => request<AssistantSnapshot>(base(source), { signal }),
  update: (source: number, update: AssistantUpdate) => request<AssistantProfile>(base(source), body('PUT', update)),
  memory: (source: number, update: AssistantMemoryUpdate) =>
    request<AssistantMemory | null>(`${base(source)}/memory`, body('POST', update)),
  reminder: (source: number, id: string, control: ReminderControl) =>
    request<Reminder>(`${base(source)}/reminders/${encodeURIComponent(id)}`, body('POST', control)),
}
