import { describe, expect, test } from 'bun:test'
import { ref } from 'vue'

import { useSectionPanelRegistry } from './useSectionPanelRegistry'

describe('useSectionPanelRegistry', () => {
  test('exposes current panel from active tab', () => {
    const activeTab = ref<'overview' | 'workflow'>('overview')
    const overview = { id: 'overview-panel' }
    const workflow = { id: 'workflow-panel' }
    const registry = useSectionPanelRegistry({
      activeTab,
      panels: {
        overview,
        workflow,
      },
    })

    expect(registry.panels.overview).toBe(overview)
    expect(registry.currentPanel.value).toBe(overview)

    activeTab.value = 'workflow'
    expect(registry.currentPanel.value).toBe(workflow)
  })
})
