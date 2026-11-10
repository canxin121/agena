<script setup lang="ts">
import { computed, defineAsyncComponent, ref } from 'vue'
import SettingsSectionWorkbench from '@/components/settings/workbench/SettingsSectionWorkbench.vue'
import { SETTINGS_DEFAULT_SUBPAGE, buildSettingsSubpages } from '@/components/settings/settingsNavigationCatalog'

const PermissionStudioPanel = defineAsyncComponent(() => import('@/components/settings/PermissionStudioPanel.vue'))
const PermissionsOverviewPanel = defineAsyncComponent(
  () => import('@/components/settings/PermissionsOverviewPanel.vue'),
)

const pages = computed(() => buildSettingsSubpages('permissions'))
// The permission workbench owns the scope selector and forwards its route-backed value.
defineProps<{ activePage: string; scope: string; sessionId?: string }>()
const studio = ref<{ refresh: () => Promise<void> } | null>(null)
defineExpose({ refresh: () => studio.value?.refresh() })
</script>

<template>
  <SettingsSectionWorkbench
    section="permissions"
    :pages="pages"
    :active-page="activePage"
    :active-scope="scope"
    scope-layout="toolbar"
    :default-page="SETTINGS_DEFAULT_SUBPAGE.permissions"
    v-slot="{ activePage }"
  >
    <PermissionsOverviewPanel v-if="activePage === 'overview'" ref="studio" :session-id="sessionId" />
    <PermissionStudioPanel
      v-else
      ref="studio"
      :scope="scope"
      :section="activePage"
      :session-id="sessionId"
    />
  </SettingsSectionWorkbench>
</template>
