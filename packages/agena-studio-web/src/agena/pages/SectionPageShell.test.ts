import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'

const filePath = '/home/canxin/Git/ai/agena/packages/agena-studio-web/src/agena/pages/SectionPageShell.vue'

describe('SectionPageShell', () => {
  test('contains shared page shell structure and refresh slot', () => {
    const source = readFileSync(filePath, 'utf8')

    expect(source.includes('page-title')).toBe(true)
    expect(source.includes('page-description')).toBe(true)
    expect(source.includes('slot name="header-actions"')).toBe(true)
    expect(source.includes("emit('refresh')")).toBe(true)
    expect(source.includes('props.actionError')).toBe(true)
    expect(source.includes('props.actionMessage')).toBe(true)
  })
})
