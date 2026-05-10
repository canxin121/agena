import { computed, type ComputedRef, type Ref } from 'vue'

import type { RuntimeSkill } from '../lib/agenaApi'

export type RuntimeSkillCatalogSection = {
  id: 'skills' | 'commands'
  title: string
  description: string
  badgeLabel: 'skill' | 'command'
  openShortcutId: 'skills' | 'commands'
  openShortcutLabel: string
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
  openWorkspaceShortcut: (shortcutId: string) => void
  runtimeSkillQuery: Ref<string>
  skillCommands: ComputedRef<RuntimeSkill[]>
}

export function useRuntimeSkillsState(input: RuntimeSkillsStateInput) {
  const catalogSections = computed<RuntimeSkillCatalogSection[]>(() => [
    {
      id: 'skills',
      title: 'Skills',
      description: 'Discovered runtime skills can be opened in Chat or traced back to their workspace sources.',
      badgeLabel: 'skill',
      openShortcutId: 'skills',
      openShortcutLabel: 'Open Skills Dir',
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
      openShortcutId: 'commands',
      openShortcutLabel: 'Open Commands Dir',
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
    openWorkspaceShortcut: input.openWorkspaceShortcut,
    runtimeSkillQuery: input.runtimeSkillQuery,
  }
}
