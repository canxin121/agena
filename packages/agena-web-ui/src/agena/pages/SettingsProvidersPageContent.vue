<script setup lang="ts">
import type { useSettingsPageState } from './useSettingsPageState'

import ProvidersSettingsPanel from './ProvidersSettingsPanel.vue'

const props = defineProps<{
  providers: ReturnType<typeof useSettingsPageState>['panels']['providers']
}>()
</script>

<template>
  <ProvidersSettingsPanel
    :action-error="props.providers.actionError"
    :action-message="props.providers.actionMessage"
    :auth-providers="props.providers.authProviders"
    :browser-auth-code-drafts="props.providers.browserAuthCodeDrafts"
    :browser-auth-instance-drafts="props.providers.browserAuthInstanceDrafts"
    :browser-auth-start-state="props.providers.browserAuthStartState"
    :device-auth-enterprise-drafts="props.providers.deviceAuthEnterpriseDrafts"
    :device-auth-start-state="props.providers.deviceAuthStartState"
    :drafts="props.providers.drafts"
    :catalog-entries="props.providers.catalogEntries"
    :load="props.providers.load"
    :provider-models="props.providers.providerModels"
    :providers="props.providers.providers"
    :finish-browser-auth="props.providers.finishBrowserAuth"
    :poll-device-auth="props.providers.pollDeviceAuth"
    :save-api-key="props.providers.saveApiKey"
    :refresh-credential="props.providers.refreshCredential"
    :clear-credential="props.providers.clearCredential"
    :start-browser-auth="props.providers.startBrowserAuth"
    :start-device-auth="props.providers.startDeviceAuth"
  />
</template>
