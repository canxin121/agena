import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'

const filePath = '/home/canxin/Git/ai/agena/packages/agena-studio-web/src/agena/pages/PluginsSectionPanelRenderer.vue'

describe('PluginsSectionPanelRenderer', () => {
  test('delegates plugin tabs to dedicated page content components', () => {
    const source = readFileSync(filePath, 'utf8')

    expect(source.includes('PluginsInstalledPageContent')).toBe(true)
    expect(source.includes('PluginsMarketplacePageContent')).toBe(true)
    expect(source.includes("props.activeTab === 'installed'")).toBe(true)
  })
})
