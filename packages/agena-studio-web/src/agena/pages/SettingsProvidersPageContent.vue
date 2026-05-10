<script setup lang="ts">
import type { useSettingsPageState } from './useSettingsPageState'

import ProvidersSettingsPanel from './ProvidersSettingsPanel.vue'

const props = defineProps<{
  providers: ReturnType<typeof useSettingsPageState>['panels']['providers']
}>()
</script>

<template>
  <ProvidersSettingsPanel
    :auth-providers="props.providers.authProviders"
    :drafts="props.providers.drafts"
    :save-api-key="props.providers.saveApiKey"
    :refresh-credential="props.providers.refreshCredential"
    :clear-credential="props.providers.clearCredential"
  />
</template>
