<script setup lang="ts">
import { computed, defineAsyncComponent, ref } from 'vue'
import SettingsSectionWorkbench from '@/components/settings/workbench/SettingsSectionWorkbench.vue'
import { SETTINGS_DEFAULT_SUBPAGE, buildSettingsSubpages } from '@/components/settings/settingsNavigationCatalog'

const ModelCatalogPanel = defineAsyncComponent(() => import('@/components/settings/ModelCatalogPanel.vue'))
const ProviderStudioPanel = defineAsyncComponent(() => import('@/components/settings/ProviderStudioPanel.vue'))
const ApprovalSettingsPanel = defineAsyncComponent(() => import('@/components/settings/ApprovalSettingsPanel.vue'))
const ModelDefaultsPanel = defineAsyncComponent(() => import('@/components/settings/ModelDefaultsPanel.vue'))

const pages = computed(() => buildSettingsSubpages('models-providers'))
// Model defaults and approval settings share the workbench's layer selector.
defineProps<{ activePage: string; scope: string }>()
const editor = ref<{ refresh: () => Promise<void> } | null>(null)
defineExpose({
  refresh: async () => {
    await editor.value?.refresh()
  },
})
</script>

<template>
  <SettingsSectionWorkbench
    section="models-providers"
    scope-layout="toolbar"
    :pages="pages"
    :active-page="activePage"
    :active-scope="scope"
    :default-page="SETTINGS_DEFAULT_SUBPAGE['models-providers']"
    v-slot="{ activePage }"
  >
    <ProviderStudioPanel ref="editor" v-if="activePage === 'provider-studio'" />
    <ModelCatalogPanel ref="editor" v-else-if="activePage === 'model-catalog'" />
    <ApprovalSettingsPanel ref="editor" v-else-if="activePage === 'approval-model'" :scope="scope" />
    <ModelDefaultsPanel ref="editor" v-else :scope="scope" />
  </SettingsSectionWorkbench>
</template>
