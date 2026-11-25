import type { McpAppRequest } from './api'

export const MCP_APP_PROTOCOL_VERSION = '2026-01-26'
export const MCP_APP_MAX_HEIGHT = 1200

const REQUEST_LIMIT = 128 * 1024
const encoder = new TextEncoder()

type RpcId = string | number
type GuestMessage = { id?: RpcId; method: string; params: Record<string, unknown> }

function record(value: unknown): value is Record<string, unknown> {
  return (
    !!value && typeof value === 'object' && !Array.isArray(value) && Object.getPrototypeOf(value) === Object.prototype
  )
}

export function mcpAppGuestMessage(
  event: Pick<MessageEvent, 'source' | 'origin' | 'data'>,
  frame: Window | null | undefined,
): GuestMessage | null {
  if (!frame || event.source !== frame || event.origin !== 'null') return null
  const message = event.data
  if (!record(message) || message.jsonrpc !== '2.0' || typeof message.method !== 'string') return null
  if (
    message.method.length > 80 ||
    Object.keys(message).some((key) => !['jsonrpc', 'id', 'method', 'params'].includes(key))
  )
    return null
  const id = message.id
  if (
    id !== undefined &&
    !(typeof id === 'string' && id.length <= 64) &&
    !(typeof id === 'number' && Number.isSafeInteger(id))
  )
    return null
  if (message.params !== undefined && !record(message.params)) return null
  try {
    if (encoder.encode(JSON.stringify(message)).byteLength > REQUEST_LIMIT) return null
  } catch {
    return null
  }
  return { id, method: message.method, params: message.params ?? {} }
}

export function mcpAppRequest(message: GuestMessage): McpAppRequest | null {
  const params = message.params
  if (message.method === 'tools/call') {
    if (typeof params.name !== 'string' || !params.name.trim() || params.name.length > 256) return null
    if (params.arguments !== undefined && !record(params.arguments)) return null
    if (Object.keys(params).some((key) => !['name', 'arguments', '_meta'].includes(key))) return null
    return { method: message.method, params: { name: params.name, arguments: params.arguments ?? {} } }
  }
  if (message.method === 'resources/read') {
    if (typeof params.uri !== 'string' || !params.uri.trim() || params.uri.length > 2048) return null
    if (Object.keys(params).some((key) => !['uri', '_meta'].includes(key))) return null
    return { method: message.method, params: { uri: params.uri } }
  }
  return null
}

export function mcpAppHeight(height: unknown): number | undefined {
  if (typeof height !== 'number' || !Number.isFinite(height)) return undefined
  return Math.max(140, Math.min(MCP_APP_MAX_HEIGHT, Math.ceil(height)))
}

const SHARED_POLICY =
  "default-src 'none'; connect-src 'none'; base-uri 'none'; object-src 'none'; form-action 'none'; worker-src 'none'"

export function mcpAppGuestDocument(html: string) {
  const policy = `${SHARED_POLICY}; script-src 'unsafe-inline' blob:; style-src 'unsafe-inline'; img-src data: blob:; font-src data: blob:; media-src data: blob:; frame-src 'none'`
  return `<!doctype html><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${policy}">${html}`
}

/** The proxy is trusted code on an opaque origin. Its frame policy bounds
 * guest navigation, and the guest's inner policy also bounds its own embeds.
 * MCP App HTML only enters the inner frame via a Blob. */
export function mcpAppProxyDocument(hostOrigin: string) {
  const origin = JSON.stringify(hostOrigin).replace(/</g, '\\u003c')
  const policy = `${SHARED_POLICY}; script-src 'unsafe-inline' blob:; style-src 'unsafe-inline'; img-src data: blob:; font-src data: blob:; media-src data: blob:; frame-src blob:`
  return `<!doctype html><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${policy}">
<style>html,body{width:100%;height:100%;margin:0}iframe{display:block;width:100%;height:100%;border:0}</style>
<body><script>
(() => {
  const origin = ${origin};
  const send = (message) => parent.postMessage(message, origin);
  let guest, url, loads = 0;
  addEventListener('message', (event) => {
    if (event.source === parent && event.origin === origin) {
      if (!guest && event.data?.method === 'ui/notifications/sandbox-resource-ready') {
        if (typeof event.data.params?.html !== 'string') return;
        guest = document.createElement('iframe');
        guest.title = 'MCP App';
        guest.setAttribute('sandbox', 'allow-scripts');
        guest.referrerPolicy = 'no-referrer';
        url = URL.createObjectURL(new Blob([event.data.params.html], {type:'text/html;charset=utf-8'}));
        guest.addEventListener('load', () => {
          URL.revokeObjectURL(url);
          if (++loads > 1) {
            guest.remove();
            send({jsonrpc:'2.0', method:'agena/guest-replaced'});
          }
        });
        guest.src = url;
        document.body.appendChild(guest);
      } else if (guest?.isConnected) guest.contentWindow.postMessage(event.data, '*');
    } else if (guest?.isConnected && event.source === guest.contentWindow && event.origin === 'null') {
      send(event.data);
    }
  });
  send({jsonrpc:'2.0', method:'ui/notifications/sandbox-proxy-ready'});
})();
</script></body>`
}
