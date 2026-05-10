import { describe, expect, test } from 'bun:test'
import { computed, ref } from 'vue'

import { createSettingsSectionShellState, useSettingsSectionShellState } from './useSettingsSectionShellState'

describe('useSettingsSectionShellState', () => {
  test('assembles settings shell state from provided runtime source', () => {
    const shell = createSettingsSectionShellState({
      activeSettingsTab: ref('providers'),
      visibleTabs: computed(() => [{ id: 'providers', label: 'Providers' }]),
    })

    expect(shell.activeTab.value).toBe('providers')
    expect(shell.tabs.value[0]?.label).toBe('Providers')
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

    const result = useSettingsSectionShellState(
      { route: route as never, router: router as never },
      {
        useRuntimeSectionState: (value) => {
          expect(value).toEqual({ route, router, section: 'settings' })
          return {
            shared,
            state: {
              activeSettingsTab: ref('providers'),
              visibleTabs: computed(() => [{ id: 'providers', label: 'Providers' }]),
            },
          }
        },
      },
    )

    expect(result.actionMessage).toBe(shared.actionMessage)
    expect(result.pageTitle).toBe(shared.pageTitle)
    expect(result.pageDescription).toBe(shared.pageDescription)
    expect(result.load).toBe(shared.load)
    expect(result.shell.activeTab.value).toBe('providers')
  })
})
