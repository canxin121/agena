import type { ComputedRef, Ref } from 'vue'

import type {
  DesktopBackendStatus,
  DesktopConfig,
  DesktopRuntimeInfo,
  DesktopUpdateProgress,
} from '../../lib/desktopConfig'

export type DesktopFact = { label: string; value: string; mono?: boolean }

export type SettingsDesktopStateInput = {
  backendErrorFacts: ComputedRef<DesktopFact[]>
  backendUrl: ComputedRef<string>
  config: Ref<DesktopConfig | null>
  configFacts: ComputedRef<DesktopFact[]>
  enabled: ComputedRef<boolean>
  form: {
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
  installerAssetName: Ref<string>
  installerUpdateUrl: Ref<string>
  notice: Ref<string>
  runtimeFacts: ComputedRef<DesktopFact[]>
  runtimeState?: Ref<DesktopRuntimeInfo | null>
  saving: Ref<boolean>
  serviceUpdateUrl: Ref<string>
  statusFacts: ComputedRef<DesktopFact[]>
  statusState?: Ref<DesktopBackendStatus | null>
  updateFacts: ComputedRef<DesktopFact[]>
  updateProgressPercent: ComputedRef<string>
  updateRunning: Ref<boolean>
  updateState?: Ref<DesktopUpdateProgress | null>
  loadPanel: () => void | Promise<void>
  openBackendUrlAction: () => void | Promise<void>
  openConfigAction: () => void | Promise<void>
  refreshUpdateProgressAction: () => void | Promise<void>
  restartBackendAction: () => void | Promise<void>
  runInstallerUpdateAction: () => void | Promise<void>
  runServiceUpdateAction: () => void | Promise<void>
  saveConfigAction: () => void | Promise<void>
}

export function useSettingsDesktopState(input: SettingsDesktopStateInput) {
  return {
    backendErrorFacts: input.backendErrorFacts,
    backendUrl: input.backendUrl,
    config: input.config,
    configFacts: input.configFacts,
    enabled: input.enabled,
    form: input.form,
    installerAssetName: input.installerAssetName,
    installerUpdateUrl: input.installerUpdateUrl,
    notice: input.notice,
    runtimeFacts: input.runtimeFacts,
    saving: input.saving,
    serviceUpdateUrl: input.serviceUpdateUrl,
    statusFacts: input.statusFacts,
    updateFacts: input.updateFacts,
    updateProgressPercent: input.updateProgressPercent,
    updateRunning: input.updateRunning,
    loadPanel: input.loadPanel,
    openBackendUrlAction: input.openBackendUrlAction,
    openConfigAction: input.openConfigAction,
    refreshUpdateProgressAction: input.refreshUpdateProgressAction,
    restartBackendAction: input.restartBackendAction,
    runInstallerUpdateAction: input.runInstallerUpdateAction,
    runServiceUpdateAction: input.runServiceUpdateAction,
    saveConfigAction: input.saveConfigAction,
  }
}
