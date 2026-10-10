<script setup lang="ts">
import { computed, defineAsyncComponent } from 'vue'
import SettingsSectionWorkbench from '@/components/settings/workbench/SettingsSectionWorkbench.vue'
import { SETTINGS_DEFAULT_SUBPAGE, buildSettingsSubpages } from '@/components/settings/settingsNavigationCatalog'

const ModelCatalogPanel = defineAsyncComponent(() => import('@/components/settings/ModelCatalogPanel.vue'))
const ProviderStudioPanel = defineAsyncComponent(() => import('@/components/settings/ProviderStudioPanel.vue'))
const ApprovalSettingsPanel = defineAsyncComponent(() => import('@/components/settings/ApprovalSettingsPanel.vue'))
const ModelDefaultsPanel = defineAsyncComponent(() => import('@/components/settings/ModelDefaultsPanel.vue'))

const pages = computed(() => buildSettingsSubpages('models-providers'))
defineProps<{ activePage: string }>()
</script>

<template>
  <SettingsSectionWorkbench
    section="models-providers"
    :pages="pages"
    :active-page="activePage"
    :default-page="SETTINGS_DEFAULT_SUBPAGE['models-providers']"
    v-slot="{ activePage }"
  >
    <ProviderStudioPanel v-if="activePage === 'provider-studio'" />
    <ModelCatalogPanel v-else-if="activePage === 'model-catalog'" />
    <ApprovalSettingsPanel v-else-if="activePage === 'approval-model'" />
    <ModelDefaultsPanel v-else />
  </SettingsSectionWorkbench>
</template>
