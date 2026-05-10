<script setup lang="ts">
import type { useSettingsPageState } from './useSettingsPageState'

import DesktopSettingsPanel from './DesktopSettingsPanel.vue'

const props = defineProps<{
  desktop: ReturnType<typeof useSettingsPageState>['panels']['desktop']
  loading: boolean
}>()
</script>

<template>
  <DesktopSettingsPanel
    :loading="props.loading"
    :desktop-enabled="props.desktop.enabled"
    :desktop-saving="props.desktop.saving"
    :desktop-update-running="props.desktop.updateRunning"
    :desktop-backend-url="props.desktop.backendUrl"
    :desktop-notice="props.desktop.notice"
    :desktop-config="props.desktop.config"
    :desktop-runtime-facts="props.desktop.runtimeFacts"
    :desktop-status-facts="props.desktop.statusFacts"
    :desktop-backend-error-facts="props.desktop.backendErrorFacts"
    :desktop-update-facts="props.desktop.updateFacts"
    :desktop-config-facts="props.desktop.configFacts"
    :desktop-update-progress-percent="props.desktop.updateProgressPercent"
    :desktop-service-update-url="props.desktop.serviceUpdateUrl"
    :desktop-installer-update-url="props.desktop.installerUpdateUrl"
    :desktop-installer-asset-name="props.desktop.installerAssetName"
    :desktop-form="props.desktop.form"
    :load-desktop-panel="props.desktop.loadPanel"
    :restart-desktop-backend-action="props.desktop.restartBackendAction"
    :open-desktop-backend-url-action="props.desktop.openBackendUrlAction"
    :open-desktop-config-action="props.desktop.openConfigAction"
    :refresh-desktop-update-progress-action="props.desktop.refreshUpdateProgressAction"
    :run-desktop-service-update-action="props.desktop.runServiceUpdateAction"
    :run-desktop-installer-update-action="props.desktop.runInstallerUpdateAction"
    :save-desktop-config-action="props.desktop.saveConfigAction"
    @update:desktop-service-update-url="props.desktop.serviceUpdateUrl = $event"
    @update:desktop-installer-update-url="props.desktop.installerUpdateUrl = $event"
    @update:desktop-installer-asset-name="props.desktop.installerAssetName = $event"
  />
</template>
