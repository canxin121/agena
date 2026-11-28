import { describe, expect, it } from 'bun:test'

import {
  acpDirectoryActivityState,
  acpSessionStatusDot,
  filterAcpDirectories,
  mergeProjectConversations,
  type AcpDraftSession,
} from '../src/layout/acpSidebar/acpSidebarModel'
import type { NativeIndexedSession } from '../src/features/sessions/api/nativeAgentSessions'
import type { DirectoryEntry } from '../src/features/sessions/model/types'

describe('acpSidebarModel', () => {
  const nativeSessions: NativeIndexedSession[] = [
    {
      native_id: 'nat-1',
      cwd: '/repo/a',
      title: 'Fix login throttling',
      updated_at: '2026-11-28T10:00:00Z',
      archived: false,
      model: 'claude-sonnet',
      state: 'running',
      last_prompt_at: null,
    },
    {
      native_id: 'nat-2',
      cwd: '/repo/a',
      title: 'Audit MCP transport',
      updated_at: '2026-11-28T09:00:00Z',
      archived: false,
      model: null,
      state: 'idle',
      last_prompt_at: null,
    },
  ]

  it('merges unsent drafts and deduplicates linked native sessions', () => {
    const drafts: AcpDraftSession[] = [
      {
        sessionId: '4503599627370497',
        workspaceId: 1,
        agent: 'antigravity',
        title: 'Antigravity',
        updatedAtMs: Date.parse('2026-11-28T11:00:00Z'),
      },
      {
        sessionId: '4503599627370498',
        workspaceId: 1,
        agent: 'antigravity',
        title: 'Linked draft that should deduplicate',
        updatedAtMs: Date.parse('2026-11-28T10:30:00Z'),
      },
      {
        sessionId: '4503599627370499',
        workspaceId: 2,
        agent: 'antigravity',
        title: 'Other workspace draft',
        updatedAtMs: Date.parse('2026-11-28T11:30:00Z'),
      },
    ]

    const rows = mergeProjectConversations({
      workspaceId: 1,
      agent: 'antigravity',
      nativeSessions,
      capabilities: { index: true, archive: true, batch_get: false, watch: true },
      drafts,
      sessionIdByNativeId: new Map([['nat-1', '4503599627370498']]),
      nativeIdBySessionId: new Map([['4503599627370498', 'nat-1']]),
    })

    expect(rows.map((r) => r.key)).toEqual([
      'draft:4503599627370497',
      'native:nat-1',
      'native:nat-2',
    ])
    expect(rows[1]?.sessionId).toBe('4503599627370498')
    expect(rows[2]?.sessionId).toBeNull()
  })

  it('filters rows and directories by query across directory labels and session titles', () => {
    const rows = mergeProjectConversations({
      workspaceId: 1,
      agent: 'antigravity',
      nativeSessions,
      capabilities: null,
      drafts: [],
      sessionIdByNativeId: new Map(),
      nativeIdBySessionId: new Map(),
      query: 'mcp',
    })
    expect(rows.map((r) => r.nativeId)).toEqual(['nat-2'])

    const dirs: DirectoryEntry[] = [
      { id: '1', path: '/repo/alpha', label: 'alpha' },
      { id: '2', path: '/repo/beta', label: 'beta' },
    ]
    const matched = filterAcpDirectories(dirs, 'mcp', (id) => id === '1')
    expect(matched.map((d) => d.id)).toEqual(['1'])
  })

  it('computes session dots and directory activity states', () => {
    expect(acpSessionStatusDot('running').dotClass).toContain('animate-pulse')
    expect(acpSessionStatusDot('requires_action').dotClass).toContain('bg-amber-500')
    expect(acpSessionStatusDot('idle').dotClass).toBe('')

    const runningOnly = mergeProjectConversations({
      workspaceId: 1,
      agent: 'antigravity',
      nativeSessions,
      capabilities: null,
      drafts: [],
      sessionIdByNativeId: new Map(),
      nativeIdBySessionId: new Map(),
    })
    expect(acpDirectoryActivityState(runningOnly)).toBe('running')

    const mixed = [
      ...runningOnly,
      { ...runningOnly[0]!, key: 'native:nat-3', state: 'requires_action' },
    ]
    expect(acpDirectoryActivityState(mixed)).toBe('mixed')
  })
})
