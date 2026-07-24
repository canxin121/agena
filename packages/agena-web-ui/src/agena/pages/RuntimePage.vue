<script setup lang="ts">
import { useRoute, useRouter } from 'vue-router'

import RuntimeSectionLayout from './RuntimeSectionLayout.vue'
import RuntimeSectionPanelRenderer from './RuntimeSectionPanelRenderer.vue'
import { formatProviderModel } from './runtimePageModel'
import { useRuntimeSectionPageState } from './useRuntimeSectionPageState'

const route = useRoute()
const router = useRouter()

const {
  activeTab,
  actionError,
  actionMessage,
  load,
  loading,
  pageDescription,
  pageTitle,
  panels,
  triggerReload,
  visibleTabs,
} = useRuntimeSectionPageState({ route, router })
</script>

<template>
  <RuntimeSectionLayout
    :active-tab="activeTab"
    :action-error="actionError"
    :action-message="actionMessage"
    :loading="loading"
    :page-description="pageDescription"
    :page-title="pageTitle"
    :tabs="visibleTabs"
    @refresh="load"
    @reload="triggerReload"
    @update:active-tab="activeTab = $event"
  >
    <RuntimeSectionPanelRenderer
      :active-tab="activeTab"
      :format-provider-model="formatProviderModel"
      :load="load"
      :panels="panels"
    />
  </RuntimeSectionLayout>
</template>
