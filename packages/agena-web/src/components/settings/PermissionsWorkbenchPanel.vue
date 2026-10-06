<script setup lang="ts">
import { computed, defineAsyncComponent } from 'vue'
import SettingsSectionWorkbench from '@/components/settings/workbench/SettingsSectionWorkbench.vue'
import { SETTINGS_DEFAULT_SUBPAGE, buildSettingsSubpages } from '@/components/settings/settingsNavigationCatalog'

const PermissionStudioPanel = defineAsyncComponent(() => import('@/components/settings/PermissionStudioPanel.vue'))
const PermissionsPanel = defineAsyncComponent(() => import('@/components/settings/PermissionsPanel.vue'))

const pages = computed(() => buildSettingsSubpages('permissions'))
</script>

<template>
  <SettingsSectionWorkbench
    section="permissions"
    :title="$st('Permissions')"
    :description="
      $st(
        'Control filesystem, network, and tool access at every configuration layer, then audit durable approval rules.',
      )
    "
    :pages="pages"
    :default-page="SETTINGS_DEFAULT_SUBPAGE.permissions"
    v-slot="{ activePage }"
  >
    <PermissionStudioPanel v-if="activePage === 'policy-studio'" />
    <PermissionsPanel v-else />
  </SettingsSectionWorkbench>
</template>
