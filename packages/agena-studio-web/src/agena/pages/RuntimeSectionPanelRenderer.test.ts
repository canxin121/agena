import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'

const filePath = '/home/canxin/Git/ai/agena/packages/agena-studio-web/src/agena/pages/RuntimeSectionPanelRenderer.vue'

describe('RuntimeSectionPanelRenderer', () => {
  test('delegates runtime tabs to dedicated page content components', () => {
    const source = readFileSync(filePath, 'utf8')

    expect(source.includes('RuntimeOverviewPageContent')).toBe(true)
    expect(source.includes('RuntimeWorkflowPageContent')).toBe(true)
    expect(source.includes('RuntimeInspectorPageContent')).toBe(true)
    expect(source.includes('RuntimeSkillsPageContent')).toBe(true)
    expect(source.includes('RuntimeOperatorPageContent')).toBe(true)
    expect(source.includes("props.activeTab === 'overview'")).toBe(true)
    expect(source.includes("props.activeTab === 'workflow'")).toBe(true)
    expect(source.includes("props.activeTab === 'mcp'")).toBe(true)
    expect(source.includes("props.activeTab === 'lsp'")).toBe(true)
    expect(source.includes("props.activeTab === 'skills'")).toBe(true)
  })
})
