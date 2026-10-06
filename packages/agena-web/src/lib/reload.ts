import { apiJson } from './api'
import { waitForRuntimeTask, type RuntimeBackgroundTask } from './runtimeTask'

export type RuntimeReloadTaskResponse = {
  started: boolean
  task: RuntimeBackgroundTask
}

export async function reloadAgenaRuntime(signal?: AbortSignal): Promise<RuntimeReloadTaskResponse> {
  const response = await apiJson<RuntimeReloadTaskResponse>('/api/v1/runtime/reload', {
    method: 'POST',
    signal: signal ? AbortSignal.any([signal, AbortSignal.timeout(15_000)]) : AbortSignal.timeout(15_000),
  })
  // Completion is published after the replacement generation is installed.
  const task = await waitForRuntimeTask(response.task, { signal, timeoutMs: 5000 })
  if (task.status !== 'succeeded') {
    throw new Error(
      task.failure?.fallback ||
        task.failure?.user?.fallback ||
        task.message ||
        'The runtime reload did not complete successfully',
    )
  }
  return { ...response, task }
}
