<script setup lang="ts">
import { computed, defineAsyncComponent, ref } from 'vue'
import SettingsSectionWorkbench from '@/components/settings/workbench/SettingsSectionWorkbench.vue'
import { SETTINGS_DEFAULT_SUBPAGE, buildSettingsSubpages } from '@/components/settings/settingsNavigationCatalog'

const PermissionProfilesPanel = defineAsyncComponent(() => import('@/components/settings/PermissionProfilesPanel.vue'))
const PermissionStudioPanel = defineAsyncComponent(() => import('@/components/settings/PermissionStudioPanel.vue'))
const PermissionsOverviewPanel = defineAsyncComponent(
  () => import('@/components/settings/PermissionsOverviewPanel.vue'),
)

const pages = computed(() => buildSettingsSubpages('permissions'))
const SandboxSettingsPanel = defineAsyncComponent(() => import('@/components/settings/SandboxSettingsPanel.vue'))
// The permission workbench owns the scope selector and forwards its route-backed value.
defineProps<{ activePage: string; scope: string; sessionId?: string }>()
const studio = ref<{ refresh: () => Promise<void> } | null>(null)
defineExpose({ refresh: () => studio.value?.refresh() })
</script>

<template>
  <PermissionStudioPanel v-if="activePage === 'access'" ref="studio" :scope="scope" :session-id="sessionId" />
  <SettingsSectionWorkbench
    v-else
    section="permissions"
    :pages="pages"
    :active-page="activePage"
    :active-scope="scope"
    scope-layout="toolbar"
    :default-page="SETTINGS_DEFAULT_SUBPAGE.permissions"
    v-slot="{ activePage }"
  >
    <PermissionsOverviewPanel v-if="activePage === 'overview'" ref="studio" :session-id="sessionId" />
    <PermissionProfilesPanel v-else-if="activePage === 'profiles'" ref="studio" />
    <SandboxSettingsPanel v-else-if="activePage === 'sandbox'" ref="studio" :scope="scope" />
    <PermissionStudioPanel v-else ref="studio" :scope="scope" :session-id="sessionId" />
  </SettingsSectionWorkbench>
</template>
