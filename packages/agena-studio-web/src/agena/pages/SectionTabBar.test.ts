import { describe, expect, test } from 'bun:test'

import { renderVueSsr } from './test/renderVueSsr'

describe('SectionTabBar', () => {
  test('renders active and inactive tabs', async () => {
    const html = await renderVueSsr('/src/agena/pages/SectionTabBar.vue', {
      activeTab: 'skills',
      tabs: [
        { id: 'overview', label: 'Overview' },
        { id: 'skills', label: 'Skills' },
      ],
    })

    expect(html.includes('Overview')).toBe(true)
    expect(html.includes('Skills')).toBe(true)
    expect(html.includes('primary button')).toBe(true)
  })

  test('omits wrapper when there are no tabs', async () => {
    const html = await renderVueSsr('/src/agena/pages/SectionTabBar.vue', {
      activeTab: 'overview',
      tabs: [],
    })

    expect(html).toBe('<!---->')
  })
})
