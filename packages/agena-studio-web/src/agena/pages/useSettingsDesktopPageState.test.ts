import { describe, expect, test } from 'bun:test'
import { computed, ref } from 'vue'

import { createSettingsDesktopPanelState, useSettingsDesktopPageState } from './useSettingsDesktopPageState'

describe('useSettingsDesktopPageState', () => {
  test('assembles desktop panel state from provided settings source', () => {
    const desktop = createSettingsDesktopPanelState({
      backendErrorFacts: computed(() => []),
      backendUrl: computed(() => 'http://127.0.0.1:4317'),
      config: ref(null),
      configFacts: computed(() => []),
      enabled: computed(() => true),
      form: {
        autostart_on_boot: false,
        host: '127.0.0.1',
        port: '4317',
        workspace_root: '',
        agena_config_path: '',
        database_path: '',
        database_url: '',
        backend_log_level: 'info',
        ui_cookie_samesite: 'lax',
      },
      installerAssetName: ref(''),
      installerUpdateUrl: ref(''),
      notice: ref('ok'),
      runtimeFacts: computed(() => []),
      saving: ref(false),
      serviceUpdateUrl: ref(''),
      statusFacts: computed(() => []),
      updateFacts: computed(() => []),
      updateProgressPercent: computed(() => '0%'),
      updateRunning: ref(false),
      loadPanel: async () => {},
      openBackendUrlAction: async () => {},
      openConfigAction: async () => {},
      refreshUpdateProgressAction: async () => {},
      restartBackendAction: async () => {},
      runInstallerUpdateAction: async () => {},
      runServiceUpdateAction: async () => {},
      saveConfigAction: async () => {},
    })

    expect(desktop.backendUrl.value).toBe('http://127.0.0.1:4317')
    expect(desktop.enabled.value).toBe(true)
    expect(desktop.notice.value).toBe('ok')
  })

  test('exposes shared shell fields via injected section state', () => {
    const route = { path: '/settings/desktop' }
    const router = { push: async () => {}, replace: async () => {} }
    const shared = {
      actionError: ref(''),
      actionMessage: ref('ok'),
      load: async () => {},
      loading: ref(false),
      pageDescription: computed(() => 'desc'),
      pageTitle: computed(() => 'title'),
    }

    const result = useSettingsDesktopPageState(
      { route: route as never, router: router as never },
      {
        useRuntimeSectionState: (value) => {
          expect(value).toEqual({ route, router, section: 'settings' })
          return {
            shared,
            state: {
              backendErrorFacts: computed(() => []),
              backendUrl: computed(() => 'http://127.0.0.1:4317'),
              config: ref(null),
              configFacts: computed(() => []),
              enabled: computed(() => true),
              form: {
                autostart_on_boot: false,
                host: '127.0.0.1',
                port: '4317',
                workspace_root: '',
                agena_config_path: '',
                database_path: '',
                database_url: '',
                backend_log_level: 'info',
                ui_cookie_samesite: 'lax',
              },
              installerAssetName: ref(''),
              installerUpdateUrl: ref(''),
              notice: ref('ok'),
              runtimeFacts: computed(() => []),
              saving: ref(false),
              serviceUpdateUrl: ref(''),
              statusFacts: computed(() => []),
              updateFacts: computed(() => []),
              updateProgressPercent: computed(() => '0%'),
              updateRunning: ref(false),
              loadPanel: async () => {},
              openBackendUrlAction: async () => {},
              openConfigAction: async () => {},
              refreshUpdateProgressAction: async () => {},
              restartBackendAction: async () => {},
              runInstallerUpdateAction: async () => {},
              runServiceUpdateAction: async () => {},
              saveConfigAction: async () => {},
            },
          }
        },
      },
    )

    expect(result.actionMessage).toBe(shared.actionMessage)
    expect(result.pageTitle).toBe(shared.pageTitle)
    expect(result.pageDescription).toBe(shared.pageDescription)
    expect(result.load).toBe(shared.load)
    expect(result.desktop.backendUrl.value).toBe('http://127.0.0.1:4317')
  })
})
