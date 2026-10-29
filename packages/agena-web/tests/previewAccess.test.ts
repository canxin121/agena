import { describe, expect, test } from 'bun:test'
import {
  buildIsolatedPreviewFrameSrc,
  normalizePreviewAccess,
  previewAccessOrigins,
  previewRunIdentity,
} from '../src/features/workspacePreview/model/previewAccess'
import { buildPreviewFrameSrc, normalizePreviewProxyBasePath } from '../src/features/workspacePreview/model/previewUrl'

const now = 1_700_000_000_000
const origins = { backendOrigin: 'https://api.example.com', workbenchOrigin: 'https://agena.example.com' }
const grant = { url: 'https://p-random.preview.example.com/', expiresAt: now + 30 * 60_000, generation: 'run-1' }

describe('preview access boundary', () => {
  test('request origins preserve a separate backend instead of resolving frames against the UI', () => {
    expect(previewAccessOrigins('https://api.example.com/', 'https://agena.example.com/preview?id=demo')).toEqual(
      origins,
    )
    expect(previewAccessOrigins('/', 'http://localhost:5173/preview')).toEqual({
      backendOrigin: 'http://localhost:5173',
      workbenchOrigin: 'http://localhost:5173',
    })
    for (const url of ['file:///tmp/app', 'javascript:alert(1)', 'https://token:secret@example.com']) {
      expect(() => previewAccessOrigins(url, origins.workbenchOrigin)).toThrow()
    }
  })

  test('valid isolated URLs stay absolute, with only the cache-busting query added', () => {
    const access = normalizePreviewAccess(grant, origins, now)
    const src = new URL(buildIsolatedPreviewFrameSrc(access, 3))
    expect(src.origin).toBe('https://p-random.preview.example.com')
    expect(src.pathname).toBe('/')
    expect([...src.searchParams.entries()]).toEqual([['__oc_preview_refresh', '3']])
    expect(buildIsolatedPreviewFrameSrc(access, 3)).toBe(
      buildIsolatedPreviewFrameSrc({ ...access, expiresAt: now + 60 * 60_000 }, 3),
    )
    expect(buildIsolatedPreviewFrameSrc(access, -2)).toEndWith('=0')
  })

  test('legacy paths, same-origin responses, credentials and opaque URLs cannot become frames', () => {
    for (const url of [
      '/api/v1/workbench/preview/s/demo/',
      '//p-other.preview.example.com/',
      origins.workbenchOrigin,
      origins.backendOrigin,
      'javascript:alert(1)',
      'data:text/html,hello',
      'https://p-random.preview.example.com/app',
      'https://p-random.preview.example.com/?token=secret',
      'https://p-random.preview.example.com/#token',
      'https://user:pass@p-random.preview.example.com/',
      'https://example.com/',
    ]) {
      expect(() => normalizePreviewAccess({ ...grant, url }, origins, now)).toThrow()
    }
    expect(() =>
      normalizePreviewAccess(grant, { ...origins, workbenchOrigin: 'https://p-random.preview.example.com' }, now),
    ).toThrow('isolated')
  })

  test('expired and malformed capabilities are rejected before navigation', () => {
    for (const expiresAt of [now, now - 1, '1800000000000', Number.NaN, Number.POSITIVE_INFINITY, undefined]) {
      expect(() => normalizePreviewAccess({ ...grant, expiresAt }, origins, now)).toThrow()
    }
    for (const value of [null, [], {}, { ...grant, generation: '' }]) {
      expect(() => normalizePreviewAccess(value, origins, now)).toThrow()
    }
  })

  test('restarts, state and target changes invalidate the run identity', () => {
    const session = {
      id: 'demo',
      directory: '/work',
      runDirectory: '/work',
      targetUrl: 'http://127.0.0.1:5173',
      command: 'node',
      args: ['server.js'],
      pid: 123,
      startedAt: 1,
      state: 'running',
    }
    const identity = previewRunIdentity(session)
    for (const patch of [
      { startedAt: 2 },
      { state: 'stopped' },
      { targetUrl: 'http://127.0.0.1:5174' },
      { args: ['other.js'] },
      { agenaSessionId: 'other' },
      { id: 'renamed' },
      { instanceId: 'a-new-registration-with-the-same-id' },
    ]) {
      expect(previewRunIdentity({ ...session, ...patch })).not.toBe(identity)
    }
    expect(previewRunIdentity({ ...session })).toBe(identity)
    expect(previewRunIdentity(null)).toBe('')
  })

  test('bearer-bearing diagnostics only accept relative control proxy paths', () => {
    const path = buildPreviewFrameSrc('/api/v1/workbench/preview/s/demo/', 7)
    expect(normalizePreviewProxyBasePath(path)).toBe(path)
    for (const path of [
      grant.url,
      '//evil.example.com/',
      '/api/v1/sessions',
      '/api/v1/workbench/preview/s/../../sessions',
    ]) {
      expect(normalizePreviewProxyBasePath(path)).toBe('')
    }
  })
})
