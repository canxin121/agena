<script setup lang="ts">
import ModelCatalogPanel from '@/components/settings/ModelCatalogPanel.vue'
import ProviderStudioPanel from '@/components/settings/ProviderStudioPanel.vue'
import ProvidersPanel from '@/components/settings/ProvidersPanel.vue'
import SettingsSectionWorkbench from '@/components/settings/workbench/SettingsSectionWorkbench.vue'
import { SETTINGS_DEFAULT_SUBPAGE, buildSettingsSubpages } from '@/components/settings/settingsNavigationCatalog'

const pages = buildSettingsSubpages('models-providers')
</script>

<template>
  <SettingsSectionWorkbench
    section="models-providers"
    :title="$st('Models & Providers')"
    :description="
      $st(
        'Manage provider credentials, adapters, and model routes, then inspect the complete model catalog.',
      )
    "
    :pages="pages"
    :default-page="SETTINGS_DEFAULT_SUBPAGE['models-providers']"
    v-slot="{ activePage }"
  >
    <ProviderStudioPanel v-if="activePage === 'provider-studio'" />
    <ModelCatalogPanel v-else-if="activePage === 'model-catalog'" />
    <ProvidersPanel v-else />
  </SettingsSectionWorkbench>
</template>
