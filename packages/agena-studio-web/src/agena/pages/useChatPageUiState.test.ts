import { describe, expect, test } from 'bun:test'
import { reactive, ref } from 'vue'

import type { ProviderModel, ProviderSummary, WorkspaceResource } from '@/agena/lib/agenaApi'

import { useChatPageUiState } from './useChatPageUiState'

function createInput() {
  const providerModels: Record<string, ProviderModel[]> = {
    anthropic: [
      { provider_id: 'anthropic', id: 'claude-opus-4-7', display_name: 'Claude Opus 4.7' },
      { provider_id: 'anthropic', id: 'claude-sonnet-4-6' },
    ],
  }
  const providers = ref<ProviderSummary[]>([
    { provider_id: 'anthropic', default_model: 'claude-opus-4-7', default_model_ref: 'anthropic/claude-opus-4-7' },
  ])
  const workspaces = ref<WorkspaceResource[]>([
    { id: 1, path: '/repo-a', created_at: '2026-05-10T00:00:00Z', updated_at: '2026-05-10T00:00:00Z' },
  ])
  const pushes: Array<{ path: string; query: Record<string, string> }> = []
  const input = {
    localCommandNotice: ref(''),
    providerModels,
    providers,
    selectedModelId: ref(''),
    selectedProviderId: ref(''),
    selectedWorkspaceId: ref<number | null>(null),
    userInputDrafts: reactive<Record<string, Record<string, string>>>({}),
    workspaces,
  }
  const ui = useChatPageUiState(input, {
    router: {
      push(location) {
        pushes.push(location as { path: string; query: Record<string, string> })
        return Promise.resolve()
      },
    },
  })

  return { input, pushes, ui }
}

describe('useChatPageUiState', () => {
  test('provider helpers expose defaults, labels, and options', () => {
    const { ui } = createInput()

    expect(ui.providerDefaultModel('anthropic')).toBe('claude-opus-4-7')
    expect(ui.providerModelOptions('anthropic').map((item) => item.id)).toEqual(['claude-opus-4-7', 'claude-sonnet-4-6'])
    expect(ui.providerModelLabel({ provider_id: 'anthropic', id: 'claude-opus-4-7', display_name: ' Claude Opus 4.7 ' })).toBe(
      'Claude Opus 4.7',
    )
    expect(ui.providerModelLabel({ provider_id: 'anthropic', id: 'claude-sonnet-4-6' })).toBe('claude-sonnet-4-6')
  })

  test('watch selects provider default model when model is empty', async () => {
    const { input } = createInput()
    const ui = useChatPageUiState(input, { router: { push: async () => {} } })

    input.selectedProviderId.value = 'anthropic'
    await Promise.resolve()

    expect(input.selectedModelId.value).toBe('claude-opus-4-7')
    expect(ui.providerModelOptions('anthropic').length).toBe(2)
  })

  test('workspace and runtime navigation normalize route queries', () => {
    const { input, pushes, ui } = createInput()
    input.selectedWorkspaceId.value = 1

    ui.openWorkspaceBrowser('/src/main.ts')
    ui.openRuntimeSection('plugins', 'installed')

    expect(pushes).toEqual([
      { path: '/workspace', query: { workspace: '1', path: 'src/main.ts' } },
      '/plugins/installed',
    ])
  })

  test('user input drafts and usage notice update in place', () => {
    const { input, ui } = createInput()

    expect(ui.readUserAnswer('req1', 'q1')).toBe('')
    ui.updateUserAnswer('req1', 'q1', 'alpha')
    expect(ui.readUserAnswer('req1', 'q1')).toBe('alpha')

    ui.copySessionUsageSummary(['turns 2', 'cost $0.02'])
    expect(input.localCommandNotice.value).toBe('Session usage: turns 2 · cost $0.02')

    ui.copySessionUsageSummary([])
    expect(input.localCommandNotice.value).toBe('No assistant usage has been recorded for the active session yet.')
  })
})
