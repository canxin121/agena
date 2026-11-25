import { describe, expect, test } from 'bun:test'
import {
  mcpAppGuestDocument,
  mcpAppGuestMessage,
  mcpAppHeight,
  mcpAppProxyDocument,
  mcpAppRequest,
} from '../src/features/mcpApps/bridge'

const frame = {} as Window
const request = { jsonrpc: '2.0', id: 1, method: 'tools/call', params: { name: 'refresh', arguments: { page: 2 } } }
const fromApp = (data: unknown, origin = 'null', source: Window = frame) =>
  mcpAppGuestMessage({ data, origin, source }, frame)

describe('MCP Apps bridge', () => {
  test('only accepts bounded RPC from the opaque sandbox window', () => {
    expect(fromApp(request)?.method).toBe('tools/call')
    expect(fromApp(request, 'https://other.test')).toBeNull()
    expect(fromApp(request, 'null', {} as Window)).toBeNull()
    expect(fromApp({ ...request, params: { arguments: { giant: 'x'.repeat(128 * 1024) } } })).toBeNull()
    expect(fromApp({ ...request, id: { unexpected: true } })).toBeNull()
    expect(fromApp({ ...request, params: [] })).toBeNull()
  })

  test('narrows guest messages to same-connection methods and fields', () => {
    expect(mcpAppRequest(fromApp(request)!)).toEqual({ method: request.method, params: request.params })
    expect(
      mcpAppRequest(fromApp({ ...request, params: { ...request.params, server: 'different-server' } })!),
    ).toBeNull()
    expect(mcpAppRequest(fromApp({ ...request, method: 'ui/message' })!)).toBeNull()
  })

  test('keeps both iframe policies ahead of application HTML and clamps resize messages', () => {
    const html = '<article>App</article>'
    const guest = mcpAppGuestDocument(html)
    expect(guest.indexOf('Content-Security-Policy')).toBeLessThan(guest.indexOf(html))
    expect(guest).toContain("connect-src 'none'")
    expect(guest).toContain("frame-src 'none'")
    const proxy = mcpAppProxyDocument('https://agena.test')
    expect(proxy).toContain('frame-src blob:')
    expect(proxy).toContain("setAttribute('sandbox', 'allow-scripts')")
    expect(proxy).not.toContain('allow-same-origin')
    expect(mcpAppHeight(Infinity)).toBeUndefined()
    expect(mcpAppHeight(-1)).toBe(140)
    expect(mcpAppHeight(8000)).toBe(1200)
  })
})
