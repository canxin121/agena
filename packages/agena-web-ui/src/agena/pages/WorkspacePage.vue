<script setup lang="ts">
import { nextTick, onMounted } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import WorkspacePageContent from './WorkspacePageContent.vue'
import WorkspaceSectionLayout from './WorkspaceSectionLayout.vue'
import { useWorkspacePageState } from './useWorkspacePageState'

const route = useRoute()
const router = useRouter()

const workspace = useWorkspacePageState({ route, router })

onMounted(async () => {
  workspace.syncFromRoute()
  await workspace.load()
  if (route.hash) {
    await nextTick()
    document.querySelector<HTMLElement>(route.hash)?.scrollIntoView({ behavior: 'smooth', block: 'start' })
  }
})
</script>

<template>
  <WorkspaceSectionLayout
    :action-error="workspace.actionError.value"
    :action-message="workspace.actionMessage.value"
    :loading="workspace.loading.value"
    :page-description="workspace.pageDescription.value"
    :page-title="workspace.pageTitle.value"
    @refresh="workspace.load"
    @root="workspace.goRoot"
  >
    <WorkspacePageContent :workspace="workspace" />
  </WorkspaceSectionLayout>
</template>
