import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'

const filePath = '/home/canxin/Git/ai/agena/packages/agena-studio-web/src/agena/pages/SectionTabBar.vue'

describe('SectionTabBar', () => {
  test('contains shared tab bar structure and active tab emit', () => {
    const source = readFileSync(filePath, 'utf8')

    expect(source.includes('props.tabs.length')).toBe(true)
    expect(source.includes("props.activeTab === tab.id")).toBe(true)
    expect(source.includes("emit('update:activeTab', tab.id)")).toBe(true)
  })
})
