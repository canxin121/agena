import { describe, expect, test } from 'bun:test'
import { computed, reactive, ref } from 'vue'

import type {
  DesktopBackendStatus,
  DesktopConfig,
  DesktopRuntimeInfo,
  DesktopUpdateProgress,
} from '../../lib/desktopConfig'

import { useRuntimeDesktopActions } from './useRuntimeDesktopActions'

function sampleConfig(): DesktopConfig {
  return {
    autostart_on_boot: true,
    backend: {
      host: '127.0.0.1',
      port: 3210,
      cors_origins: [],
      cors_allow_all: false,
      backend_log_level: 'info',
      ui_cookie_samesite: 'lax',
      agena_config_path: '/repo/.agena/config.json',
      agena_mode: 'default',
      workspace_root: '/repo',
      database_path: '/repo/.agena/agena.db',
      database_url: null,
    },
  }
}

function sampleStatus(): DesktopBackendStatus {
  return {
    running: true,
    url: 'http://127.0.0.1:3210',
    last_error: null,
    last_error_info: null,
  }
}

function sampleRuntimeInfo(): DesktopRuntimeInfo {
  return {
    installerVersion: '1.0.0',
    installerTarget: 'linux-x64',
    installerChannel: 'main',
    installerType: 'appimage',
    installerManager: 'native',
  }
}

function sampleUpdate(running = false): DesktopUpdateProgress {
  return {
    running,
    kind: running ? 'service' : '',
    phase: running ? 'download' : '',
    message: running ? 'downloading' : '',
    downloadedBytes: running ? 5 : 0,
    totalBytes: running ? 10 : null,
    error: null,
  }
}

function createState(enabled = true) {
  const desktopEnabled = ref(enabled)
  const desktopStatus = ref<DesktopBackendStatus | null>(sampleStatus())
  const state = {
    actionError: ref(''),
    actionMessage: ref(''),
    desktopBackendUrl: computed(() => desktopStatus.value?.url?.trim() || ''),
    desktopConfig: ref<DesktopConfig | null>(sampleConfig()),
    desktopEnabled: computed(() => desktopEnabled.value),
    desktopForm: reactive({
      autostart_on_boot: false,
      host: '',
      port: '',
      workspace_root: '',
      agena_config_path: '',
      agena_mode: '',
      database_path: '',
      database_url: '',
      backend_log_level: '',
      ui_cookie_samesite: '',
    }),
    desktopInstallerAssetName: ref('installer.tgz'),
    desktopInstallerUpdateUrl: ref('https://example.com/installer.tgz'),
    desktopNotice: ref(''),
    desktopRuntimeState: ref<DesktopRuntimeInfo | null>(sampleRuntimeInfo()),
    desktopSaving: ref(false),
    desktopServiceUpdateUrl: ref('https://example.com/service.tgz'),
    desktopStatus,
    desktopUpdate: ref<DesktopUpdateProgress | null>(sampleUpdate(false)),
    desktopUpdateRunning: ref(false),
  }
  const calls: string[] = []
  let savedConfig: DesktopConfig | null = null
  const actions = useRuntimeDesktopActions(state, {
    desktopBackendRestart: async () => {
      calls.push('desktopBackendRestart')
    },
    desktopBackendStatus: async () => {
      calls.push('desktopBackendStatus')
      return sampleStatus()
    },
    desktopConfigGet: async () => {
      calls.push('desktopConfigGet')
      return sampleConfig()
    },
    desktopConfigSave: async (config) => {
      calls.push('desktopConfigSave')
      savedConfig = config
      return config
    },
    desktopInstallerUpdate: async (assetUrl, assetName) => {
      calls.push(`desktopInstallerUpdate:${assetUrl}:${assetName || ''}`)
    },
    desktopOpenConfig: async () => {
      calls.push('desktopOpenConfig')
    },
    desktopOpenExternal: async (url) => {
      calls.push(`desktopOpenExternal:${url}`)
    },
    desktopRuntimeInfo: async () => {
      calls.push('desktopRuntimeInfo')
      return sampleRuntimeInfo()
    },
    desktopServiceUpdate: async (assetUrl) => {
      calls.push(`desktopServiceUpdate:${assetUrl}`)
    },
    desktopUpdateProgressGet: async () => {
      calls.push('desktopUpdateProgressGet')
      return sampleUpdate(true)
    },
  })

  return { actions, calls, savedConfigRef: () => savedConfig, state, desktopEnabled }
}

