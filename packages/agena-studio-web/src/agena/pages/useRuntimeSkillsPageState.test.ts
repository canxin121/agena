import { describe, expect, test } from 'bun:test'
import { computed, ref } from 'vue'

import { createRuntimeSkillsPanelState, useRuntimeSkillsPageState } from './useRuntimeSkillsPageState'

describe('useRuntimeSkillsPageState', () => {
  test('assembles panel state from provided runtime skills source', () => {
    const panel = createRuntimeSkillsPanelState({
      discoveredSkills: computed(() => [
        { name: 'review', description: 'Review code', aliases: ['rv'], source_path: '.agena/skills/review.md' },
      ]),
      filteredDiscoveredSkills: computed(() => [
        { name: 'review', description: 'Review code', aliases: ['rv'], source_path: '.agena/skills/review.md' },
      ]),
      filteredSkillCommands: computed(() => [
        { name: 'deploy', description: 'Deploy app', aliases: ['ship'], source_path: '.agena/commands/deploy.md' },
      ]),
      openPluginLogsWorkspacePath: () => {},
      openRuntimeConfigRoot: () => {},
      openRuntimeEntryInChat: () => {},
      openRuntimeEntrySource: () => {},
      openWorkspaceShortcut: () => {},
      runtimeSkillQuery: ref('review'),
      skillCommands: computed(() => [
        { name: 'deploy', description: 'Deploy app', aliases: ['ship'], source_path: '.agena/commands/deploy.md' },
      ]),
    })

    expect(panel.runtimeSkillQuery.value).toBe('review')
    expect(panel.catalogSections.value.map((section) => section.id)).toEqual(['skills', 'commands'])
  })

  test('exposes shared shell fields via injected section state', () => {
    const route = { path: '/runtime/skills' }
    const router = { push: async () => {}, replace: async () => {} }
    const shared = {
      actionError: ref(''),
      actionMessage: ref('ok'),
      load: async () => {},
      loading: ref(false),
      pageDescription: computed(() => 'desc'),
      pageTitle: computed(() => 'title'),
    }

    const result = useRuntimeSkillsPageState(
      { route: route as never, router: router as never },
      {
        useRuntimeSectionState: (value) => {
          expect(value).toEqual({ route, router, section: 'runtime' })
          return {
            shared,
            state: {
              discoveredSkills: computed(() => []),
              filteredDiscoveredSkills: computed(() => []),
              filteredSkillCommands: computed(() => []),
              openPluginLogsWorkspacePath: () => {},
              openRuntimeConfigRoot: () => {},
              openRuntimeEntryInChat: () => {},
              openRuntimeEntrySource: () => {},
              openWorkspaceShortcut: () => {},
              runtimeSkillQuery: ref(''),
              skillCommands: computed(() => []),
            },
          }
        },
      },
    )

    expect(result.actionMessage).toBe(shared.actionMessage)
    expect(result.pageTitle).toBe(shared.pageTitle)
    expect(result.pageDescription).toBe(shared.pageDescription)
    expect(result.load).toBe(shared.load)
    expect(result.skills.catalogSections.value).toEqual([
      {
        id: 'skills',
        title: 'Skills',
        description: 'Discovered runtime skills can be opened in Chat or traced back to their workspace sources.',
        badgeLabel: 'skill',
        openShortcutId: 'skills',
        openShortcutLabel: 'Open Skills Dir',
        totalCount: 0,
        filteredCount: 0,
        entries: [],
        emptyLabel: 'No skills matched the current filter.',
      },
      {
        id: 'commands',
        title: 'Commands',
        description: 'Discovered runtime commands now share the same slash surface as Chat and the global command palette.',
        badgeLabel: 'command',
        openShortcutId: 'commands',
        openShortcutLabel: 'Open Commands Dir',
        totalCount: 0,
        filteredCount: 0,
        entries: [],
        emptyLabel: 'No commands matched the current filter.',
      },
    ])
  })
})
