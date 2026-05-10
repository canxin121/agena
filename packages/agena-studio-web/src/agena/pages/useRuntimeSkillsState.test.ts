import { describe, expect, test } from 'bun:test'
import { computed, ref } from 'vue'

import { useRuntimeSkillsState } from './useRuntimeSkillsState'

describe('useRuntimeSkillsState', () => {
  test('builds catalog sections for skills and commands', () => {
    const runtimeSkillQuery = ref('rev')
    const skills = computed(() => [
      { name: 'review', description: 'Review code', aliases: ['rv'], source_path: '.agena/skills/review.md' },
      { name: 'summarize', description: 'Summarize logs', aliases: [], source_path: '.agena/skills/summarize.md' },
    ])
    const filteredSkills = computed(() => [skills.value[0]!])
    const commands = computed(() => [
      { name: 'deploy', description: 'Deploy app', aliases: ['ship'], source_path: '.agena/commands/deploy.md' },
    ])
    const filteredCommands = computed(() => commands.value)

    const state = useRuntimeSkillsState({
      discoveredSkills: skills,
      filteredDiscoveredSkills: filteredSkills,
      filteredSkillCommands: filteredCommands,
      openPluginLogsWorkspacePath: () => {},
      openRuntimeConfigRoot: () => {},
      openRuntimeEntryInChat: () => {},
      openRuntimeEntrySource: () => {},
      openWorkspaceShortcut: () => {},
      runtimeSkillQuery,
      skillCommands: commands,
    })

    expect(state.runtimeSkillQuery.value).toBe('rev')
    expect(state.catalogSections.value).toEqual([
      {
        id: 'skills',
        title: 'Skills',
        description: 'Discovered runtime skills can be opened in Chat or traced back to their workspace sources.',
        badgeLabel: 'skill',
        openShortcutId: 'skills',
        openShortcutLabel: 'Open Skills Dir',
        totalCount: 2,
        filteredCount: 1,
        entries: [skills.value[0]!],
        emptyLabel: 'No skills matched the current filter.',
      },
      {
        id: 'commands',
        title: 'Commands',
        description: 'Discovered runtime commands now share the same slash surface as Chat and the global command palette.',
        badgeLabel: 'command',
        openShortcutId: 'commands',
        openShortcutLabel: 'Open Commands Dir',
        totalCount: 1,
        filteredCount: 1,
        entries: commands.value,
        emptyLabel: 'No commands matched the current filter.',
      },
    ])
  })
})
