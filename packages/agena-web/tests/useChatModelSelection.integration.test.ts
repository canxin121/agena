import test from 'node:test'
import assert from 'node:assert/strict'

import { nextTick } from 'vue'

import { localStorageKeys } from '../src/lib/persistence/storageKeys'
import { createTestHarness } from './testRuntime'

const CPA_PROVIDERS = [
  {
    id: 'cpa',
    name: 'CPA',
    models: [
      { provider_id: 'cpa', adapter_id: 'openai_responses', id: 'deepseek-v4.1-flash' },
      { provider_id: 'cpa', adapter_id: 'openai_responses', id: 'gpt-5.5' },
    ],
  },
]

const CPA_DEFAULT_SELECTION = {
  provider: 'cpa',
  adapter: 'openai_responses',
  model: 'deepseek-v4.1-flash',
  thinkingMode: 'max',
  speedMode: '',
  verbosity: '',
}

test('useChatModelSelection: uses the runtime-wide default when the session has no model', async () => {
  const { selection } = await createTestHarness({ selectedSessionId: 'session-global-default' })

  selection.runtimeDefaultSelection.value = {
    provider: 'default-provider',
    adapter: 'default-adapter',
    model: 'default-model',
    thinkingMode: 'high',
    speedMode: 'fast',
    verbosity: 'compact',
  }
  selection.applySessionSelection()

  assert.equal(selection.selectedProviderId.value, 'default-provider')
  assert.equal(selection.selectedAdapterId.value, 'default-adapter')
  assert.equal(selection.selectedModelId.value, 'default-model')
  assert.equal(selection.modelSource.value, 'default')
  assert.equal(selection.selectedThinkingMode.value, 'high')
  assert.equal(selection.selectedSpeedMode.value, 'fast')
})

test('useChatModelSelection: restores per-session manual model after session switch', async () => {
  const { chat, selection } = await createTestHarness({ selectedSessionId: 'session-1' })

  selection.chooseModelSlug('manual-provider/manual-adapter/manual-model')
  assert.equal(selection.selectedProviderId.value, 'manual-provider')
  assert.equal(selection.selectedAdapterId.value, 'manual-adapter')
  assert.equal(selection.selectedModelId.value, 'manual-model')

  chat.selectedSessionId = 'session-2'
  chat.selectedSessionRunConfig = {
    providerID: 'session-provider',
    modelID: 'session-model',
    at: 1,
  }
  selection.resetSelectionForSessionSwitch()
  selection.applySessionSelection()

  assert.equal(selection.selectedProviderId.value, 'session-provider')
  assert.equal(selection.selectedModelId.value, 'session-model')

  chat.selectedSessionId = 'session-1'
  chat.selectedSessionRunConfig = { at: 2 }
  chat.messages = []
  selection.resetSelectionForSessionSwitch()
  selection.applySessionSelection()

  assert.equal(selection.selectedProviderId.value, 'manual-provider')
  assert.equal(selection.selectedAdapterId.value, 'manual-adapter')
  assert.equal(selection.selectedModelId.value, 'manual-model')
})

test('useChatModelSelection: session switch without run-config or message changes restores the global default', async () => {
  const { chat, selection } = await createTestHarness({
    selectedSessionId: 'session-before-project',
  })

  selection.runtimeDefaultSelection.value = {
    provider: 'default-provider',
    adapter: 'default-adapter',
    model: 'default-model',
    thinkingMode: '',
    speedMode: '',
    verbosity: '',
  }
  selection.applySessionSelection()
  selection.chooseModelSlug('manual-provider/manual-adapter/manual-model')
  assert.equal(selection.selectedModelId.value, 'manual-model')

  // A newly created project's session can have neither a run config nor any
  // messages yet. Those watched values remain unchanged while its ID changes.
  chat.selectedSessionId = 'session-new-project'
  await nextTick()

  assert.equal(selection.selectedProviderId.value, 'default-provider')
  assert.equal(selection.selectedAdapterId.value, 'default-adapter')
  assert.equal(selection.selectedModelId.value, 'default-model')
  assert.equal(selection.modelSource.value, 'default')
})

