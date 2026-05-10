<script setup lang="ts">
import type { RuntimeSkill } from '@/agena/lib/agenaApi'

import RuntimeCatalogSectionsPanel from './RuntimeCatalogSectionsPanel.vue'
import type { RuntimeSkillCatalogSection } from './useRuntimeSkillsState'

const props = defineProps<{
  queryValue: string
  queryLabel: string
  queryPlaceholder: string
  querySummary: string
  sections: RuntimeSkillCatalogSection[]
  openWorkspaceShortcut: (shortcutId: string) => void
  openRuntimeConfigRoot: () => void
  openPluginLogsWorkspacePath: () => void
  openRuntimeEntryInChat: (entry: RuntimeSkill) => void
  openRuntimeEntrySource: (entry: RuntimeSkill) => void
}>()

const emit = defineEmits<{
  'update:queryValue': [value: string]
}>()
</script>

<template>
  <div class="grid two">
    <section class="card" style="grid-column: 1 / -1">
      <div class="page-header" style="align-items: flex-start">
        <div>
          <h3>{{ props.queryLabel }}</h3>
          <p class="muted">{{ props.querySummary }}</p>
        </div>
        <div class="button-row" style="flex-wrap: wrap; justify-content: flex-end">
          <span class="badge mono">query={{ props.queryValue.trim() || 'all' }}</span>
          <button class="button" @click="props.openRuntimeConfigRoot">Open Config Root</button>
          <button class="button" @click="props.openPluginLogsWorkspacePath">Open Logs</button>
        </div>
      </div>
      <div class="field" style="margin-top: 12px">
        <label class="label" for="runtime-catalog-query">{{ props.queryLabel }}</label>
        <input
          id="runtime-catalog-query"
          :value="props.queryValue"
          class="input mono"
          :placeholder="props.queryPlaceholder"
          @input="emit('update:queryValue', ($event.target as HTMLInputElement).value)"
        />
      </div>
    </section>

    <RuntimeCatalogSectionsPanel
      :sections="props.sections"
      :open-workspace-shortcut="props.openWorkspaceShortcut"
      :open-runtime-entry-in-chat="props.openRuntimeEntryInChat"
      :open-runtime-entry-source="props.openRuntimeEntrySource"
    />
  </div>
</template>
