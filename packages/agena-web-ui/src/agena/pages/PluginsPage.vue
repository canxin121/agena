<script setup lang="ts">
import { useRoute, useRouter } from 'vue-router'

import PluginsSectionPanelRenderer from './PluginsSectionPanelRenderer.vue'
import SectionTabbedPageLayout from './SectionTabbedPageLayout.vue'
import { usePluginsPageState } from './usePluginsPageState'

const route = useRoute()
const router = useRouter()

const {
  activePluginsTab,
  actionError,
  actionMessage,
  load,
  loading,
  pageDescription,
  pageTitle,
  panels,
} = usePluginsPageState({ route, router })
</script>

<template>
  <SectionTabbedPageLayout
    :active-tab="activePluginsTab"
    :action-error="actionError"
    :action-message="actionMessage"
    :loading="loading"
    :page-description="pageDescription"
    :page-title="pageTitle"
    :tabs="tabs"
    @refresh="load"
    @update:active-tab="activePluginsTab = $event as 'installed' | 'marketplace'"
  >
    <PluginsSectionPanelRenderer
      :active-tab="activePluginsTab"
      :panels="panels"
    />
  </SectionTabbedPageLayout>
</template>
