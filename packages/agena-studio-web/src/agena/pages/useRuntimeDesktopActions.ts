import type { ComputedRef, Ref } from 'vue'

import {
  desktopBackendRestart,
  desktopBackendStatus,
  desktopConfigGet,
  desktopConfigSave,
  desktopInstallerUpdate,
  desktopOpenConfig,
  desktopOpenExternal,
  desktopRuntimeInfo,
  desktopServiceUpdate,
  desktopUpdateProgressGet,
  type DesktopBackendStatus,
  type DesktopConfig,
  type DesktopRuntimeInfo,
  type DesktopUpdateProgress,
} from '../../lib/desktopConfig'
import { normalizeOptionalText, normalizePort } from './runtimePageStateModel'

export type RuntimeDesktopFormState = {
  autostart_on_boot: boolean
  host: string
  port: string
  workspace_root: string
  agena_config_path: string
  database_path: string
  database_url: string
  backend_log_level: string
  ui_cookie_samesite: string
}

export type RuntimeDesktopActionsInput = {
  actionError: Ref<string>
  actionMessage: Ref<string>
  desktopBackendUrl: ComputedRef<string>
  desktopConfig: Ref<DesktopConfig | null>
  desktopEnabled: ComputedRef<boolean>
  desktopForm: RuntimeDesktopFormState
  desktopInstallerAssetName: Ref<string>
  desktopInstallerUpdateUrl: Ref<string>
  desktopNotice: Ref<string>
  desktopRuntimeState: Ref<DesktopRuntimeInfo | null>
  desktopSaving: Ref<boolean>
  desktopServiceUpdateUrl: Ref<string>
  desktopStatus: Ref<DesktopBackendStatus | null>
  desktopUpdate: Ref<DesktopUpdateProgress | null>
  desktopUpdateRunning: Ref<boolean>
}

export type RuntimeDesktopActionsDeps = {
  desktopBackendRestart: typeof desktopBackendRestart
  desktopBackendStatus: typeof desktopBackendStatus
  desktopConfigGet: typeof desktopConfigGet
  desktopConfigSave: typeof desktopConfigSave
  desktopInstallerUpdate: typeof desktopInstallerUpdate
  desktopOpenConfig: typeof desktopOpenConfig
  desktopOpenExternal: typeof desktopOpenExternal
  desktopRuntimeInfo: typeof desktopRuntimeInfo
  desktopServiceUpdate: typeof desktopServiceUpdate
  desktopUpdateProgressGet: typeof desktopUpdateProgressGet
}

const defaultDeps: RuntimeDesktopActionsDeps = {
  desktopBackendRestart,
  desktopBackendStatus,
  desktopConfigGet,
  desktopConfigSave,
  desktopInstallerUpdate,
  desktopOpenConfig,
  desktopOpenExternal,
  desktopRuntimeInfo,
  desktopServiceUpdate,
  desktopUpdateProgressGet,
}

