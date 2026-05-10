import { describe, expect, test } from 'bun:test'
import { computed, ref } from 'vue'

import { createRuntimeWorkflowPanelState, useRuntimeWorkflowPageState } from './useRuntimeWorkflowPageState'

describe('useRuntimeWorkflowPageState', () => {
  test('assembles workflow panel state from provided runtime source', () => {
    const workflow = createRuntimeWorkflowPanelState({
      approvePermission: async () => {},
      executionFacts: computed(() => [{ label: 'state', value: 'idle' }]),
      openSelectedSessionInChat: () => {},
      selectSession: async () => {},
      selectWorkspace: async () => {},
      selectedSessionId: ref(7),
      selectedWorkspaceId: ref(3),
      sessionExecution: ref(null),
      sessions: ref([]),
      timelineSummaries: computed(() => [
        { key: 'timeline-1', kind: 'step', summary: 'step', timestamp: '2026-05-11T00:00:00Z', sessionId: '7' },
      ]),
      workflowLoading: ref(false),
      workspaces: ref([]),
    })

    expect(workflow.selectedSessionId.value).toBe(7)
    expect(workflow.selectedWorkspaceId.value).toBe(3)
    expect(workflow.executionFacts.value[0]?.label).toBe('state')
    expect(workflow.timelineSummaries.value[0]?.summary).toBe('step')
  })

  test('exposes shared shell fields via injected section state', () => {
    const route = { path: '/runtime/workflow' }
    const router = { push: async () => {}, replace: async () => {} }
    const shared = {
      actionError: ref(''),
      actionMessage: ref('ok'),
      load: async () => {},
      loading: ref(false),
      pageDescription: computed(() => 'desc'),
      pageTitle: computed(() => 'title'),
    }

    const result = useRuntimeWorkflowPageState(
      { route: route as never, router: router as never },
      {
        useRuntimeSectionState: (value) => {
          expect(value).toEqual({ route, router, section: 'runtime' })
          return {
            shared,
            state: {
              approvePermission: async () => {},
              executionFacts: computed(() => []),
              openSelectedSessionInChat: () => {},
              selectSession: async () => {},
              selectWorkspace: async () => {},
              selectedSessionId: ref(null),
              selectedWorkspaceId: ref(null),
              sessionExecution: ref(null),
              sessions: ref([]),
              timelineSummaries: computed(() => []),
              workflowLoading: ref(false),
              workspaces: ref([]),
            },
          }
        },
      },
    )

    expect(result.actionMessage).toBe(shared.actionMessage)
    expect(result.pageTitle).toBe(shared.pageTitle)
    expect(result.pageDescription).toBe(shared.pageDescription)
    expect(result.load).toBe(shared.load)
    expect(result.workflow.timelineSummaries.value).toEqual([])
  })
})
