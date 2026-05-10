import { describe, expect, test } from 'bun:test'
import { computed, ref } from 'vue'

import { createRuntimeOperatorPanelState, useRuntimeOperatorPageState } from './useRuntimeOperatorPageState'

describe('useRuntimeOperatorPageState', () => {
  test('assembles operator panel state from provided runtime source', () => {
    const runtime = ref(null)
    const operator = createRuntimeOperatorPanelState({ runtime })

    expect(operator.runtime).toBe(runtime)
    expect(operator.runtime.value).toBe(null)
  })

  test('exposes shared shell fields via injected section state', () => {
    const route = { path: '/runtime/operator' }
    const router = { push: async () => {}, replace: async () => {} }
    const shared = {
      actionError: ref(''),
      actionMessage: ref('ok'),
      load: async () => {},
      loading: ref(false),
      pageDescription: computed(() => 'desc'),
      pageTitle: computed(() => 'title'),
    }

    const result = useRuntimeOperatorPageState(
      { route: route as never, router: router as never },
      {
        useRuntimeSectionState: (value) => {
          expect(value).toEqual({ route, router, section: 'runtime' })
          return {
            shared,
            state: {
              runtime: ref(null),
            },
          }
        },
      },
    )

    expect(result.actionMessage).toBe(shared.actionMessage)
    expect(result.pageTitle).toBe(shared.pageTitle)
    expect(result.pageDescription).toBe(shared.pageDescription)
    expect(result.load).toBe(shared.load)
    expect(result.operator.runtime.value).toBe(null)
  })
})
