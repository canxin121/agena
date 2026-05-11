import { describe, expect, test } from 'bun:test'

import { renderVueSsr } from './test/renderVueSsr'

describe('SectionTabbedPageLayout', () => {
  test('renders shared shell and tab buttons', async () => {
    const html = await renderVueSsr('/src/agena/pages/SectionTabbedPageLayout.vue', {
      activeTab: 'providers',
      actionError: '',
      actionMessage: '',
      loading: false,
      pageDescription: 'Settings description',
      pageTitle: 'Settings title',
      tabs: [
        { id: 'providers', label: 'Providers' },
        { id: 'permissions', label: 'Permissions' },
      ],
    })

    expect(html.includes('Settings title')).toBe(true)
    expect(html.includes('Providers')).toBe(true)
    expect(html.includes('Permissions')).toBe(true)
    expect(html.includes('Refresh')).toBe(false)
  })
})
