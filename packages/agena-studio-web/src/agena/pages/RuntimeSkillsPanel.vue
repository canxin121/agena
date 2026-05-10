<script setup lang="ts">
import type { RuntimeSkill } from '@/agena/lib/agenaApi'

import RuntimeCatalogPageShell from './RuntimeCatalogPageShell.vue'
import type { RuntimeSkillCatalogSection } from './useRuntimeSkillsState'

const props = defineProps<{
  runtimeSkillQuery: string
  catalogSections: RuntimeSkillCatalogSection[]
  openWorkspaceShortcut: (shortcutId: string) => void
  openRuntimeEntryInChat: (entry: RuntimeSkill) => void
  openRuntimeEntrySource: (entry: RuntimeSkill) => void
}>()

const emit = defineEmits<{
  'update:runtimeSkillQuery': [value: string]
}>()
</script>

<template>
  <RuntimeCatalogPageShell
    :query-value="props.runtimeSkillQuery"
    query-label="Search Skills & Commands"
    query-placeholder="name / alias / source path"
    query-summary="Search across runtime-discovered skill names, descriptions, aliases, and source paths."
    :sections="props.catalogSections"
    :open-workspace-shortcut="props.openWorkspaceShortcut"
    :open-runtime-entry-in-chat="props.openRuntimeEntryInChat"
    :open-runtime-entry-source="props.openRuntimeEntrySource"
    @update:query-value="emit('update:runtimeSkillQuery', $event)"
  />
</template>
