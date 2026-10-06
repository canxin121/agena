<script setup lang="ts">
import { computed, defineAsyncComponent } from 'vue'
import SettingsSectionWorkbench from '@/components/settings/workbench/SettingsSectionWorkbench.vue'
import { SETTINGS_DEFAULT_SUBPAGE, buildSettingsSubpages } from '@/components/settings/settingsNavigationCatalog'

const ModelCatalogPanel = defineAsyncComponent(() => import('@/components/settings/ModelCatalogPanel.vue'))
const ProviderStudioPanel = defineAsyncComponent(() => import('@/components/settings/ProviderStudioPanel.vue'))
const ProvidersPanel = defineAsyncComponent(() => import('@/components/settings/ProvidersPanel.vue'))

const pages = computed(() => buildSettingsSubpages('models-providers'))
</script>

<template>
  <SettingsSectionWorkbench
    section="models-providers"
    :title="$st('Models & Providers')"
    :description="
      $st('Manage provider credentials, adapters, and model routes, then inspect the complete model catalog.')
    "
    :pages="pages"
    :default-page="SETTINGS_DEFAULT_SUBPAGE['models-providers']"
    v-slot="{ activePage }"
  >
    <ProviderStudioPanel v-if="activePage === 'provider-studio'" />
    <ModelCatalogPanel v-else-if="activePage === 'model-catalog'" />
    <ProvidersPanel v-else :view="activePage === 'defaults' ? 'defaults' : 'inventory'" />
  </SettingsSectionWorkbench>
</template>
