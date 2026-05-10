import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'

const filePath = '/home/canxin/Git/ai/agena/packages/agena-studio-web/src/agena/pages/SectionTabbedPageLayout.vue'

describe('SectionTabbedPageLayout', () => {
  test('contains shared section shell, tab bar, and content slot', () => {
    const source = readFileSync(filePath, 'utf8')

    expect(source.includes('SectionPageShell')).toBe(true)
    expect(source.includes('SectionTabBar')).toBe(true)
    expect(source.includes("emit('refresh')")).toBe(true)
    expect(source.includes("'update:activeTab'")).toBe(true)
    expect(source.includes('<slot />')).toBe(true)
  })
})
