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
        getGitStatus: async () => ({
          workspace_root: '/repo-b',
          git_available: true,
          repo: true,
          gh_available: false,
          branch: 'main',
          upstream: null,
          ahead: 0,
          behind: 0,
          staged_files: 0,
          unstaged_files: 1,
          untracked_files: 1,
          changed_files: 2,
          clean: false,
          worktree_active_sessions: 0,
          worktree_managed_dirs: 0,
        }),
        getVcsDiffRaw: async () => 'diff --git a/.agena/config.toml b/.agena/config.toml\n+mode = "studio"\n',
        initGitProject: async () => ({
          workspace_root: '/repo-b',
          git_available: true,
          repo: true,
          gh_available: false,
          branch: 'main',
          upstream: null,
          ahead: null,
          behind: null,
          staged_files: 0,
          unstaged_files: 0,
          untracked_files: 0,
          changed_files: 0,
          clean: true,
          worktree_active_sessions: 0,
          worktree_managed_dirs: 0,
        }),
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
    await state.loadVcsDiffRawAction()

    const html = await renderVueSsr('/src/agena/pages/WorkspacePageContent.vue', {
      workspace: state,
    })

    expect(html.includes('Resolve Workspace')).toBe(true)
    expect(html.includes('Project Status')).toBe(true)
    expect(html.includes('Load Raw Diff')).toBe(true)
    expect(html.includes('Current Workspace')).toBe(true)
    expect(html.includes('Project Entry Points')).toBe(true)
    expect(html.includes('Open Worktrees')).toBe(true)
    expect(html.includes('Open Logs')).toBe(true)
    expect(html.includes('File Tree')).toBe(true)
    expect(html.includes('/repo-b')).toBe(true)
    expect(html.includes('.agena/skills')).toBe(true)
    expect(html.includes('3 entries')).toBe(true)
    expect(html.includes('raw_diff=available')).toBe(true)
    expect(html.includes('diff --git a/.agena/config.toml b/.agena/config.toml')).toBe(true)
  })
})
