<script setup lang="ts">
import PermissionStudioPanel from '@/components/settings/PermissionStudioPanel.vue'
import PermissionsPanel from '@/components/settings/PermissionsPanel.vue'
import SettingsSectionWorkbench from '@/components/settings/workbench/SettingsSectionWorkbench.vue'
import type { SettingsSubpageDefinition } from '@/components/settings/workbench/settingsSectionNavigation'
import { settingsText as st } from '@/i18n/settingsText'

const pages: SettingsSubpageDefinition[] = [
  {
    id: 'policy-studio',
    label: st('Permission Studio'),
    description: st('Edit global, workspace, current-session, and effective permission policy layers.'),
    keywords: ['filesystem', 'network', 'tools', 'allow', 'auto', 'ask', 'deny'],
  },
  {
    id: 'persistent-rules',
    label: st('Persistent rules'),
    description: st('Inspect and revoke durable approval rules captured from interactive permission decisions.'),
    keywords: ['rules', 'approval', 'revoke', 'history'],
  },
]
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
    default-page="policy-studio"
    v-slot="{ activePage }"
  >
    <PermissionStudioPanel v-if="activePage === 'policy-studio'" />
    <PermissionsPanel v-else />
  </SettingsSectionWorkbench>
</template>
