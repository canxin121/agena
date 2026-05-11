import { describe, expect, test } from 'bun:test'

import { renderVueSsr } from './test/renderVueSsr'

describe('SectionPageShell', () => {
  test('renders title, description, notices, and refresh action', async () => {
    const html = await renderVueSsr('/src/agena/pages/SectionPageShell.vue', {
      actionError: '',
      actionMessage: 'Saved',
      loading: false,
      pageDescription: 'Panel description',
      pageTitle: 'Panel title',
      refreshLabel: 'Reload',
      showRefresh: true,
    })

    expect(html.includes('Panel title')).toBe(true)
    expect(html.includes('Panel description')).toBe(true)
    expect(html.includes('Saved')).toBe(true)
    expect(html.includes('Reload')).toBe(true)
  })

  test('hides refresh button when disabled', async () => {
    const html = await renderVueSsr('/src/agena/pages/SectionPageShell.vue', {
      actionError: '',
      actionMessage: '',
      loading: false,
      pageDescription: 'Panel description',
      pageTitle: 'Panel title',
      showRefresh: false,
    })

    expect(html.includes('Refresh')).toBe(false)
  })
})
