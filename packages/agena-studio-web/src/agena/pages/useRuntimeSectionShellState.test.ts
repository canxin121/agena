import { describe, expect, test } from 'bun:test'
import { computed, ref } from 'vue'

import { createRuntimeSectionShellState, useRuntimeSectionShellState } from './useRuntimeSectionShellState'

describe('useRuntimeSectionShellState', () => {
  test('assembles runtime shell state from provided runtime source', () => {
    const shell = createRuntimeSectionShellState({
      activeTab: ref('overview'),
      triggerReload: async () => {},
      visibleTabs: computed(() => [{ id: 'overview', label: 'Overview' }]),
    })

    expect(shell.activeTab.value).toBe('overview')
    expect(shell.visibleTabs.value).toEqual([{ id: 'overview', label: 'Overview' }])
    expect(typeof shell.triggerReload).toBe('function')
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

    const result = useRuntimeSectionShellState(
      { route: route as never, router: router as never },
      {
        useRuntimeSectionState: (value) => {
          expect(value).toEqual({ route, router, section: 'runtime' })
          return {
            shared,
            state: {
              activeTab: ref('overview'),
              triggerReload: async () => {},
              visibleTabs: computed(() => [{ id: 'overview', label: 'Overview' }]),
            },
          }
        },
      },
    )

    expect(result.actionMessage).toBe(shared.actionMessage)
    expect(result.pageTitle).toBe(shared.pageTitle)
    expect(result.pageDescription).toBe(shared.pageDescription)
    expect(result.load).toBe(shared.load)
    expect(result.shell.activeTab.value).toBe('overview')
  })
})
