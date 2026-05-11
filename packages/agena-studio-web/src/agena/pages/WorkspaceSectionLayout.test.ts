import { describe, expect, test } from 'bun:test'

import { renderVueSsr } from './test/renderVueSsr'

describe('WorkspaceSectionLayout', () => {
  test('renders workspace actions', async () => {
    const html = await renderVueSsr('/src/agena/pages/WorkspaceSectionLayout.vue', {
      actionError: '',
      actionMessage: '',
      loading: false,
      pageDescription: 'Workspace description',
      pageTitle: 'Workspace title',
    })

    expect(html.includes('Workspace title')).toBe(true)
    expect(html.includes('Refresh')).toBe(true)
    expect(html.includes('Root')).toBe(true)
  })
})
