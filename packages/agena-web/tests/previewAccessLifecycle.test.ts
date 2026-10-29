import { describe, expect, test } from 'bun:test'
import { effectScope, ref } from 'vue'
import type { WorkspacePreviewSession } from '../src/features/workspacePreview/api/workspacePreviewApi'
import { usePreviewAccess } from '../src/features/workspacePreview/model/usePreviewAccess'
import type { WorkspacePreviewAccess } from '../src/features/workspacePreview/model/previewAccess'

function session(id: string): WorkspacePreviewSession {
  return {
    id,
    directory: '/work',
    runDirectory: '/work',
    command: 'node',
    args: [],
    targetUrl: 'http://127.0.0.1:5173/',
    proxyBasePath: `/api/v1/workbench/preview/s/${id}/`,
    logsPath: '/tmp/preview.log',
    state: 'running',
    pid: 42,
    startedAt: 1,
  }
}
function grant(id: string): WorkspacePreviewAccess {
  return { url: `https://p-${id}.preview.example.com/`, expiresAt: Date.now() + 30 * 60_000, generation: id }
}
const settle = async () => {
  await Promise.resolve()
  await Promise.resolve()
}

describe('preview access lifecycle', () => {
  test('a late response cannot switch the iframe back to the previous session', async () => {
    const active = ref<WorkspacePreviewSession | null>(session('old'))
    const requests: { id: string; signal?: AbortSignal; resolve: (value: WorkspacePreviewAccess) => void }[] = []
    const scope = effectScope()
    const view = scope.run(() =>
      usePreviewAccess({
        session: active,
        visible: ref(true),
        refreshToken: ref(0),
        request: (id, signal) =>
          new Promise((resolve) => {
            requests.push({ id, signal, resolve })
          }),
      }),
    )!
    try {
      active.value = session('new')
      expect(requests.length).toBe(2)
      expect(requests[0]!.signal!.aborted).toBe(true)
      requests[1]!.resolve(grant('new'))
      await settle()
      requests[0]!.resolve(grant('old'))
      await settle()
      expect(view.previewSrc.value).toStartWith('https://p-new.preview.example.com/')
    } finally {
      scope.stop()
    }
  })

  test('renewal preserves URL while restart withdraws the old grant immediately', async () => {
    const active = ref<WorkspacePreviewSession | null>(session('demo'))
    let count = 0
    const scope = effectScope()
    const view = scope.run(() =>
      usePreviewAccess({
        session: active,
        visible: ref(true),
        refreshToken: ref(0),
        request: async () => {
          count++
          return grant('demo')
        },
      }),
    )!
    try {
      await settle()
      const original = view.previewSrc.value
      await view.refreshAccess()
      expect(count).toBe(2)
      expect(view.previewSrc.value).toBe(original)
      active.value = { ...active.value!, startedAt: 2 }
      expect(view.previewSrc.value).toBe('')
      await settle()
      expect(count).toBe(3)
    } finally {
      scope.stop()
    }
  })

  test('configuration errors are visible, and disposal aborts pending authorization', async () => {
    const scope = effectScope()
    let signal: AbortSignal | undefined
    const view = scope.run(() =>
      usePreviewAccess({
        session: ref(session('demo')),
        visible: ref(true),
        refreshToken: ref(0),
        request: async (_id, requestSignal) => {
          signal = requestSignal
          throw new Error('Set AGENA_PREVIEW_ORIGIN')
        },
      }),
    )!
    await settle()
    expect(view.previewSrc.value).toBe('')
    expect(view.accessError.value).toBe('Set AGENA_PREVIEW_ORIGIN')
    const pending = view.refreshAccess()
    scope.stop()
    expect(signal!.aborted).toBe(true)
    await pending
  })

  test('hidden panes do not authorize until shown and stale grants cannot reappear after hiding', async () => {
    const visible = ref(false)
    const requests: { signal?: AbortSignal; resolve: (value: WorkspacePreviewAccess) => void }[] = []
    const scope = effectScope()
    const view = scope.run(() =>
      usePreviewAccess({
        session: ref(session('demo')),
        visible,
        refreshToken: ref(0),
        request: (_id, signal) =>
          new Promise((resolve) => {
            requests.push({ signal, resolve })
          }),
      }),
    )!
    try {
      expect(requests.length).toBe(0)
      visible.value = true
      expect(requests.length).toBe(1)
      visible.value = false
      expect(requests[0]!.signal!.aborted).toBe(true)
      requests[0]!.resolve(grant('demo'))
      await settle()
      expect(view.previewSrc.value).toBe('')
    } finally {
      scope.stop()
    }
  })
})
