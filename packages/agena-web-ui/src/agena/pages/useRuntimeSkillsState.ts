import { computed, type ComputedRef, type Ref } from 'vue'

import type { RuntimeSkill } from '../lib/agenaApi'

export type RuntimeSkillCatalogSection = {
  id: 'skills' | 'commands'
  title: string
  description: string
  badgeLabel: 'skill' | 'command'
  totalCount: number
  filteredCount: number
  entries: RuntimeSkill[]
  emptyLabel: string
}

export type RuntimeSkillsStateInput = {
  discoveredSkills: ComputedRef<RuntimeSkill[]>
  filteredDiscoveredSkills: ComputedRef<RuntimeSkill[]>
  filteredSkillCommands: ComputedRef<RuntimeSkill[]>
  openPluginLogsWorkspacePath: () => void
  openRuntimeConfigRoot: () => void
  openRuntimeEntryInChat: (entry: RuntimeSkill) => void
  openRuntimeEntrySource: (entry: RuntimeSkill) => void
  runtimeSkillQuery: Ref<string>
  skillCommands: ComputedRef<RuntimeSkill[]>
}

export function useRuntimeSkillsState(input: RuntimeSkillsStateInput) {
  const catalogSections = computed<RuntimeSkillCatalogSection[]>(() => [
    {
      id: 'skills',
      title: 'Skills',
      description: 'Runtime skills can be opened in Chat or inspected from their resolved source metadata.',
      badgeLabel: 'skill',
      totalCount: input.discoveredSkills.value.length,
      filteredCount: input.filteredDiscoveredSkills.value.length,
      entries: input.filteredDiscoveredSkills.value,
      emptyLabel: 'No skills matched the current filter.',
    },
    {
      id: 'commands',
      title: 'Commands',
      description: 'Discovered runtime commands now share the same slash surface as Chat and the global command palette.',
      badgeLabel: 'command',
      totalCount: input.skillCommands.value.length,
      filteredCount: input.filteredSkillCommands.value.length,
      entries: input.filteredSkillCommands.value,
      emptyLabel: 'No commands matched the current filter.',
    },
  ])

  return {
    catalogSections,
    openPluginLogsWorkspacePath: input.openPluginLogsWorkspacePath,
    openRuntimeConfigRoot: input.openRuntimeConfigRoot,
    openRuntimeEntryInChat: input.openRuntimeEntryInChat,
    openRuntimeEntrySource: input.openRuntimeEntrySource,
    runtimeSkillQuery: input.runtimeSkillQuery,
  }
}