describe('useRuntimeDesktopActions', () => {
  test('loadDesktopPanel resets state when desktop runtime is disabled', async () => {
    const { actions, state } = createState(false)

    await actions.loadDesktopPanel()

    expect(state.desktopConfig.value).toBe(null)
    expect(state.desktopStatus.value).toBe(null)
    expect(state.desktopRuntimeState.value).toBe(null)
    expect(state.desktopUpdate.value).toBe(null)
    expect(state.desktopForm.host).toBe('')
    expect(state.desktopNotice.value).toBe('')
  })

  test('loadDesktopPanel hydrates desktop state and syncs form', async () => {
    const { actions, calls, state } = createState(true)
    state.desktopConfig.value = null
    state.desktopStatus.value = null
    state.desktopRuntimeState.value = null
    state.desktopUpdate.value = null

    await actions.loadDesktopPanel()

    expect(calls).toEqual(['desktopConfigGet', 'desktopBackendStatus', 'desktopRuntimeInfo', 'desktopUpdateProgressGet'])
    const loadedConfig = state.desktopConfig.value as unknown as DesktopConfig
    const loadedUpdate = state.desktopUpdate.value as unknown as DesktopUpdateProgress
    expect(loadedConfig.backend.host).toBe('127.0.0.1')
    expect(state.desktopForm.host).toBe('127.0.0.1')
    expect(state.desktopForm.port).toBe('3210')
    expect(loadedUpdate.running).toBe(true)
  })

  test('saveDesktopConfigAction normalizes form values and reloads panel', async () => {
    const { actions, calls, savedConfigRef, state } = createState(true)
    state.desktopForm.autostart_on_boot = false
    state.desktopForm.host = ' 0.0.0.0 '
    state.desktopForm.port = '9999'
    state.desktopForm.workspace_root = ' '
    state.desktopForm.agena_config_path = '/tmp/config.json '
    state.desktopForm.agena_mode = ' fast '
    state.desktopForm.database_path = ''
    state.desktopForm.database_url = ' sqlite:///tmp/db '
    state.desktopForm.backend_log_level = ' debug '
    state.desktopForm.ui_cookie_samesite = ' strict '

    await actions.saveDesktopConfigAction()

    expect(calls).toEqual([
      'desktopConfigSave',
      'desktopConfigGet',
      'desktopBackendStatus',
      'desktopRuntimeInfo',
      'desktopUpdateProgressGet',
    ])
    const savedConfig = savedConfigRef()
    if (!savedConfig) throw new Error('expected saved desktop config')
    expect(savedConfig.autostart_on_boot).toBe(false)
    expect(savedConfig.backend.host).toBe('0.0.0.0')
    expect(savedConfig.backend.port).toBe(9999)
    expect(savedConfig.backend.workspace_root).toBe(null)
    expect(savedConfig.backend.agena_config_path).toBe('/tmp/config.json')
    expect(savedConfig.backend.agena_mode).toBe('fast')
    expect(savedConfig.backend.database_url).toBe('sqlite:///tmp/db')
    expect(savedConfig.backend.backend_log_level).toBe('debug')
    expect(savedConfig.backend.ui_cookie_samesite).toBe('strict')
    expect(state.actionMessage.value).toBe('Desktop settings saved.')
    expect(state.desktopForm.host).toBe('127.0.0.1')
    expect(state.desktopSaving.value).toBe(false)
  })

  test('desktop actions trigger expected side effects and notices', async () => {
    const { actions, calls, state } = createState(true)

    await actions.restartDesktopBackendAction()
    await actions.refreshDesktopUpdateProgressAction()
    await actions.runDesktopServiceUpdateAction()
    await actions.runDesktopInstallerUpdateAction()
    await actions.openDesktopConfigAction()
    await actions.openDesktopBackendUrlAction()

    expect(calls).toEqual([
      'desktopBackendRestart',
      'desktopConfigGet',
      'desktopBackendStatus',
      'desktopRuntimeInfo',
      'desktopUpdateProgressGet',
      'desktopUpdateProgressGet',
      'desktopServiceUpdate:https://example.com/service.tgz',
      'desktopConfigGet',
      'desktopBackendStatus',
      'desktopRuntimeInfo',
      'desktopUpdateProgressGet',
      'desktopInstallerUpdate:https://example.com/installer.tgz:installer.tgz',
      'desktopConfigGet',
      'desktopBackendStatus',
      'desktopRuntimeInfo',
      'desktopUpdateProgressGet',
      'desktopOpenConfig',
      'desktopOpenExternal:http://127.0.0.1:3210',
    ])
    expect(state.desktopUpdate.value?.running).toBe(true)
    expect(state.desktopNotice.value).toBe('Opened http://127.0.0.1:3210.')
    expect(state.actionMessage.value).toBe('Opened http://127.0.0.1:3210.')
    expect(state.desktopUpdateRunning.value).toBe(false)
  })
})
