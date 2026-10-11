<script setup lang="ts">
import { computed, defineAsyncComponent } from 'vue'
import SettingsSectionWorkbench from '@/components/settings/workbench/SettingsSectionWorkbench.vue'
import { SETTINGS_DEFAULT_SUBPAGE, buildSettingsSubpages } from '@/components/settings/settingsNavigationCatalog'

const PermissionStudioPanel = defineAsyncComponent(() => import('@/components/settings/PermissionStudioPanel.vue'))

const pages = computed(() => buildSettingsSubpages('permissions'))
// The permission source belongs to the shared left navigation now, so this
// workbench only forwards the layer the user selected there.
defineProps<{ activePage: string; scope: string }>()
</script>

<template>
  <SettingsSectionWorkbench
    section="permissions"
    :pages="pages"
    :active-page="activePage"
    :default-page="SETTINGS_DEFAULT_SUBPAGE.permissions"
    v-slot="{ activePage }"
  >
    <PermissionStudioPanel :scope="scope" :section="activePage" />
  </SettingsSectionWorkbench>
</template>
