import { describe, expect, test } from 'bun:test'

import type { RouteLocationNormalizedLoaded } from 'vue-router'

import type { WorkspaceFileTreeResource } from '../lib/agenaApi'
import { formatWorkspaceNodeSize, useWorkspacePageState } from './useWorkspacePageState'

function createRoute(query: Record<string, string> = {}) {
  return { query } as RouteLocationNormalizedLoaded
}

describe('useWorkspacePageState', () => {
  test('formats workspace node sizes', () => {
    expect(formatWorkspaceNodeSize({ kind: 'directory', name: 'src', path: 'src', children: [] })).toBe('dir')
    expect(formatWorkspaceNodeSize({ kind: 'file', name: 'a.ts', path: 'a.ts', size: 12 })).toBe('12 B')
    expect(formatWorkspaceNodeSize({ kind: 'file', name: 'a.ts', path: 'a.ts', size: 2048 })).toBe('2.0 KB')
  })

  test('loads workspace tree, summaries, and normalized route query', async () => {
    const replaceCalls: Array<{ path: string; query: Record<string, string> }> = []
    const pushCalls: Array<{ path: string; query?: Record<string, string | undefined> }> = []
    const state = useWorkspacePageState(
      {
        route: createRoute({ workspace: '2', path: '/.agena/skills' }),
        router: {
          replace: async (value: unknown) => {
            replaceCalls.push(value as { path: string; query: Record<string, string> })
          },
          push: async (value: unknown) => {
            pushCalls.push(value as { path: string; query?: Record<string, string | undefined> })
          },
        } as never,
      },
      {
        createWorkspace: async (path) => ({ id: 3, path, created_at: 'x', updated_at: 'x', session_count: 0 }),
        getGitStatus: async () => null as never,
        listWorkspaceFileTree: async ({ workspaceId, path }): Promise<WorkspaceFileTreeResource> => ({
          workspace_id: workspaceId,
          root: '/repo-b',
          path: path || '',
          entries: [
            { kind: 'directory', name: '.agena', path: '.agena', children: [{ kind: 'file', name: 'x.md', path: '.agena/x.md', size: 128 }] },
          ],
        }),
        listWorkspaces: async () => [
          { id: 1, path: '/repo-a', created_at: 'x', updated_at: 'x', session_count: 1 },
          { id: 2, path: '/repo-b', created_at: 'x', updated_at: 'x', session_count: 4 },
        ],
        resolveWorkspace: async (path) => ({ id: 4, path, created_at: 'x', updated_at: 'x', session_count: 0 }),
      },
    )

    await state.load()

    expect(state.selectedWorkspaceId.value).toBe(2)
    expect(state.selectedWorkspace.value?.path).toBe('/repo-b')
    expect(state.pathInput.value).toBe('.agena/skills')
    expect(state.rows.value.length).toBe(2)
    expect(state.workspaceSummaryFacts.value).toEqual(['id=2', 'sessions=4', 'updated=x'])
    expect(state.configSummaryFacts.value.includes('entry_points=6')).toBe(true)
    expect(replaceCalls).toEqual([{ path: '/workspace', query: { workspace: '2', path: '.agena/skills' } }])

    state.openSettingsForShortcut('hooks')
    state.openRuntimeForWorkspace()
    state.openChatForWorkspace()
    state.openRuntimeConfigRoot()
    state.openWorktreeDirectory()
    state.openLogsDirectory()

    expect(pushCalls).toEqual([
      { path: '/settings/permissions', query: { workspace: '2' } },
      { path: '/runtime/workflow', query: { workspace: '2' } },
      { path: '/chat', query: { workspace: '2' } },
    ])
    expect(state.pathInput.value).toBe('.agena/logs')
  })

  test('resolves workspace and reports success message', async () => {
    const state = useWorkspacePageState(
      {
        route: createRoute(),
        router: { replace: async () => {}, push: async () => {} } as never,
      },
      {
        createWorkspace: async (path) => ({ id: 5, path, created_at: 'x', updated_at: 'x', session_count: 0 }),
        getGitStatus: async () => null as never,
        listWorkspaceFileTree: async ({ workspaceId, path }): Promise<WorkspaceFileTreeResource> => ({ workspace_id: workspaceId, root: '/repo-c', path: path || '', entries: [] }),
        listWorkspaces: async () => [{ id: 5, path: '/repo-c', created_at: 'x', updated_at: 'x', session_count: 0 }],
        resolveWorkspace: async (path) => ({ id: 5, path, created_at: 'x', updated_at: 'x', session_count: 0 }),
      },
    )

    state.workspacePath.value = '/repo-c'
    await state.resolveWorkspaceAction(true)

    expect(state.selectedWorkspaceId.value).toBe(5)
    expect(state.actionMessage.value).toBe('Opened workspace /repo-c.')
  })
})
