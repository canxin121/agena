<script setup lang="ts">
import { computed, defineAsyncComponent, ref } from 'vue'
import SettingsSectionWorkbench from '@/components/settings/workbench/SettingsSectionWorkbench.vue'
import { SETTINGS_DEFAULT_SUBPAGE, buildSettingsSubpages } from '@/components/settings/settingsNavigationCatalog'

const ModelCatalogPanel = defineAsyncComponent(() => import('@/components/settings/ModelCatalogPanel.vue'))
const ProviderStudioPanel = defineAsyncComponent(() => import('@/components/settings/ProviderStudioPanel.vue'))

const pages = computed(() => buildSettingsSubpages('models-providers'))
defineProps<{ activePage: string }>()
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
    :pages="pages"
    :active-page="activePage"
    :default-page="SETTINGS_DEFAULT_SUBPAGE['models-providers']"
    v-slot="{ activePage }"
  >
    <ModelCatalogPanel v-if="activePage === 'model-catalog'" ref="editor" />
    <ProviderStudioPanel v-else ref="editor" />
  </SettingsSectionWorkbench>
</template>
