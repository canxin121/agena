import { describe, expect, test } from 'bun:test'
import {
  parseAppManifest,
  parseChartSpec,
  splitReplySegments,
  serviceRequestUrl,
} from '../src/features/interactive/model'
import { appFrameDocument } from '../src/features/interactive/frame'

const app = {
  id: 'board',
  title: 'Board',
  html: '<main>Hi</main>',
  services: [{ name: 'api', command: 'python3', args: ['server.py', '--port', '{port}'] }],
}
const chart = { type: 'line', labels: ['一', '二', '三'], series: [{ name: 'Trend', values: [2, null, -3] }] }
const fence = (lang: string, value: unknown) => `\`\`\`${lang}\n${JSON.stringify(value)}\n\`\`\``
describe('rich reply parsing', () => {
  test('completed apps preserve their identity as later text streams in', () => {
    const first = splitReplySegments(`Before\n\n${fence('agena-ui', app)}`, true)
    const next = splitReplySegments(`Before\n\n${fence('agena-ui', app)}\n\nStill streaming`, true)
    expect(first.find((s) => s.kind === 'app')?.key).toBe(next.find((s) => s.kind === 'app')?.key)
    expect(next.map((s) => s.kind)).toEqual(['markdown', 'app', 'markdown'])
  })
  test('user Markdown, open fences, quoted examples and longer fences remain source', () => {
    expect(splitReplySegments(fence('agena-ui', app)).every((s) => s.kind === 'markdown')).toBe(true)
    expect(splitReplySegments(`\`\`\`agena-ui\n${JSON.stringify(app)}`, true).every((s) => s.kind === 'markdown')).toBe(
      true,
    )
    for (const source of [
      `\`\`\`\`markdown\n${fence('agena-ui', app)}\n\`\`\`\``,
      fence('agena-ui', app)
        .split('\n')
        .map((line) => '> ' + line)
        .join('\n'),
    ]) {
      expect(splitReplySegments(source, true).every((s) => s.kind === 'markdown')).toBe(true)
    }
  })
  test('safe charts work without app capabilities; invalid and duplicate apps retain source', () => {
    expect(splitReplySegments(fence('agena-chart', chart))[0]?.kind).toBe('chart')
    expect(splitReplySegments('```agena-chart\n{"invalid":true}\n```')[0]).toMatchObject({
      kind: 'markdown',
      content: '```agena-chart\n{"invalid":true}\n```',
    })
    const segments = splitReplySegments(`${fence('agena-ui', app)}\n${fence('agena-ui', app)}`, true)
    expect(segments.filter((s) => s.kind === 'app')).toHaveLength(1)
    expect(segments.find((s) => s.kind === 'markdown' && s.error)?.kind).toBe('markdown')
  })
  test('schemas reject mismatched data, invalid numbers and excess capabilities', () => {
    expect(() => parseChartSpec({ ...chart, series: [{ name: 'bad', values: [Infinity, 2, 3] }] })).toThrow()
    expect(() => parseChartSpec({ ...chart, series: [{ name: 'bad', values: [1] }] })).toThrow()
    expect(() => parseChartSpec({ ...chart, type: 'pie' })).toThrow()
    expect(() => parseAppManifest({ ...app, services: Array(5).fill(app.services[0]) })).toThrow()
    expect(() => parseAppManifest({ ...app, services: [app.services[0], app.services[0]] })).toThrow()
  })
})

describe('iframe capability boundaries', () => {
  test('proxy traversal and arbitrary origins are rejected before authenticated fetch', () => {
    const base = '/api/v1/workbench/preview/s/app-123/'
    expect(serviceRequestUrl(base, '/tasks?q=one%20two')).toBe(base + 'tasks?q=one%20two')
    for (const path of [
      'https://evil.test',
      '//evil.test',
      '/../git',
      '/%2e%2e/git',
      '/%252e%252e/git',
      '/a%2fb',
      '/a\\b',
      '/a#b',
    ])
      expect(() => serviceRequestUrl(base, path)).toThrow()
    expect(() => serviceRequestUrl('/api/v1/server/', '/tasks')).toThrow()
  })
  test('bootstrap state cannot terminate its script and generated HTML follows CSP', () => {
    const document = appFrameDocument(
      app,
      'secret-channel',
      { text: '</script><script>evil()</script>' },
      'zh-CN',
      'dark',
    )
    expect(document.indexOf('Content-Security-Policy')).toBeLessThan(document.indexOf(app.html))
    expect(document).toContain("connect-src 'none'")
    expect(document).toContain('event.source!==parent')
    expect(document).not.toContain('</script><script>evil()')
    expect(document).toContain('\\u003c/script>')
  })
})
