import { describe, expect, test } from 'bun:test'
import { computed, ref } from 'vue'

import { useRuntimeSectionState } from './useRuntimeSectionState'

describe('useRuntimeSectionState', () => {
  test('forwards section into useRuntimePageState and exposes shared fields', () => {
    const sentinel = {
      actionError: ref('err'),
      actionMessage: ref('msg'),
      load: async () => {},
      loading: ref(true),
      pageDescription: computed(() => 'desc'),
      pageTitle: computed(() => 'title'),
    }
    let calledWith: unknown = null
    const route = { path: '/runtime' }
    const router = { push: async () => {}, replace: async () => {} }
    const result = useRuntimeSectionState(
      {
        route: route as never,
        router: router as never,
        section: 'runtime',
      },
      {
        useRuntimePageState: (value) => {
          calledWith = value
          return sentinel
        },
      },
    )

    expect(calledWith).toEqual({ route, router, section: 'runtime' })
    expect(result.state).toBe(sentinel)
    expect(result.shared.actionError).toBe(sentinel.actionError)
    expect(result.shared.actionMessage).toBe(sentinel.actionMessage)
    expect(result.shared.load).toBe(sentinel.load)
    expect(result.shared.loading).toBe(sentinel.loading)
    expect(result.shared.pageDescription).toBe(sentinel.pageDescription)
    expect(result.shared.pageTitle).toBe(sentinel.pageTitle)
  })
})
