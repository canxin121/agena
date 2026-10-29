import { describe, expect, test } from 'bun:test'
import { effectScope, ref } from 'vue'
import type { WorkspacePreviewSession } from '../src/features/workspacePreview/api/workspacePreviewApi'
import type { PreviewProxyResponse } from '../src/features/workspacePreview/api/previewProxyProbe'
import { usePreviewHealth } from '../src/features/workspacePreview/model/usePreviewHealth'

const session = (startedAt = 1): WorkspacePreviewSession => ({
  id: 'app',
  directory: '/workspace',
  runDirectory: '/workspace',
  command: 'node',
  args: [],
  logsPath: '/tmp/app.log',
  targetUrl: 'http://127.0.0.1:5173/',
  proxyBasePath: '/api/v1/workbench/preview/s/app/',
  state: 'running',
  pid: 42,
  startedAt,
})
const response = (status: number, upstreamResponded = false): PreviewProxyResponse => ({
  status,
  ok: status === 200,
  upstreamResponded,
  body: '',
  contentType: '',
})
const settle = async () => {
  for (let index = 0; index < 8; index++) await Promise.resolve()
}

describe('preview health ownership', () => {
  test('a new run replaces its pending probe, and an old completion cannot clear or overwrite it', async () => {
    const sessions = ref([session()])
    const pending: { signal: AbortSignal; resolve: (response: PreviewProxyResponse) => void }[] = []
    const scope = effectScope()
    const health = scope.run(() =>
      usePreviewHealth({
        sessions,
        visible: ref(true),
        loading: ref(false),
        probe: async (_src, signal) => new Promise((resolve) => pending.push({ signal, resolve })),
      }),
    )!
    try {
      expect(pending.length).toBe(1)
      sessions.value = [session(2)]
      expect(pending.length).toBe(2)
      expect(pending[0]!.signal.aborted).toBe(true)
      pending[0]!.resolve(response(200))
      await settle()
      expect(health.entries.value.app?.state).toBe('checking')
      health.refresh()
      expect(pending.length).toBe(2)
      pending[1]!.resolve(response(503))
      await settle()
      expect(health.entries.value.app?.state).toBe('error')
    } finally {
      for (const job of pending) job.resolve(response(200))
      scope.stop()
    }
  })

  test('stopping a run cancels its probe and removes the obsolete health status', async () => {
    const sessions = ref([session()])
    let signal: AbortSignal | undefined
    let resolve!: (response: PreviewProxyResponse) => void
    const scope = effectScope()
    const health = scope.run(() =>
      usePreviewHealth({
        sessions,
        visible: ref(true),
        loading: ref(false),
        probe: async (_src, current) => {
          signal = current
          return new Promise((done) => {
            resolve = done
          })
        },
      }),
    )!
    try {
      sessions.value = [{ ...session(), state: 'stopped', pid: undefined }]
      expect(signal!.aborted).toBe(true)
      resolve(response(200))
      await settle()
      expect(health.entries.value.app).toBeUndefined()
    } finally {
      scope.stop()
    }
  })

  test('a startup failure is retried without a registry change, and hidden panes cancel retries', async () => {
    const visible = ref(true)
    let attempts = 0
    const scope = effectScope()
    const health = scope.run(() =>
      usePreviewHealth({
        sessions: ref([session()]),
        visible,
        loading: ref(false),
        retryMs: 5,
        probe: async () => response(++attempts === 1 ? 502 : 401, attempts > 1),
      }),
    )!
    try {
      await new Promise((resolve) => setTimeout(resolve, 30))
      expect(attempts).toBe(2)
      // The isolated iframe can render/login despite a cookieless health 401.
      expect(health.entries.value.app?.state).toBe('ok')
      visible.value = false
      health.clear()
      await new Promise((resolve) => setTimeout(resolve, 15))
      expect(attempts).toBe(2)
    } finally {
      scope.stop()
    }
  })

  test('hiding cancels a probe without converting cancellation into an offline result', async () => {
    const visible = ref(true)
    let signal: AbortSignal | undefined
    const scope = effectScope()
    const health = scope.run(() =>
      usePreviewHealth({
        sessions: ref([session()]),
        visible,
        loading: ref(false),
        probe: async (_src, current) => {
          signal = current
          return new Promise((_, reject) => current.addEventListener('abort', () => reject(current.reason)))
        },
      }),
    )!
    try {
      visible.value = false
      await settle()
      expect(signal!.aborted).toBe(true)
      expect(health.entries.value.app).toBeUndefined()
    } finally {
      scope.stop()
    }
  })
})
