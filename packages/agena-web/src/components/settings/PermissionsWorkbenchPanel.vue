<script setup lang="ts">
import { defineAsyncComponent, ref } from 'vue'

const PermissionProfilesPanel = defineAsyncComponent(() => import('@/components/settings/PermissionProfilesPanel.vue'))
const PermissionStudioPanel = defineAsyncComponent(() => import('@/components/settings/PermissionStudioPanel.vue'))
// The permission workbench owns the scope selector and forwards its route-backed value.
defineProps<{ activePage: string; scope: string; sessionId?: string }>()
const studio = ref<{ refresh: () => Promise<void> } | null>(null)
defineExpose({ refresh: () => studio.value?.refresh() })
</script>

<template>
  <PermissionStudioPanel v-if="activePage === 'access'" ref="studio" :scope="scope" :session-id="sessionId" />
  <PermissionProfilesPanel v-else :key="activePage" ref="studio" :assignments="activePage === 'assignments'" />
</template>
