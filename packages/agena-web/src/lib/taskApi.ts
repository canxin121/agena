import { apiJson } from './api'
import type { Task, TaskState, CreateTask, ResumeTask } from '@/generated/agenaState'

export type { Task, TaskState, CreateTask, ResumeTask }
export type TaskQuery = { source_session_id?: number; state?: TaskState; offset: number; limit: number }
export interface TaskApi {
  list(query: TaskQuery, signal?: AbortSignal): Promise<Task[]>
  get(id: string, signal?: AbortSignal): Promise<Task>
  create(request: CreateTask): Promise<Task>
  resume(id: string, request: ResumeTask): Promise<Task>
  cancel(id: string, revision: number): Promise<Task>
  accept(id: string, revision: number): Promise<Task>
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

function post<T>(path: string, body: unknown): Promise<T> {
  return request<T>(path, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  })
}

export const taskApi: TaskApi = {
  list(query, signal) {
    const params = new URLSearchParams()
    for (const [key, value] of Object.entries(query)) if (value !== undefined) params.set(key, String(value))
    return request<Task[]>(`/api/v1/tasks?${params}`, { signal })
  },
  get: (id, signal) => request<Task>(`/api/v1/tasks/${encodeURIComponent(id)}`, { signal }),
  create: (request) => post<Task>('/api/v1/tasks', request),
  resume: (id, request) => post<Task>(`/api/v1/tasks/${encodeURIComponent(id)}/resume`, request),
  cancel: (id, expected_revision) =>
    post<Task>(`/api/v1/tasks/${encodeURIComponent(id)}/cancel`, { expected_revision }),
  accept: (id, expected_revision) =>
    post<Task>(`/api/v1/tasks/${encodeURIComponent(id)}/accept`, { expected_revision }),
}
