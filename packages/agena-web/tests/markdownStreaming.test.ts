import { describe, expect, test } from 'bun:test'
import { createServer } from 'vite'
const vite = await createServer({ configFile: new URL('../vite.config.ts', import.meta.url).pathname, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false } })
const { MarkdownStreamRenderer, renderMarkdown } = await vite.ssrLoadModule('/src/lib/markdown.ts') as typeof import('../src/lib/markdown')
await vite.close()

describe('incremental Markdown semantics', () => {
  test('every streamed prefix preserves lists, fences, tables, math and partial blocks', () => {
    const fixtures = [
      '# Title\n\nA paragraph.\n\nAnother paragraph\n\n- first\n- second\n\n  continuation\n\n## Next\n\nEnd.',
      'Intro\n\n```ts\nconst x = 1\n\n// still inside the fence\n```\n\nAfter\n\nFinal',
      'Intro\n\nA | B\n--- | ---\n1 | 2\n3 | 4\n\nAfter\n\nLast',
      '# Math\n\n$$x^2$$\n\nWords\n\n> quote\n> next\n\nFinal',
      '# 中文\n\n中文🙂\n\nHeading\n=======\n\nFinal',
    ]
    for (const source of fixtures) {
      const renderer = new MarkdownStreamRenderer()
      for (let end = 1; end <= source.length; end++) {
        const prefix = source.slice(0, end)
        expect(renderer.render(prefix).join('')).toBe(renderMarkdown(prefix))
      }
    }
  })
  test('future references and replacement content invalidate earlier interpretation', () => {
    const renderer = new MarkdownStreamRenderer()
    const first = '# Intro\n\n[linked][target]\n\nOther\n\nLast'
    const next = `${first}\n\n[target]: https://example.com\n`
    expect(renderer.render(first).join('')).toBe(renderMarkdown(first))
    expect(renderer.render(next).join('')).toBe(renderMarkdown(next))
    const replacement = '# Replaced\n\nNew\n\nBody\n\nEnd'
    expect(renderer.render(replacement).join('')).toBe(renderMarkdown(replacement))
  })
  test('a long growing document only parses its active tail and reuses completed HTML', () => {
    const renderer = new MarkdownStreamRenderer()
    let body = ''
    let last: string[] = []
    for (let i = 0; i < 1000; i++) {
      body += `Paragraph ${i} with **stable** content.\n\n`
      const parts = renderer.render(body)
      for (const n of new Set([0, Math.floor((last.length - 2) / 2), last.length - 2])) {
        if (n >= 0 && n < last.length - 1) expect(parts[n]).toBe(last[n])
      }
      last = parts
    }
    expect(last.join('')).toBe(renderMarkdown(body))
    expect(renderer.parsedBytes).toBeLessThan(body.length * 5)
    expect(last.length).toBeGreaterThan(900)
  })
})
