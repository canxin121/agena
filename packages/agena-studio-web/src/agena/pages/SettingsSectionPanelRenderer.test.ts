import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'

const filePath = '/home/canxin/Git/ai/agena/packages/agena-studio-web/src/agena/pages/SettingsSectionPanelRenderer.vue'

describe('SettingsSectionPanelRenderer', () => {
  test('delegates settings tabs to dedicated page content components', () => {
    const source = readFileSync(filePath, 'utf8')

    expect(source.includes('SettingsProvidersPageContent')).toBe(true)
    expect(source.includes('SettingsPermissionsPageContent')).toBe(true)
    expect(source.includes('SettingsDesktopPageContent')).toBe(true)
    expect(source.includes("props.activeTab === 'providers'")).toBe(true)
    expect(source.includes("props.activeTab === 'permissions'")).toBe(true)
  })
})
