<script setup lang="ts">
import ActivitiesPanel from '@/components/settings/ActivitiesPanel.vue'
import AdvancedSettingsPanel from '@/components/settings/AdvancedSettingsPanel.vue'
import DiagnosticsPanel from '@/components/settings/DiagnosticsPanel.vue'
import MemoriesPanel from '@/components/settings/MemoriesPanel.vue'
import SettingsSectionWorkbench from '@/components/settings/workbench/SettingsSectionWorkbench.vue'
import type { SettingsSubpageDefinition } from '@/components/settings/workbench/settingsSectionNavigation'
import UsagePanel from '@/components/settings/UsagePanel.vue'

const pages: SettingsSubpageDefinition[] = [
  {
    id: 'runtime',
    label: 'Runtime & tracing',
    description: 'Configure tracing, inspect the runtime snapshot, validate settings, and reload the runtime.',
    keywords: ['tracing', 'logs', 'runtime', 'reload', 'validate'],
  },
  {
    id: 'advanced-settings',
    label: 'Advanced settings',
    description: 'Edit any explicit Global or Workspace JSON path with dry-run validation and source comparison.',
    keywords: ['advanced', 'configuration', 'json path', 'global', 'workspace', 'override'],
  },
  {
    id: 'activities',
    label: 'Activity history',
    description: 'Inspect durable operational activity records and their current states.',
    keywords: ['activities', 'tasks', 'operations', 'history'],
  },
  {
    id: 'memories',
    label: 'Memories',
    description: 'Inspect memory records and indexing state.',
    keywords: ['memory', 'index', 'documents'],
  },
  {
    id: 'usage',
    label: 'Usage',
    description: 'Review recorded usage and cost information.',
    keywords: ['usage', 'tokens', 'cost'],
  },
]
</script>

<template>
  <SettingsSectionWorkbench
    section="diagnostics"
    title="Diagnostics"
    description="Trace and validate the runtime, then inspect operational records without mixing them into one long page."
    :pages="pages"
    default-page="runtime"
    v-slot="{ activePage }"
  >
    <DiagnosticsPanel v-if="activePage === 'runtime'" />
    <AdvancedSettingsPanel v-else-if="activePage === 'advanced-settings'" />
    <ActivitiesPanel v-else-if="activePage === 'activities'" />
    <MemoriesPanel v-else-if="activePage === 'memories'" />
    <UsagePanel v-else />
  </SettingsSectionWorkbench>
</template>
