<script setup lang="ts">
import PermissionStudioPanel from '@/components/settings/PermissionStudioPanel.vue'
import PermissionsPanel from '@/components/settings/PermissionsPanel.vue'
import SettingsSectionWorkbench from '@/components/settings/workbench/SettingsSectionWorkbench.vue'
import { SETTINGS_DEFAULT_SUBPAGE, buildSettingsSubpages } from '@/components/settings/settingsNavigationCatalog'

const pages = buildSettingsSubpages('permissions')
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
