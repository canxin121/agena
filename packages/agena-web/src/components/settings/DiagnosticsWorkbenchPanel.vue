<script setup lang="ts">
import ActivitiesPanel from '@/components/settings/ActivitiesPanel.vue'
import AdvancedSettingsPanel from '@/components/settings/AdvancedSettingsPanel.vue'
import DiagnosticsPanel from '@/components/settings/DiagnosticsPanel.vue'
import MemoriesPanel from '@/components/settings/MemoriesPanel.vue'
import SettingsSectionWorkbench from '@/components/settings/workbench/SettingsSectionWorkbench.vue'
import { SETTINGS_DEFAULT_SUBPAGE, buildSettingsSubpages } from '@/components/settings/settingsNavigationCatalog'
import UsagePanel from '@/components/settings/UsagePanel.vue'

const pages = buildSettingsSubpages('diagnostics')
</script>

<template>
  <SettingsSectionWorkbench
    section="diagnostics"
    :title="$st('Diagnostics')"
    :description="
      $st('Trace and validate the runtime, then inspect operational records without mixing them into one long page.')
    "
    :pages="pages"
    :default-page="SETTINGS_DEFAULT_SUBPAGE.diagnostics"
    v-slot="{ activePage }"
  >
    <DiagnosticsPanel v-if="activePage === 'runtime'" />
    <AdvancedSettingsPanel v-else-if="activePage === 'advanced-settings'" />
    <ActivitiesPanel v-else-if="activePage === 'activities'" />
    <MemoriesPanel v-else-if="activePage === 'memories'" />
    <UsagePanel v-else />
  </SettingsSectionWorkbench>
</template>
