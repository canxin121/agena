import { describe, expect, test } from 'bun:test'

import { useChatPageState } from './useChatPageState'

describe('useChatPageState', () => {
  test('creates empty chat page state with expected defaults', () => {
    const state = useChatPageState()

    expect(state.runtime.value).toBe(null)
    expect(state.providers.value).toEqual([])
    expect(state.messages.value).toEqual([])
    expect(state.timelineEvents.value).toEqual([])
    expect(state.inspectedMessage.value).toBe(null)
    expect(state.inspectedMessageParts.value).toEqual([])
    expect(state.inspectedPart.value).toBe(null)
    expect(state.sessionState.value).toBe(null)
    expect(state.selectedWorkspaceId.value).toBe(null)
    expect(state.selectedSessionId.value).toBe(null)
    expect(state.workspacePath.value).toBe('')
    expect(state.sessionSearch.value).toBe('')
    expect(state.newSessionTitle.value).toBe('')
    expect(state.composer.value).toBe('')
    expect(state.selectedProviderId.value).toBe('')
    expect(state.selectedModelId.value).toBe('')
    expect(state.loading.value).toBe(false)
    expect(state.sending.value).toBe(false)
    expect(state.continuing.value).toBe(false)
    expect(state.errorMessage.value).toBe('')
    expect(state.localCommandNotice.value).toBe('')
    expect(state.sessionImportJsonl.value).toBe('')
    expect(state.providerModels).toEqual({})
    expect(state.userInputDrafts).toEqual({})
  })
})
