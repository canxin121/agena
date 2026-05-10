import { describe, expect, test } from 'bun:test'
import { computed, ref } from 'vue'

import { createSettingsProvidersPanelState, useSettingsProvidersPageState } from './useSettingsProvidersPageState'

describe('useSettingsProvidersPageState', () => {
  test('assembles providers panel state from provided settings source', () => {
    const providers = createSettingsProvidersPanelState({
      authProviders: ref([]),
      browserAuthCodeDrafts: {},
      browserAuthInstanceDrafts: {},
      browserAuthStartState: {},
      deviceAuthEnterpriseDrafts: {},
      deviceAuthStartState: {},
      drafts: { anthropic: 'sk-test' },
      finishBrowserAuth: async () => {},
      pollDeviceAuth: async () => {},
      saveApiKey: async () => {},
      refreshCredential: async () => {},
      clearCredential: async () => {},
      startBrowserAuth: async () => {},
      startDeviceAuth: async () => {},
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
              browserAuthCodeDrafts: {},
              browserAuthInstanceDrafts: {},
              browserAuthStartState: {},
              deviceAuthEnterpriseDrafts: {},
              deviceAuthStartState: {},
              drafts: {},
              finishBrowserAuth: async () => {},
              pollDeviceAuth: async () => {},
              saveApiKey: async () => {},
              refreshCredential: async () => {},
              clearCredential: async () => {},
              startBrowserAuth: async () => {},
              startDeviceAuth: async () => {},
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
