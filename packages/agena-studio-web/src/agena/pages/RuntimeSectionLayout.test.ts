import { describe, expect, test } from 'bun:test'

import { renderVueSsr } from './test/renderVueSsr'

describe('RuntimeSectionLayout', () => {
  test('renders runtime actions and tabs', async () => {
    const html = await renderVueSsr('/src/agena/pages/RuntimeSectionLayout.vue', {
      activeTab: 'overview',
      actionError: '',
      actionMessage: '',
      loading: false,
      pageDescription: 'Runtime description',
      pageTitle: 'Runtime title',
      tabs: [
        { id: 'overview', label: 'Overview' },
        { id: 'skills', label: 'Skills' },
      ],
    })

    expect(html.includes('Runtime title')).toBe(true)
    expect(html.includes('Refresh')).toBe(true)
    expect(html.includes('Reload Runtime')).toBe(true)
    expect(html.includes('Overview')).toBe(true)
    expect(html.includes('Skills')).toBe(true)
  })
})
