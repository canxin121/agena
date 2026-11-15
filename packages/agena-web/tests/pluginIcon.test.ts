import { describe, expect, test } from 'bun:test'
import { MAX_PLUGIN_ICON_BYTES, pluginIconSource } from '../src/lib/pluginIcon'

describe('plugin-owned image URLs', () => {
  test('accepts HTTP(S) and same-origin resources, preserving signed URLs', () => {
    for (const source of [
      'https://example.test/icon.png?signature=A%2FB#logo',
      'http://127.0.0.1:8080/logo.webp',
      '/plugin-assets/demo/r1/logo.svg',
      '/images/plugin.png?size=48',
    ])
      expect(pluginIconSource(`  ${source}  `)).toBe(source)
  })

  test('accepts base64 image data, with optional UTF-8 headers and padding', () => {
    for (const mime of ['png', 'jpeg', 'gif', 'webp', 'avif', 'svg+xml', 'x-icon', 'vnd.microsoft.icon']) {
      for (const payload of ['Zg==', 'Zm8=', 'Zm9v', 'Zg', 'Zm8']) {
        const source = `data:image/${mime};base64,${payload}`
        expect(pluginIconSource(source)).toBe(source)
      }
    }
    for (const header of ['charset=utf-8', 'charset=utf8', 'utf8']) {
      const source = `data:image/svg+xml;${header};base64,PHN2Zy8+`
      expect(pluginIconSource(source)).toBe(source)
    }
    expect(pluginIconSource('DATA:IMAGE/PNG;BASE64,Zg==')).toBe('DATA:IMAGE/PNG;BASE64,Zg==')
  })

  test('accepts percent-encoded SVG data without injecting its markup', () => {
    const source = 'data:image/svg+xml;charset=utf-8,%3Csvg%20xmlns%3D%22http%3A%2F%2Fwww.w3.org%2F2000%2Fsvg%22%2F%3E'
    expect(pluginIconSource(source)).toBe(source)
    expect(pluginIconSource('data:image/svg+xml,%3Csvg%2F%3E')).toBe('data:image/svg+xml,%3Csvg%2F%3E')
  })

  test('rejects non-image schemes, credentials, network-path references and control characters', () => {
    for (const source of [
      'javascript:alert(1)',
      'file:///tmp/icon.png',
      'blob:https://example.test/image',
      'data:text/html;base64,PGh0bWw+',
      'data:image/html;base64,PGh0bWw+',
      '//example.test/icon.png',
      '/\\example.test/icon.png',
      'https://example.test\\@other.test/icon.png',
      'https://user:password@example.test/icon.png',
      'https://user@example.test/icon.png',
      'https:example.test/icon.png',
      'http:/example.test/icon.png',
      'https://',
      'https://exa\nmple.test/logo.png',
      'https://example.test/logo icon.png',
      '/images/\u0000icon.png',
    ])
      expect(pluginIconSource(source)).toBeUndefined()
  })

  test('rejects malformed data encodings and unsupported parameters', () => {
    for (const source of [
      'data:image/png;base64,',
      'data:image/png;base64,A',
      'data:image/png;base64,AAA==',
      'data:image/png;base64,AAAA===',
      'data:image/png;base64,AA=A',
      'data:image/png;base64,AA-A',
      'data:image/png;base64,AA_A',
      'data:image/png;base64,AA A',
      'data:image/png,not-base64',
      'data:image/svg+xml,',
      'data:image/svg+xml,%G0',
      'data:image/svg+xml,%3',
      'data:image/svg+xml;charset=ascii,%3Csvg%2F%3E',
      'data:image/svg+xml;utf8;charset=utf-8,%3Csvg%2F%3E',
      'data:image/svg+xml;base64;utf8,PHN2Zy8+',
      'data:image/svg+xml;unknown,PHN2Zy8+',
    ])
      expect(pluginIconSource(source)).toBeUndefined()
  })

  test('bounds source size in UTF-8 bytes and handles absent metadata', () => {
    for (const source of [undefined, null, '', '  ', {}, 42]) expect(pluginIconSource(source)).toBeUndefined()
    const prefix = 'https://example.test/'
    expect(pluginIconSource(prefix + 'a'.repeat(MAX_PLUGIN_ICON_BYTES - prefix.length))).toBeDefined()
    expect(pluginIconSource(prefix + 'a'.repeat(MAX_PLUGIN_ICON_BYTES))).toBeUndefined()
    expect(pluginIconSource(prefix + 'é'.repeat(MAX_PLUGIN_ICON_BYTES / 2))).toBeUndefined()
  })
})