test('useChatModelSelection: manual model overrides newer session run-config', async () => {
  const { chat, selection } = await createTestHarness({
    selectedSessionId: 'session-1',
    selectedSessionRunConfig: {
      providerID: 'session-provider',
      modelID: 'session-model',
      at: 1,
    },
  })

  selection.applySessionSelection()
  assert.equal(selection.selectedProviderId.value, 'session-provider')
  assert.equal(selection.selectedModelId.value, 'session-model')

  selection.chooseModelSlug('manual-provider/manual-adapter/manual-model')
  chat.selectedSessionRunConfig = {
    providerID: 'new-session-provider',
    modelID: 'new-session-model',
    at: 2,
  }
  selection.applySessionSelection()

  assert.equal(selection.selectedProviderId.value, 'manual-provider')
  assert.equal(selection.selectedAdapterId.value, 'manual-adapter')
  assert.equal(selection.selectedModelId.value, 'manual-model')
})

test('useChatModelSelection: model label does not borrow an adapter from another model', async () => {
  const { selection } = await createTestHarness({ selectedSessionId: 'session-model-label' })

  selection.chooseModelSlug('manual-provider//manual-model')

  assert.equal(selection.modelChipLabel.value, 'manual-provider/manual-model')
})

test('useChatModelSelection: watch selectedSessionRunConfig.at triggers session apply', async () => {
  const { chat, selection } = await createTestHarness({
    selectedSessionId: 'session-watch-run-config',
    selectedSessionRunConfig: {
      providerID: 'run-provider-1',
      modelID: 'run-model-1',
      at: 1,
    },
  })

  await nextTick()
  assert.equal(selection.selectedProviderId.value, 'run-provider-1')
  assert.equal(selection.selectedModelId.value, 'run-model-1')

  if (chat.selectedSessionRunConfig) {
    chat.selectedSessionRunConfig.providerID = 'run-provider-2'
    chat.selectedSessionRunConfig.modelID = 'run-model-2'
    chat.selectedSessionRunConfig.at = 2
  }

  await nextTick()
  assert.equal(selection.selectedProviderId.value, 'run-provider-2')
  assert.equal(selection.selectedModelId.value, 'run-model-2')
})

test('useChatModelSelection: watch messages.length triggers derived session apply', async () => {
  const { chat, selection } = await createTestHarness({
    selectedSessionId: 'session-watch-messages',
    selectedSessionRunConfig: { at: 1 },
  })

  await nextTick()
  assert.equal(selection.selectedProviderId.value, '')
  assert.equal(selection.selectedModelId.value, '')

  chat.messages.push({
    info: {
      providerID: 'msg-provider-1',
      modelID: 'msg-model-1',
    },
  })

  await nextTick()
  assert.equal(selection.selectedProviderId.value, 'msg-provider-1')
  assert.equal(selection.selectedModelId.value, 'msg-model-1')

  chat.messages.push({
    info: {
      providerID: 'msg-provider-2',
      modelID: 'msg-model-2',
    },
  })

  await nextTick()
  assert.equal(selection.selectedProviderId.value, 'msg-provider-2')
  assert.equal(selection.selectedModelId.value, 'msg-model-2')
})

test('useChatModelSelection: an explicit session model exposes speed display names for overrides', async () => {
  const { chat, selection } = await createTestHarness({
    selectedSessionId: 'session-speed-selection',
    selectedSessionRunConfig: {
      providerID: 'cpa',
      adapterID: 'openai_responses',
      modelID: 'gpt-5.6-luna',
      at: 1,
    },
  })
  selection.providers.value = [
    {
      id: 'cpa',
      name: 'CPA',
      models: [
        {
          provider_id: 'cpa',
          adapter_id: 'openai_responses',
          id: 'gpt-5.6-luna',
          speed_modes: {
            fast: { display_name: 'Fast' },
            pro: { display_name: 'Pro' },
          },
        },
      ],
    },
  ]
  selection.applySessionSelection()

  assert.equal(selection.selectedSpeedMode.value, '')
  assert.equal(selection.speedModeChipLabel.value, '')

  selection.chooseSpeedMode('fast')
  assert.equal(selection.speedModeChipLabel.value, 'Fast')
})

