<script setup lang="ts">
import { computed, defineAsyncComponent, ref } from 'vue'
import SettingsSectionWorkbench from '@/components/settings/workbench/SettingsSectionWorkbench.vue'
import { SETTINGS_DEFAULT_SUBPAGE, buildSettingsSubpages } from '@/components/settings/settingsNavigationCatalog'

const PermissionStudioPanel = defineAsyncComponent(() => import('@/components/settings/PermissionStudioPanel.vue'))

const pages = computed(() => buildSettingsSubpages('permissions'))
const permissionScope = ref<'effective' | 'global' | 'workspace' | 'session'>('effective')
</script>

<template>
  <SettingsSectionWorkbench
    section="permissions"
    :pages="pages"
    :default-page="SETTINGS_DEFAULT_SUBPAGE.permissions"
    v-slot="{ activePage }"
  >
    <PermissionStudioPanel v-model:scope="permissionScope" :section="activePage" />
  </SettingsSectionWorkbench>
</template>
