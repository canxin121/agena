import { describe, expect, test } from 'bun:test'

import type { RouteLocationNormalizedLoaded } from 'vue-router'

import type { WorkspaceFileTreeResource } from '../lib/agenaApi'
import { renderVueSsr } from './test/renderVueSsr'
import { useWorkspacePageState } from './useWorkspacePageState'

function createRoute(query: Record<string, string> = {}) {
  return { query } as RouteLocationNormalizedLoaded
}

describe('WorkspacePageContent', () => {
  test('renders workspace cards, entry points, and file tree from real page state', async () => {
    const state = useWorkspacePageState(
      {
        route: createRoute({ workspace: '2', path: '/.agena/skills' }),
        router: {
          replace: async () => {},
          push: async () => {},
        } as never,
      },
      {
        createWorkspace: async (path) => ({ id: 3, path, created_at: 'x', updated_at: 'x', session_count: 0 }),
        deleteWorkspace: async (workspaceId) => ({ id: workspaceId, path: '/repo-b', created_at: 'x', updated_at: 'x', session_count: 4 }),
        getGitStatus: async () => null as never,
        listWorkspaceFileTree: async ({ workspaceId, path }): Promise<WorkspaceFileTreeResource> => ({
          workspace_id: workspaceId,
          root: '/repo-b',
          path: path || '',
          entries: [
            {
              kind: 'directory',
              name: '.agena',
              path: '.agena',
              children: [
                { kind: 'directory', name: 'skills', path: '.agena/skills', children: [] },
                { kind: 'file', name: 'config.toml', path: '.agena/config.toml', size: 128 },
              ],
            },
          ],
        }),
        listWorkspaces: async () => [
          { id: 2, path: '/repo-b', created_at: 'x', updated_at: 'x', session_count: 4 },
        ],
        resolveWorkspace: async (path) => ({ id: 4, path, created_at: 'x', updated_at: 'x', session_count: 0 }),
        updateWorkspace: async ({ workspaceId, path }) => ({ id: workspaceId, path, created_at: 'x', updated_at: 'x', session_count: 4 }),
      },
    )

    await state.load()

    const html = await renderVueSsr('/src/agena/pages/WorkspacePageContent.vue', {
      workspace: state,
    })

    expect(html.includes('Resolve Workspace')).toBe(true)
    expect(html.includes('Project Status')).toBe(true)
    expect(html.includes('Current Workspace')).toBe(true)
    expect(html.includes('Project Entry Points')).toBe(true)
    expect(html.includes('Open Worktrees')).toBe(true)
    expect(html.includes('Open Logs')).toBe(true)
    expect(html.includes('File Tree')).toBe(true)
    expect(html.includes('/repo-b')).toBe(true)
    expect(html.includes('.agena/skills')).toBe(true)
    expect(html.includes('3 entries')).toBe(true)
  })
})