test('useChatModelSelection: a session run config cached before the session existed is ignored', async () => {
  const createdMs = Date.parse('2026-10-19T13:43:26Z')
  const { selection } = await createTestHarness({
    selectedSessionId: '9',
    selectedSessionCreatedMs: createdMs,
    // Entry written by an older install whose session ids restarted at 1.
    selectedSessionRunConfig: {
      providerID: 'cpa',
      adapterID: 'anthropic',
      modelID: 'glm-5.3-flash',
      at: Date.parse('2026-08-30T06:45:00Z'),
    },
  })
  selection.providers.value = CPA_PROVIDERS
  selection.runtimeDefaultSelection.value = { ...CPA_DEFAULT_SELECTION }

  selection.applySessionSelection()

  assert.equal(selection.selectedProviderId.value, 'cpa')
  assert.equal(selection.selectedAdapterId.value, 'openai_responses')
  assert.equal(selection.selectedModelId.value, 'deepseek-v4.1-flash')
  assert.equal(selection.modelSource.value, 'default')
})

test('useChatModelSelection: a fresh session model the catalog no longer offers falls back to the default', async () => {
  const createdMs = Date.parse('2026-10-19T13:43:26Z')
  const { selection } = await createTestHarness({
    selectedSessionId: 'session-retired-model',
    selectedSessionCreatedMs: createdMs,
    selectedSessionRunConfig: {
      providerID: 'cpa',
      adapterID: 'anthropic',
      modelID: 'glm-5.3-flash',
      at: createdMs + 1_000,
    },
  })
  selection.providers.value = CPA_PROVIDERS
  selection.runtimeDefaultSelection.value = { ...CPA_DEFAULT_SELECTION }

  selection.applySessionSelection()

  assert.equal(selection.selectedAdapterId.value, 'openai_responses')
  assert.equal(selection.selectedModelId.value, 'deepseek-v4.1-flash')
  assert.equal(selection.modelSource.value, 'default')
})

test('useChatModelSelection: a session model cached for another adapter is ignored', async () => {
  const createdMs = Date.parse('2026-10-19T13:43:26Z')
  const { selection } = await createTestHarness({
    selectedSessionId: 'session-adapter-mismatch',
    selectedSessionCreatedMs: createdMs,
    // Same model id, but cached through an adapter that is not configured.
    selectedSessionRunConfig: {
      providerID: 'cpa',
      adapterID: 'anthropic',
      modelID: 'deepseek-v4.1-flash',
      at: createdMs + 1_000,
    },
  })
  selection.providers.value = CPA_PROVIDERS
  selection.runtimeDefaultSelection.value = { ...CPA_DEFAULT_SELECTION }

  selection.applySessionSelection()

  assert.equal(selection.selectedAdapterId.value, 'openai_responses')
  assert.equal(selection.selectedModelId.value, 'deepseek-v4.1-flash')
  assert.equal(selection.modelSource.value, 'default')
})

test('useChatModelSelection: a fresh session model the catalog offers still wins over the default', async () => {
  const createdMs = Date.parse('2026-10-19T13:43:26Z')
  const { selection } = await createTestHarness({
    selectedSessionId: 'session-fresh-model',
    selectedSessionCreatedMs: createdMs,
    selectedSessionRunConfig: {
      providerID: 'cpa',
      adapterID: 'openai_responses',
      modelID: 'gpt-5.5',
      at: createdMs + 1_000,
    },
  })
  selection.providers.value = CPA_PROVIDERS
  selection.runtimeDefaultSelection.value = { ...CPA_DEFAULT_SELECTION }

  selection.applySessionSelection()

  assert.equal(selection.selectedModelId.value, 'gpt-5.5')
  assert.equal(selection.modelSource.value, 'session')
})

test('useChatModelSelection: a manual model cached before the session existed is dropped', async () => {
  const createdMs = Date.parse('2026-10-19T13:43:26Z')
  const { selection } = await createTestHarness({
    selectedSessionId: '9',
    selectedSessionCreatedMs: createdMs,
    storedManualModels: {
      '9': JSON.stringify({ at: Date.parse('2026-08-30T06:45:00Z'), value: 'cpa/anthropic/glm-5.3-flash' }),
    },
  })
  selection.providers.value = CPA_PROVIDERS
  selection.runtimeDefaultSelection.value = { ...CPA_DEFAULT_SELECTION }

  selection.applySessionSelection()

  assert.equal(selection.selectedModelId.value, 'deepseek-v4.1-flash')
  assert.equal(selection.modelSource.value, 'default')

  await new Promise((resolve) => setTimeout(resolve, 400))
  const stored = JSON.parse(localStorage.getItem(localStorageKeys.chat.sessionManualModelBySession) || '{}')
  assert.equal(stored['9'], undefined)
})
