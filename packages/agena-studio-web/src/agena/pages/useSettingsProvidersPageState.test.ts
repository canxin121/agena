import { describe, expect, test } from 'bun:test'
import { computed, ref } from 'vue'

import { createSettingsProvidersPanelState, useSettingsProvidersPageState } from './useSettingsProvidersPageState'

describe('useSettingsProvidersPageState', () => {
  test('assembles providers panel state from provided settings source', () => {
    const providers = createSettingsProvidersPanelState({
      authProviders: ref([]),
      drafts: { anthropic: 'sk-test' },
      saveApiKey: async () => {},
      refreshCredential: async () => {},
      clearCredential: async () => {},
    })

    expect(providers.authProviders.value).toEqual([])
    expect(providers.drafts.anthropic).toBe('sk-test')
  })

  test('exposes shared shell fields via injected section state', () => {
    const route = { path: '/settings/providers' }
    const router = { push: async () => {}, replace: async () => {} }
    const shared = {
      actionError: ref(''),
      actionMessage: ref('ok'),
      load: async () => {},
      loading: ref(false),
      pageDescription: computed(() => 'desc'),
      pageTitle: computed(() => 'title'),
    }

    const result = useSettingsProvidersPageState(
      { route: route as never, router: router as never },
      {
        useRuntimeSectionState: (value) => {
          expect(value).toEqual({ route, router, section: 'settings' })
          return {
            shared,
            state: {
              authProviders: ref([]),
              drafts: {},
              saveApiKey: async () => {},
              refreshCredential: async () => {},
              clearCredential: async () => {},
            },
          }
        },
      },
    )

    expect(result.actionMessage).toBe(shared.actionMessage)
    expect(result.pageTitle).toBe(shared.pageTitle)
    expect(result.pageDescription).toBe(shared.pageDescription)
    expect(result.load).toBe(shared.load)
    expect(result.providers.authProviders.value).toEqual([])
  })
})
