import { describe, expect, test } from 'bun:test'
import { computed, ref } from 'vue'

import { createRuntimeOverviewPanelState, useRuntimeOverviewPageState } from './useRuntimeOverviewPageState'

describe('useRuntimeOverviewPageState', () => {
  test('assembles overview panel state from provided runtime source', () => {
    const overview = createRuntimeOverviewPanelState({
      operatorCards: computed(() => [{ label: 'Providers', value: '2' }]),
      providerModels: { anthropic: [] },
      providers: ref([]),
      runtime: ref(null),
      runtimeSnapshotFacts: computed(() => [{ label: 'Generation', value: '1' }]),
      sessionCacheFacts: computed(() => [{ label: 'Entries', value: '3' }]),
    })

    expect(overview.operatorCards.value[0]?.label).toBe('Providers')
    expect(overview.runtimeSnapshotFacts.value[0]?.value).toBe('1')
    expect(overview.sessionCacheFacts.value[0]?.value).toBe('3')
    expect(overview.providerModels.anthropic).toEqual([])
  })

  test('exposes shared shell fields via injected section state', () => {
    const route = { path: '/runtime/overview' }
    const router = { push: async () => {}, replace: async () => {} }
    const shared = {
      actionError: ref(''),
      actionMessage: ref('ok'),
      load: async () => {},
      loading: ref(false),
      pageDescription: computed(() => 'desc'),
      pageTitle: computed(() => 'title'),
    }

    const result = useRuntimeOverviewPageState(
      { route: route as never, router: router as never },
      {
        useRuntimeSectionState: (value) => {
          expect(value).toEqual({ route, router, section: 'runtime' })
          return {
            shared,
            state: {
              operatorCards: computed(() => []),
              providerModels: {},
              providers: ref([]),
              runtime: ref(null),
              runtimeSnapshotFacts: computed(() => []),
              sessionCacheFacts: computed(() => []),
            },
          }
        },
      },
    )

    expect(result.actionMessage).toBe(shared.actionMessage)
    expect(result.pageTitle).toBe(shared.pageTitle)
    expect(result.pageDescription).toBe(shared.pageDescription)
    expect(result.load).toBe(shared.load)
    expect(result.overview.operatorCards.value).toEqual([])
  })
})
