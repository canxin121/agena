import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'

const filePath = '/home/canxin/Git/ai/agena/packages/agena-studio-web/src/agena/pages/RuntimeSectionLayout.vue'

describe('RuntimeSectionLayout', () => {
  test('contains runtime section shell, tab bar, and refresh/reload actions', () => {
    const source = readFileSync(filePath, 'utf8')

    expect(source.includes('SectionPageShell')).toBe(true)
    expect(source.includes('SectionTabBar')).toBe(true)
    expect(source.includes("emit('refresh')")).toBe(true)
    expect(source.includes("emit('reload')")).toBe(true)
    expect(source.includes("'update:activeTab'")).toBe(true)
    expect(source.includes('<slot />')).toBe(true)
  })
})
