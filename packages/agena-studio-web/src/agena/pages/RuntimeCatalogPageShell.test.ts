import { describe, expect, test } from 'bun:test'

import { renderVueSsr } from './test/renderVueSsr'

describe('RuntimeCatalogPageShell', () => {
  test('renders shared catalog controls and section entries', async () => {
    const html = await renderVueSsr('/src/agena/pages/RuntimeCatalogPageShell.vue', {
      queryValue: 'review',
      queryLabel: 'Search Skills',
      queryPlaceholder: 'name / alias',
      querySummary: 'Search runtime entries.',
      sections: [
        {
          id: 'skills',
          title: 'Skills',
          description: 'Discovered skills',
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
    })

    expect(html.includes('Search Skills')).toBe(true)
    expect(html.includes('runtime-catalog-query')).toBe(true)
    expect(html.includes('query=review')).toBe(true)
    expect(html.includes('Open Config Root')).toBe(true)
    expect(html.includes('Open Logs')).toBe(true)
    expect(html.includes('Skills')).toBe(true)
    expect(html.includes('Review code')).toBe(true)
    expect(html.includes('Use in Chat')).toBe(true)
    expect(html.includes('Open Source')).toBe(true)
  })
})
