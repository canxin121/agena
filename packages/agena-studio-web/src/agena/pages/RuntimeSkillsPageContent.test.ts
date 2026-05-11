import { describe, expect, test } from 'bun:test'

import { renderVueSsr } from './test/renderVueSsr'

describe('RuntimeSkillsPageContent', () => {
  test('renders the runtime skills shell and shared actions', async () => {
    const html = await renderVueSsr('/src/agena/pages/RuntimeSkillsPageContent.vue', {
      skills: {
        runtimeSkillQuery: 'review',
        catalogSections: [
          {
            id: 'skills',
            title: 'Skills',
            description: 'Discovered runtime skills',
            badgeLabel: 'skill',
            openShortcutId: 'skills',
            openShortcutLabel: 'Open Skills Dir',
            totalCount: 1,
            filteredCount: 1,
            entries: [
              {
                name: 'review',
                description: 'Review code',
                aliases: ['rv'],
                source_path: '.agena/skills/review.md',
              },
            ],
            emptyLabel: 'No skills',
          },
        ],
        openWorkspaceShortcut: () => {},
        openRuntimeConfigRoot: () => {},
        openPluginLogsWorkspacePath: () => {},
        openRuntimeEntryInChat: () => {},
        openRuntimeEntrySource: () => {},
      },
    })

    expect(html.includes('Search Skills &amp; Commands')).toBe(true)
    expect(html.includes('Open Config Root')).toBe(true)
    expect(html.includes('Open Logs')).toBe(true)
    expect(html.includes('Review code')).toBe(true)
    expect(html.includes('Open Source')).toBe(true)
  })
})
