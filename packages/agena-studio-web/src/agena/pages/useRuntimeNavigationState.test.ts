import { describe, expect, test } from 'bun:test'
import { ref } from 'vue'

import { useRuntimeNavigationState } from './useRuntimeNavigationState'

function createState() {
  const pushes: Array<string | { path: string; query: Record<string, string> }> = []
  const selectedSessionId = ref<number | null>(7)
  const selectedWorkspaceId = ref<number | null>(1)
  const selectedPluginManifest = ref<unknown>({ name: 'demo-plugin' })
  const workspaces = ref([
    {
      id: 1,
      path: '/repo',
      created_at: '2026-05-10T00:00:00Z',
      updated_at: '2026-05-10T00:00:00Z',
    },
  ])

  const navigation = useRuntimeNavigationState(
    {
      selectedSessionId,
      selectedWorkspaceId,
      selectedPluginManifest,
      workspaces,
    },
    {
      router: {
        push: async (value) => {
          pushes.push(value as string | { path: string; query: Record<string, string> })
        },
      },
    },
  )

  return { navigation, pushes, selectedPluginManifest, selectedSessionId, selectedWorkspaceId, workspaces }
}

describe('useRuntimeNavigationState', () => {
  test('opens selected session in chat', () => {
    const { navigation, pushes } = createState()

    navigation.openSelectedSessionInChat()

    expect(pushes).toEqual(['/chat?session=7'])
  })

  test('opens workspace paths and shortcuts with normalized query', () => {
    const { navigation, pushes } = createState()

    navigation.openWorkspacePath('/src/main.ts')
    navigation.openRuntimeConfigRoot()
    navigation.openPluginLogsWorkspacePath()

    expect(pushes).toEqual([
      { path: '/workspace', query: { workspace: '1', path: 'src/main.ts' } },
      { path: '/workspace', query: { workspace: '1', path: '.agena' } },
      { path: '/workspace', query: { workspace: '1', path: '.agena/logs' } },
    ])
  })

  test('opens plugin/runtime entry sources and chat slash handoff', () => {
    const { navigation, pushes } = createState()

    navigation.openPluginManifestInWorkspace()
    navigation.openRuntimeEntrySource({ name: 'review', description: '', aliases: [], source_path: '.agena/skills/review.md' })
    navigation.openRuntimeEntryInChat({ name: 'review', description: '', aliases: [], source_path: '.agena/skills/review.md' })

    expect(pushes).toEqual([
      { path: '/workspace', query: { workspace: '1', path: '.agena/plugins' } },
      { path: '/workspace', query: { workspace: '1', path: '.agena/skills/review.md' } },
      { path: '/chat', query: { slash: '/review', session: '7' } },
    ])
  })

  test('skips workspace-dependent actions when no workspace is selected', () => {
    const { navigation, pushes, selectedWorkspaceId } = createState()
    selectedWorkspaceId.value = null

    navigation.openWorkspacePath('src')
    navigation.openWorkspaceShortcut('commands')

    expect(pushes).toEqual([])
  })
})