export function useRuntimeDesktopActions(
  input: RuntimeDesktopActionsInput,
  deps: RuntimeDesktopActionsDeps = defaultDeps,
) {
  function syncDesktopForm(config: DesktopConfig | null) {
    input.desktopForm.autostart_on_boot = config?.autostart_on_boot ?? false
    input.desktopForm.host = config?.backend.host || ''
    input.desktopForm.port = config ? String(config.backend.port) : ''
    input.desktopForm.workspace_root = config?.backend.workspace_root || ''
    input.desktopForm.agena_config_path = config?.backend.agena_config_path || ''
    input.desktopForm.database_path = config?.backend.database_path || ''
    input.desktopForm.database_url = config?.backend.database_url || ''
    input.desktopForm.backend_log_level = config?.backend.backend_log_level || ''
    input.desktopForm.ui_cookie_samesite = config?.backend.ui_cookie_samesite || ''
  }

  function resetDesktopState() {
    input.desktopConfig.value = null
    input.desktopStatus.value = null
    input.desktopRuntimeState.value = null
    input.desktopUpdate.value = null
    input.desktopNotice.value = ''
    syncDesktopForm(null)
  }

  async function loadDesktopPanel() {
    if (!input.desktopEnabled.value) {
      resetDesktopState()
      return
    }
    input.desktopNotice.value = ''
    try {
      const [config, status, runtimeInfo, update] = await Promise.all([
        deps.desktopConfigGet().catch(() => null),
        deps.desktopBackendStatus().catch(() => null),
        deps.desktopRuntimeInfo().catch(() => null),
        deps.desktopUpdateProgressGet().catch(() => null),
      ])
      input.desktopConfig.value = config
      input.desktopStatus.value = status
      input.desktopRuntimeState.value = runtimeInfo
      input.desktopUpdate.value = update
      syncDesktopForm(config)
    } catch (err) {
      input.actionError.value = err instanceof Error ? err.message : String(err)
    }
  }

  async function saveDesktopConfigAction() {
    if (!input.desktopEnabled.value || !input.desktopConfig.value) return
    input.desktopSaving.value = true
    input.desktopNotice.value = ''
    input.actionError.value = ''
    input.actionMessage.value = ''
    try {
      const nextConfig: DesktopConfig = {
        ...input.desktopConfig.value,
        autostart_on_boot: input.desktopForm.autostart_on_boot,
        backend: {
          ...input.desktopConfig.value.backend,
          host: input.desktopForm.host.trim() || input.desktopConfig.value.backend.host,
          port: normalizePort(input.desktopForm.port, input.desktopConfig.value.backend.port),
          workspace_root: normalizeOptionalText(input.desktopForm.workspace_root),
          agena_config_path: normalizeOptionalText(input.desktopForm.agena_config_path),
          database_path: normalizeOptionalText(input.desktopForm.database_path),
          database_url: normalizeOptionalText(input.desktopForm.database_url),
          backend_log_level: normalizeOptionalText(input.desktopForm.backend_log_level),
          ui_cookie_samesite: normalizeOptionalText(input.desktopForm.ui_cookie_samesite),
        },
      }
      const saved = await deps.desktopConfigSave(nextConfig)
      input.desktopConfig.value = saved || nextConfig
      syncDesktopForm(input.desktopConfig.value)
      input.desktopNotice.value = 'Desktop settings saved.'
      input.actionMessage.value = 'Desktop settings saved.'
      await loadDesktopPanel()
    } catch (err) {
      input.actionError.value = err instanceof Error ? err.message : String(err)
    } finally {
      input.desktopSaving.value = false
    }
  }

  async function restartDesktopBackendAction() {
    if (!input.desktopEnabled.value) return
    input.desktopNotice.value = ''
    input.actionError.value = ''
    input.actionMessage.value = ''
    try {
      await deps.desktopBackendRestart()
      await loadDesktopPanel()
      input.actionMessage.value = 'Requested desktop backend restart.'
      input.desktopNotice.value = 'Requested desktop backend restart.'
    } catch (err) {
      input.actionError.value = err instanceof Error ? err.message : String(err)
    }
  }

  async function refreshDesktopUpdateProgressAction() {
    if (!input.desktopEnabled.value) return
    input.desktopNotice.value = ''
    input.actionError.value = ''
    try {
      input.desktopUpdate.value = await deps.desktopUpdateProgressGet()
      if (input.desktopUpdate.value?.running) {
        input.desktopNotice.value = 'Desktop update is still running.'
      }
    } catch (err) {
      input.actionError.value = err instanceof Error ? err.message : String(err)
    }
  }

  async function runDesktopServiceUpdateAction() {
    const assetUrl = input.desktopServiceUpdateUrl.value.trim()
    if (!input.desktopEnabled.value || !assetUrl) return
    input.desktopUpdateRunning.value = true
    input.desktopNotice.value = ''
    input.actionError.value = ''
    input.actionMessage.value = ''
    try {
      await deps.desktopServiceUpdate(assetUrl)
      input.actionMessage.value = 'Requested desktop service update.'
      input.desktopNotice.value = 'Requested desktop service update.'
      await loadDesktopPanel()
    } catch (err) {
      input.actionError.value = err instanceof Error ? err.message : String(err)
    } finally {
      input.desktopUpdateRunning.value = false
    }
  }

  async function runDesktopInstallerUpdateAction() {
    const assetUrl = input.desktopInstallerUpdateUrl.value.trim()
    const assetName = input.desktopInstallerAssetName.value.trim() || undefined
    if (!input.desktopEnabled.value || !assetUrl) return
    input.desktopUpdateRunning.value = true
    input.desktopNotice.value = ''
    input.actionError.value = ''
    input.actionMessage.value = ''
    try {
      await deps.desktopInstallerUpdate(assetUrl, assetName)
      input.actionMessage.value = 'Requested desktop installer update.'
      input.desktopNotice.value = 'Requested desktop installer update.'
      await loadDesktopPanel()
    } catch (err) {
      input.actionError.value = err instanceof Error ? err.message : String(err)
    } finally {
      input.desktopUpdateRunning.value = false
    }
  }

  async function openDesktopConfigAction() {
    if (!input.desktopEnabled.value) return
    input.desktopNotice.value = ''
    input.actionError.value = ''
    input.actionMessage.value = ''
    try {
      await deps.desktopOpenConfig()
      input.actionMessage.value = 'Opened desktop config in the host runtime.'
      input.desktopNotice.value = 'Opened desktop config in the host runtime.'
    } catch (err) {
      input.actionError.value = err instanceof Error ? err.message : String(err)
    }
  }

  async function openDesktopBackendUrlAction() {
    if (!input.desktopEnabled.value || !input.desktopBackendUrl.value) return
    input.desktopNotice.value = ''
    input.actionError.value = ''
    input.actionMessage.value = ''
    try {
      await deps.desktopOpenExternal(input.desktopBackendUrl.value)
      input.actionMessage.value = `Opened ${input.desktopBackendUrl.value}.`
      input.desktopNotice.value = `Opened ${input.desktopBackendUrl.value}.`
    } catch (err) {
      input.actionError.value = err instanceof Error ? err.message : String(err)
    }
  }

  return {
    loadDesktopPanel,
    openDesktopBackendUrlAction,
    openDesktopConfigAction,
    refreshDesktopUpdateProgressAction,
    resetDesktopState,
    restartDesktopBackendAction,
    runDesktopInstallerUpdateAction,
    runDesktopServiceUpdateAction,
    saveDesktopConfigAction,
    syncDesktopForm,
  }
}
