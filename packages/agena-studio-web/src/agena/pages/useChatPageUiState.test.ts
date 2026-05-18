import { describe, expect, test } from 'bun:test'
import { nextTick, reactive, ref } from 'vue'

import type { ProviderModel, ProviderSummary, WorkspaceResource } from '@/agena/lib/agenaApi'

import { useChatPageUiState } from './useChatPageUiState'

function createInput() {
  const providerModels: Record<string, ProviderModel[]> = {
    anthropic: [
      { provider_id: 'anthropic', id: 'claude-opus-4-7', display_name: 'Claude Opus 4.7' },
      {
        provider_id: 'anthropic',
        id: 'claude-sonnet-4-6',
        metadata: {
          supports_parallel_tool_calls: true,
          supports_verbosity: true,
          default_verbosity: 'low',
        },
        thinking_modes: {
          light: { display_name: 'Light', description: 'Quick thinking' },
          deep: { display_name: 'Deep', description: 'More thinking' },
        },
        speed_modes: {
          fast: { display_name: 'Fast', description: 'Priority route' },
        },
      },
    ],
    openai: [
      {
        provider_id: 'openai',
        id: 'gpt-5.2-chat-latest',
        metadata: {
          supports_verbosity: true,
          default_verbosity: 'medium',
        },
      },
    ],
  }
  const providers = ref<ProviderSummary[]>([
    {
      provider_id: 'anthropic',
      default_adapter: 'anthropic',
      default_model: 'claude-opus-4-7',
      adapters: [{ adapter_id: 'anthropic', enabled: true, configured_model_count: 2 }],
    },
    {
      provider_id: 'openai',
      default_adapter: 'openai',
      default_model: 'gpt-5.2-chat-latest',
      adapters: [{ adapter_id: 'openai', enabled: true, configured_model_count: 1 }],
    },
  ])
  const workspaces = ref<WorkspaceResource[]>([
    { id: 1, path: '/repo-a', created_at: '2026-05-10T00:00:00Z', updated_at: '2026-05-10T00:00:00Z' },
  ])
  const pushes: Array<{ path: string; query: Record<string, string> }> = []
  const input = {
    localCommandNotice: ref(''),
    providerModels,
    providers,
    selectedAdapterId: ref(''),
    selectedModelId: ref(''),
    selectedProviderId: ref(''),
    selectedThinkingMode: ref(''),
    selectedSpeedMode: ref(''),
    selectedVerbosity: ref(''),
    selectedParallelToolCalls: ref(''),
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
    expect(ui.providerDefaultAdapter('anthropic')).toBe('anthropic')
    expect(ui.providerAdapterOptions('anthropic')).toEqual(['anthropic'])
    expect(ui.providerModelOptions('anthropic').map((item) => item.id)).toEqual([
      'claude-opus-4-7',
      'claude-sonnet-4-6',
    ])
    expect(
      ui.providerModelLabel({ provider_id: 'anthropic', id: 'claude-opus-4-7', display_name: ' Claude Opus 4.7 ' }),
    ).toBe('Claude Opus 4.7')
    expect(ui.providerModelLabel({ provider_id: 'anthropic', id: 'claude-sonnet-4-6' })).toBe('claude-sonnet-4-6')
  })

  test('mode options follow the selected model', async () => {
    const { input, ui } = createInput()

    input.selectedProviderId.value = 'anthropic'
    input.selectedModelId.value = 'claude-sonnet-4-6'

    expect(ui.modelThinkingModeOptions().map((item) => item.id)).toEqual(['light', 'deep'])
    expect(ui.modelSpeedModeOptions().map((item) => item.id)).toEqual(['fast'])
    expect(ui.modelVerbosityOptions().map((item) => item.id)).toEqual(['low', 'medium', 'high'])
    expect(ui.modelParallelToolCallsOptions().map((item) => item.id)).toEqual(['true', 'false'])

    input.selectedThinkingMode.value = 'deep'
    input.selectedSpeedMode.value = 'fast'
    input.selectedVerbosity.value = 'high'
    input.selectedParallelToolCalls.value = 'true'
    input.selectedModelId.value = 'claude-opus-4-7'
    await nextTick()

    expect(input.selectedThinkingMode.value).toBe('')
    expect(input.selectedSpeedMode.value).toBe('')
    expect(input.selectedVerbosity.value).toBe('')
    expect(input.selectedParallelToolCalls.value).toBe('')
    expect(ui.modelThinkingModeOptions()).toEqual([])
    expect(ui.modelSpeedModeOptions()).toEqual([])
    expect(ui.modelVerbosityOptions()).toEqual([])
    expect(ui.modelParallelToolCallsOptions()).toEqual([])
  })

  test('verbosity options respect model-specific constraints', () => {
    const { input, ui } = createInput()

    input.selectedProviderId.value = 'openai'
    input.selectedModelId.value = 'gpt-5.2-chat-latest'

    expect(ui.modelVerbosityOptions().map((item) => item.id)).toEqual(['medium'])
  })

  test('watch selects provider default model when model is empty', async () => {
    const { input } = createInput()
    const ui = useChatPageUiState(input, { router: { push: async () => {} } })

    input.selectedProviderId.value = 'anthropic'
    await Promise.resolve()

    expect(input.selectedModelId.value).toBe('claude-opus-4-7')
    expect(input.selectedAdapterId.value).toBe('anthropic')
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
