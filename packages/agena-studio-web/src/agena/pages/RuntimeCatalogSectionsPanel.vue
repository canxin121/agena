<script setup lang="ts">
import type { RuntimeSkill } from '@/agena/lib/agenaApi'

import type { RuntimeSkillCatalogSection } from './useRuntimeSkillsState'

const props = defineProps<{
  sections: RuntimeSkillCatalogSection[]
  openWorkspaceShortcut: (shortcutId: string) => void
  openRuntimeEntryInChat: (entry: RuntimeSkill) => void
  openRuntimeEntrySource: (entry: RuntimeSkill) => void
}>()
</script>

<template>
  <template v-for="section in props.sections" :key="section.id">
    <section class="card">
      <div class="page-header" style="align-items: flex-start">
        <div>
          <h3>{{ section.title }}</h3>
          <p class="muted">{{ section.description }}</p>
        </div>
        <div class="button-row" style="flex-wrap: wrap">
          <span class="badge">{{ section.filteredCount }}/{{ section.totalCount }}</span>
          <button class="button" @click="props.openWorkspaceShortcut(section.openShortcutId)">{{ section.openShortcutLabel }}</button>
        </div>
      </div>
      <div v-if="section.entries.length" class="list">
        <div v-for="skill in section.entries" :key="`${section.id}-${skill.name}`" class="list-item">
          <div class="page-header" style="align-items: flex-start">
            <div>
              <div><strong>{{ skill.name }}</strong></div>
              <div class="muted">{{ skill.description || 'No description' }}</div>
              <div v-if="skill.aliases.length" class="muted">aliases: {{ skill.aliases.join(', ') }}</div>
              <div class="muted mono">source={{ skill.source_path || 'runtime' }}</div>
            </div>
            <span class="badge">{{ section.badgeLabel }}</span>
          </div>
          <div class="button-row" style="margin-top: 10px; flex-wrap: wrap">
            <button class="button" @click="props.openRuntimeEntryInChat(skill)">Use in Chat</button>
            <button v-if="skill.source_path" class="button" @click="props.openRuntimeEntrySource(skill)">Open Source</button>
          </div>
        </div>
      </div>
      <p v-else class="muted">{{ section.emptyLabel }}</p>
    </section>
  </template>
</template>
