<script setup lang="ts">
import PermissionStudioPanel from '@/components/settings/PermissionStudioPanel.vue'
import PermissionsPanel from '@/components/settings/PermissionsPanel.vue'
import SettingsSectionWorkbench from '@/components/settings/workbench/SettingsSectionWorkbench.vue'
import type { SettingsSubpageDefinition } from '@/components/settings/workbench/settingsSectionNavigation'

const pages: SettingsSubpageDefinition[] = [
  {
    id: 'policy-studio',
    label: 'Permission Studio',
    description: 'Edit global, workspace, current-session, and effective permission policy layers.',
    keywords: ['filesystem', 'network', 'tools', 'allow', 'auto', 'ask', 'deny'],
  },
  {
    id: 'persistent-rules',
    label: 'Persistent rules',
    description: 'Inspect and revoke durable approval rules captured from interactive permission decisions.',
    keywords: ['rules', 'approval', 'revoke', 'history'],
  },
]
</script>

<template>
  <SettingsSectionWorkbench
    section="permissions"
    title="Permissions"
    description="Control filesystem, network, and tool access at every configuration layer, then audit durable approval rules."
    :pages="pages"
    default-page="policy-studio"
    v-slot="{ activePage }"
  >
    <PermissionStudioPanel v-if="activePage === 'policy-studio'" />
    <PermissionsPanel v-else />
  </SettingsSectionWorkbench>
</template>
