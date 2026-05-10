import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'

const filePath = '/home/canxin/Git/ai/agena/packages/agena-studio-web/src/agena/pages/WorkspaceSectionLayout.vue'

describe('WorkspaceSectionLayout', () => {
  test('contains shared page shell and workspace header actions', () => {
    const source = readFileSync(filePath, 'utf8')

    expect(source.includes('SectionPageShell')).toBe(true)
    expect(source.includes("emit('refresh')")).toBe(true)
    expect(source.includes("emit('root')")).toBe(true)
    expect(source.includes('<slot />')).toBe(true)
  })
})
