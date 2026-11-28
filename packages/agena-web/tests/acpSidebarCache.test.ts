import { beforeEach, describe, expect, it } from 'bun:test'

import {
  invalidateCachedAcpProject,
  readCachedAcpProject,
  writeCachedAcpProject,
} from '@/layout/acpSidebar/acpSidebarCache'
import { sessionStorageKeys } from '@/lib/persistence/storageKeys'

function installMemorySessionStorage() {
  const map = new Map<string, string>()
  const storage = {
    get length() {
      return map.size
    },
    clear() {
      map.clear()
    },
    getItem(key: string) {
      return map.has(key) ? map.get(key)! : null
    },
    key(index: number) {
      return [...map.keys()][index] ?? null
    },
    removeItem(key: string) {
      map.delete(key)
    },
    setItem(key: string, value: string) {
      map.set(key, String(value))
    },
  }
  ;(globalThis as unknown as { sessionStorage: Storage }).sessionStorage = storage as Storage
  return storage
}

describe('acpSidebarCache', () => {
  beforeEach(() => {
    installMemorySessionStorage()
  })

  it('stores and reads cached project entries per agent and workspace', () => {
    const now = 1_700_000_000_000
    writeCachedAcpProject(
      'antigravity',
      '42',
      {
        sessions: [
          {
            native_id: 'nat-1',
            cwd: '/repo/a',
            title: 'Cached thread',
            updated_at: '2026-11-28T10:00:00Z',
            archived: null,
            model: null,
            state: null,
            last_prompt_at: null,
          },
        ],
        cursor: null,
        capabilities: { index: false, archive: false, batch_get: false, watch: false },
        showArchived: false,
      },
      now,
    )

    const hit = readCachedAcpProject('antigravity', '42', now + 1000)
    expect(hit).not.toBeNull()
    expect(hit?.sessions).toHaveLength(1)
    expect(hit?.sessions[0]?.title).toBe('Cached thread')
    expect(readCachedAcpProject('claude-code', '42', now + 1000)).toBeNull()
  })

  it('expires entries older than 30 minutes', () => {
    const now = 1_700_000_000_000
    writeCachedAcpProject(
      'antigravity',
      '1',
      { sessions: [], cursor: null, capabilities: null, showArchived: false },
      now,
    )
    expect(readCachedAcpProject('antigravity', '1', now + 29 * 60 * 1000)).not.toBeNull()
    expect(readCachedAcpProject('antigravity', '1', now + 31 * 60 * 1000)).toBeNull()
  })

  it('bounds stored entries to 20 most recently cached projects', () => {
    const base = 1_700_000_000_000
    for (let i = 1; i <= 25; i++) {
      writeCachedAcpProject(
        'antigravity',
        String(i),
        { sessions: [], cursor: null, capabilities: null, showArchived: false },
        base + i * 1000,
      )
    }

    const raw = sessionStorage.getItem(sessionStorageKeys.acp.sidebarListCache)
    const parsed = JSON.parse(raw || '{}') as Record<string, unknown>
    expect(Object.keys(parsed)).toHaveLength(20)
    expect(readCachedAcpProject('antigravity', '1', base + 30_000)).toBeNull()
    expect(readCachedAcpProject('antigravity', '25', base + 30_000)).not.toBeNull()
  })

  it('invalidates by workspace, by agent, or globally', () => {
    const now = 1_700_000_000_000
    writeCachedAcpProject('antigravity', '1', { sessions: [], cursor: null, capabilities: null, showArchived: false }, now)
    writeCachedAcpProject('antigravity', '2', { sessions: [], cursor: null, capabilities: null, showArchived: false }, now)
    writeCachedAcpProject('claude-code', '1', { sessions: [], cursor: null, capabilities: null, showArchived: false }, now)

    invalidateCachedAcpProject('antigravity', '1')
    expect(readCachedAcpProject('antigravity', '1', now)).toBeNull()
    expect(readCachedAcpProject('antigravity', '2', now)).not.toBeNull()

    invalidateCachedAcpProject('antigravity')
    expect(readCachedAcpProject('antigravity', '2', now)).toBeNull()
    expect(readCachedAcpProject('claude-code', '1', now)).not.toBeNull()

    invalidateCachedAcpProject()
    expect(readCachedAcpProject('claude-code', '1', now)).toBeNull()
  })
})
