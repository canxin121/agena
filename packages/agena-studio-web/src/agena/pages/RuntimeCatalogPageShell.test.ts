import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'

const filePath = '/home/canxin/Git/ai/agena/packages/agena-studio-web/src/agena/pages/RuntimeCatalogPageShell.vue'

describe('RuntimeCatalogPageShell', () => {
  test('contains shared catalog query shell and sections panel wiring', () => {
    const source = readFileSync(filePath, 'utf8')

    expect(source.includes('RuntimeCatalogSectionsPanel')).toBe(true)
    expect(source.includes("'update:queryValue'" ) || source.includes("'update:queryValue':")).toBe(true)
    expect(source.includes('runtime-catalog-query')).toBe(true)
    expect(source.includes('Open Config Root')).toBe(true)
    expect(source.includes('Open Logs')).toBe(true)
    expect(source.includes('props.queryPlaceholder')).toBe(true)
    expect(source.includes('props.sections')).toBe(true)
  })
})
