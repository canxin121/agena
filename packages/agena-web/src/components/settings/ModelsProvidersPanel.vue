<script setup lang="ts">
import { computed, defineAsyncComponent, ref } from 'vue'
import SettingsSectionWorkbench from '@/components/settings/workbench/SettingsSectionWorkbench.vue'
import { SETTINGS_DEFAULT_SUBPAGE, buildSettingsSubpages } from '@/components/settings/settingsNavigationCatalog'

const ModelCatalogPanel = defineAsyncComponent(() => import('@/components/settings/ModelCatalogPanel.vue'))
const ProviderStudioPanel = defineAsyncComponent(() => import('@/components/settings/ProviderStudioPanel.vue'))
const ModelSettingsPanel = defineAsyncComponent(() => import('@/components/settings/ModelSettingsPanel.vue'))

const pages = computed(() => buildSettingsSubpages('models-providers'))
// Both model selections share one page and the workbench's layer selector.
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
    <ModelSettingsPanel ref="editor" v-else :scope="scope" />
  </SettingsSectionWorkbench>
</template>
