import { describe, expect, test } from 'bun:test'
import { renderMarkdown } from '../src/lib/markdown'

describe('markdown resource hydration', () => {
  test('images never issue an unauthenticated request before Vue hydration', () => {
    const rendered = renderMarkdown('![generated](/api/v1/sessions/46/media?path=generated.png)')
    expect(rendered).toContain('data-oc-md-raw-src="/api/v1/sessions/46/media?path=generated.png"')
    expect(rendered).toContain('src="data:image/gif;base64,')
    expect(rendered).not.toContain('<img src="/api/')
  })
  test('audio and video URLs are also deferred until authenticated hydration', () => {
    for (const [tag, path] of [
      ['audio', '/state/clip.wav'],
      ['video', '/state/clip.webm'],
    ] as const) {
      const rendered = renderMarkdown(`![clip](${path})`)
      expect(rendered).toContain(`<${tag} `)
      expect(rendered).toContain(`<source data-oc-md-raw-src="${path}"`)
      expect(rendered).not.toContain(`<source src="${path}"`)
    }
  })
})
