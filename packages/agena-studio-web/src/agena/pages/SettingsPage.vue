<script setup lang="ts">
import { useRoute, useRouter } from 'vue-router'

import SectionTabbedPageLayout from './SectionTabbedPageLayout.vue'
import SettingsSectionPanelRenderer from './SettingsSectionPanelRenderer.vue'
import { useSettingsPageState } from './useSettingsPageState'

const route = useRoute()
const router = useRouter()

const {
  activeSettingsTab,
  actionError,
  actionMessage,
  load,
  loading,
  pageDescription,
  pageTitle,
  panels,
} = useSettingsPageState({ route, router })
</script>

<template>
  <SectionTabbedPageLayout
    :active-tab="activeSettingsTab"
    :action-error="actionError"
    :action-message="actionMessage"
    :loading="loading"
    :page-description="pageDescription"
    :page-title="pageTitle"
    :tabs="tabs"
    @refresh="load"
    @update:active-tab="activeSettingsTab = $event as 'providers' | 'permissions' | 'desktop'"
  >
    <SettingsSectionPanelRenderer
      :active-tab="activeSettingsTab"
      :loading="loading"
      :load="load"
      :panels="panels"
    />
  </SectionTabbedPageLayout>
</template>
