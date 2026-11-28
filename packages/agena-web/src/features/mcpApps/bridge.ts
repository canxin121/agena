import type { McpAppRequest, McpAppServerRequest, McpAppViewCall, McpAppViewPrompt } from './api'
import { MAX_MCP_APP_MEDIA_BYTES, mcpAppMessageContent, type McpAppMessageContent } from './media'
import { mcpAppResourceContents, mcpAppResourceUri, record } from './resources'

export const MCP_APP_PROTOCOL_VERSION = '2026-01-26'
export const MCP_APP_MAX_HEIGHT = 1200
export const MCP_APP_TEARDOWN_ID = 'agena-teardown'
export const MCP_TASKS_EXTENSION = 'io.modelcontextprotocol/tasks'

const REQUEST_LIMIT = 128 * 1024
const MEDIA_REQUEST_LIMIT = REQUEST_LIMIT + Math.ceil(MAX_MCP_APP_MEDIA_BYTES / 3) * 4
const DOWNLOAD_LIMIT = 1024 * 1024
const DOWNLOAD_FILE_LIMIT = 4
const encoder = new TextEncoder()

export type RpcId = string | number
type GuestMessage = { id?: RpcId; method: string; params: Record<string, unknown> }
type GuestEvent = Pick<MessageEvent, 'source' | 'origin' | 'data'>
export type McpAppGuestResponse =
  | { id: RpcId; result: Record<string, unknown> }
  | { id: RpcId; error: { code: number; message: string } }
export type McpAppMode = 'inline' | 'fullscreen' | 'pip'
export const MCP_APP_MODES: McpAppMode[] = ['inline', 'fullscreen', 'pip']

export type McpAppDownload = { filename: string; blob: Blob } | { filename: string; uri: string }
export type McpAppAction =
  | { kind: 'link'; uri: string }
  | ({ kind: 'message' | 'context' } & McpAppMessageContent)
  | { kind: 'download'; files: McpAppDownload[] }

function guestEnvelope(event: GuestEvent, frame: Window | null | undefined) {
  const message: unknown = event.data
  if (!frame || event.source !== frame || event.origin !== 'null' || !record(message) || message.jsonrpc !== '2.0')
    return null
  return message
}

export function mcpAppTeardownResponse(event: GuestEvent, frame: Window | null | undefined) {
  const response = guestEnvelope(event, frame)
  if (response?.id !== MCP_APP_TEARDOWN_ID || Object.hasOwn(response, 'method')) return false
  if (Object.hasOwn(response, 'result')) return record(response.result) && !Object.hasOwn(response, 'error')
  return record(response.error)
}

export function mcpAppGuestResponse(
  event: GuestEvent,
  frame: Window | null | undefined,
): McpAppGuestResponse | null {
  const message = guestEnvelope(event, frame)
  if (!message || Object.hasOwn(message, 'method')) return null
  const { id, result, error } = message
  if (!(typeof id === 'string' && id.length <= 64) && !(typeof id === 'number' && Number.isSafeInteger(id))) {
    return null
  }
  if (record(result) && !Object.hasOwn(message, 'error')) {
    try {
      if (encoder.encode(JSON.stringify(result)).byteLength <= REQUEST_LIMIT) return { id, result }
    } catch {
      return null
    }
  }
  if (!Object.hasOwn(message, 'result') && record(error) &&
      typeof error.code === 'number' && Number.isSafeInteger(error.code) &&
      typeof error.message === 'string' && error.message.length <= 2048) {
    return { id, error: { code: error.code, message: error.message } }
  }
  return null
}

export function mcpAppGuestMessage(event: GuestEvent, frame: Window | null | undefined): GuestMessage | null {
  const message = guestEnvelope(event, frame)
  if (!message || typeof message.method !== 'string') return null
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
    const limit =
      message.method === 'ui/message' || message.method === 'ui/update-model-context'
        ? MEDIA_REQUEST_LIMIT
        : REQUEST_LIMIT
    const json = JSON.stringify(message)
    if (json.length > limit || encoder.encode(json).byteLength > limit) return null
  } catch {
    return null
  }
  return { id, method: message.method, params: message.params ?? {} }
}

export function mcpAppRequest(message: GuestMessage): McpAppRequest | null {
  const params = message.params
  if (message.method === 'tools/list' || message.method === 'resources/list' || message.method === 'resources/templates/list') {
    if (params.cursor !== undefined && (typeof params.cursor !== 'string' || params.cursor.length > 2048)) return null
    if (Object.keys(params).some((key) => !['cursor', '_meta'].includes(key))) return null
    return { method: message.method, params: { cursor: params.cursor } }
  }
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

export function mcpAppServerRequest(message: GuestMessage, extension: boolean): McpAppServerRequest | null {
  const { method, params } = message
  if (method === 'tasks/update') {
    if (!extension || !taskId(params.taskId) || !record(params.inputResponses)) return null
    if (Object.keys(params).some((key) => !['taskId', 'inputResponses', '_meta'].includes(key))) return null
    return { method, params: { taskId: params.taskId, inputResponses: params.inputResponses } }
  }
  if (method === 'tasks/get' || method === 'tasks/result' || method === 'tasks/cancel') {
    if (!taskId(params.taskId)) return null
    if (Object.keys(params).some((key) => !['taskId', '_meta'].includes(key))) return null
    return { method, params: { taskId: params.taskId } }
  }
  if (method === 'resources/read') {
    const read = mcpAppRequest(message)
    return read?.method === 'resources/read' ? read : null
  }
  if (method === 'sampling/createMessage') {
    const fields = [
      '_meta', 'messages', 'modelPreferences', 'systemPrompt', 'includeContext', 'temperature',
      'maxTokens', 'stopSequences', 'metadata', 'tools', 'toolChoice',
    ]
    if (!Array.isArray(params.messages) || typeof params.maxTokens !== 'number' ||
        Object.keys(params).some((key) => !fields.includes(key))) return null
    const { _meta, ...sampling } = params
    return { method, params: sampling }
  }
  if (method !== 'tools/call') return null
  const { task, ...toolParams } = params
  const tool = mcpAppRequest({ ...message, params: toolParams })
  if (tool?.method !== 'tools/call') return null
  if (task === undefined) return tool
  if (!record(task) || Object.keys(task).some((key) => key !== 'ttl')) return null
  const ttl = task.ttl
  if (ttl !== undefined && (typeof ttl !== 'number' || !Number.isSafeInteger(ttl) || ttl < 0)) return null
  return { method, params: { ...tool.params, task: { ttl } } }
}

function taskId(value: unknown): value is string {
  return typeof value === 'string' && value.length > 0 && value.length <= 128
}

export function mcpAppProgressToken(meta: unknown): RpcId | undefined {
  if (!record(meta)) return undefined
  const token = meta.progressToken
  if (typeof token === 'string' && token.length <= 64) return token
  if (typeof token === 'number' && Number.isSafeInteger(token)) return token
}

export function mcpAppViewRequest(call: Exclude<McpAppViewCall, McpAppViewPrompt>) {
  if ('params' in call) return { method: call.method, params: call.params }
  if ('method' in call) {
    const params = { taskId: call.taskId, ...('inputResponses' in call && { inputResponses: call.inputResponses }) }
    return { method: call.method, params }
  }
  return {
    method: 'tools/call' as const,
    params: {
      name: call.name,
      arguments: call.arguments,
      ...(call.task && { task: call.task }),
      ...(call.requestState !== undefined && { requestState: call.requestState }),
      ...(call.inputResponses !== undefined && { inputResponses: call.inputResponses }),
    },
  }
}

const consoleLevels = new Map<string, 'debug' | 'info' | 'warn' | 'error'>([
  ['debug', 'debug'],
  ['info', 'info'],
  ['notice', 'info'],
  ['warning', 'warn'],
  ['error', 'error'],
  ['critical', 'error'],
  ['alert', 'error'],
  ['emergency', 'error'],
])

export function mcpAppLog(message: GuestMessage) {
  if (message.method !== 'notifications/message' || message.id !== undefined) return null
  const { level, logger = '', data } = message.params
  const consoleLevel = typeof level === 'string' ? consoleLevels.get(level) : undefined
  if (!consoleLevel || typeof logger !== 'string' || logger.length > 128 || data === undefined) return null
  // Keep a guest's diagnostics local to the browser console and small enough to inspect.
  if (encoder.encode(JSON.stringify(data)).byteLength > 4096) return null
  return { level: consoleLevel, logger, data }
}

function openableLink(value: unknown) {
  if (typeof value !== 'string' || value.length > 2048 || /[\x00-\x20\x7f]/.test(value)) return null
  try {
    const url = new URL(value)
    if ((url.protocol === 'https:' || url.protocol === 'http:') && !url.username && !url.password) {
      return url.href
    }
  } catch {
    return null
  }
  return null
}

function downloadFilename(uri: URL, name?: unknown) {
  const segment = uri.pathname.split('/').pop() ?? ''
  let filename = typeof name === 'string' && name.trim() ? name : segment
  if (filename === segment) {
    try {
      filename = decodeURIComponent(segment)
    } catch {
      // Preserve a filename with malformed percent encoding as literal text.
    }
  }
  return (
    filename
      .split(/[\\/]/)
      .pop()
      ?.replace(/[<>:"|?*\u0000-\u001f\u007f\u202a-\u202e\u2066-\u2069]/g, '_')
      .trim()
      .slice(0, 160)
      .replace(/^\.+|[. ]+$/g, '') || 'mcp-app-download'
  )
}

function downloadBlob(resource: Record<string, unknown>) {
  // Keep even active document types inert if a browser opens the generated URL.
  const binary = { type: 'application/octet-stream' }
  const { text, blob } = resource
  if (typeof text === 'string' && blob === undefined && text.length <= DOWNLOAD_LIMIT) {
    const data = new Blob([text], binary)
    return data.size <= DOWNLOAD_LIMIT ? data : null
  }
  if (typeof blob === 'string' && text === undefined && blob.length <= Math.ceil(DOWNLOAD_LIMIT / 3) * 4) {
    try {
      const bytes = Uint8Array.from(atob(blob), (character) => character.charCodeAt(0))
      return bytes.length <= DOWNLOAD_LIMIT ? new Blob([bytes], binary) : null
    } catch {
      return null
    }
  }
  return null
}

export function mcpAppResourceDownload(result: unknown, uri: string) {
  const resource = mcpAppResourceContents(result, uri)
  return resource ? downloadBlob(resource) : null
}

function mcpAppDownloads(contents: unknown): McpAppDownload[] | null {
  if (!Array.isArray(contents) || !contents.length || contents.length > DOWNLOAD_FILE_LIMIT) return null
  const files: McpAppDownload[] = []
  for (const content of contents) {
    if (!record(content)) return null
    if (content.type === 'resource_link' && typeof content.uri === 'string') {
      const uri = mcpAppResourceUri(content.uri)
      if (!uri) return null
      files.push({ filename: downloadFilename(uri, content.name), uri: content.uri })
    } else if (content.type === 'resource' && record(content.resource)) {
      const uri = mcpAppResourceUri(content.resource.uri)
      const blob = downloadBlob(content.resource)
      if (!uri || !blob) return null
      files.push({ filename: downloadFilename(uri), blob })
    } else {
      return null
    }
  }
  return files
}

export function mcpAppAction(message: GuestMessage): McpAppAction | null {
  const { method, params } = message
  if (method === 'ui/open-link') {
    const uri = openableLink(params.url)
    return uri ? { kind: 'link', uri } : null
  }
  if (method === 'ui/message') {
    const content = params.role === 'user' ? mcpAppMessageContent(params.content) : null
    return content && (content.text || content.media || content.resourceLinks) ? { kind: 'message', ...content } : null
  }
  if (method === 'ui/update-model-context') {
    if (params.structuredContent !== undefined && !record(params.structuredContent)) return null
    const structured = params.structuredContent ? JSON.stringify(params.structuredContent, null, 2) : ''
    const content = mcpAppMessageContent(params.content === undefined ? [] : params.content, structured)
    return content ? { kind: 'context', ...content } : null
  }
  if (method === 'ui/download-file') {
    const files = mcpAppDownloads(params.contents)
    return files ? { kind: 'download', files } : null
  }
  return null
}

export function mcpAppModes(capabilities: unknown): McpAppMode[] {
  if (!record(capabilities) || !Array.isArray(capabilities.availableDisplayModes)) return ['inline']
  return MCP_APP_MODES.filter((mode) => (capabilities.availableDisplayModes as unknown[]).includes(mode))
}

export function mcpAppTaskExtension(capabilities: unknown) {
  if (!record(capabilities)) return false
  return [capabilities.experimental, capabilities.extensions].some(
    (extensions) => record(extensions) && record(extensions[MCP_TASKS_EXTENSION]),
  )
}

export function mcpAppHeight(height: unknown): number | undefined {
  if (typeof height !== 'number' || !Number.isFinite(height)) return undefined
  return Math.max(140, Math.min(MCP_APP_MAX_HEIGHT, Math.ceil(height)))
}

const SHARED_POLICY = "default-src 'none'; object-src 'none'; form-action 'none'; worker-src 'none'"

export type McpAppPermission = 'camera' | 'microphone' | 'geolocation' | 'clipboard-write'
const PERMISSION_MAP: ReadonlyArray<readonly [string, McpAppPermission]> = [
  ['camera', 'camera'],
  ['microphone', 'microphone'],
  ['geolocation', 'geolocation'],
  ['clipboardWrite', 'clipboard-write'],
]
const VALID_PERMISSIONS = new Set<string>(PERMISSION_MAP.map(([, feature]) => feature))

export type McpAppCsp = {
  connectDomains: string[]
  resourceDomains: string[]
  baseUriDomains: string[]
  frameDomains: string[]
}

export function mcpAppSandboxPermissions(permissions?: readonly McpAppPermission[]): Record<string, Record<string, never>> {
  const granted: Record<string, Record<string, never>> = {}
  if (!permissions) return granted
  for (const [specKey, feature] of PERMISSION_MAP) {
    if (permissions.includes(feature)) granted[specKey] = {}
  }
  return granted
}

function validWildcardBaseDomain(domain: string) {
  if (
    !domain ||
    domain.length > 253 ||
    domain === 'localhost' ||
    domain.endsWith('.localhost') ||
    domain.endsWith('.local') ||
    domain.endsWith('.internal') ||
    /^[0-9.]+$/.test(domain) ||
    domain.includes(':')
  ) {
    return false
  }
  const labels = domain.split('.')
  if (labels.length < 2) return false
  return labels.every((label) => /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/i.test(label))
}

function sameOrSubdomain(host: string, base: string) {
  return host === base || host.endsWith(`.${base}`)
}

function parseWildcardOrigin(origin: string, websocket: boolean, hostHostname: string): string | null {
  const match = /^(https|wss):\/\/\*\.([^/?#@*]+)\/?$/i.exec(origin)
  if (!match) return null
  const scheme = match[1]!.toLowerCase()
  if (scheme === 'wss' && !websocket) return null
  const authority = match[2]!
  let host = authority
  let port: number | undefined
  const colon = authority.lastIndexOf(':')
  if (colon !== -1) {
    const portText = authority.slice(colon + 1)
    if (!/^[0-9]{1,5}$/.test(portText)) return null
    port = Number(portText)
    if (!Number.isInteger(port) || port < 1 || port > 65535) return null
    host = authority.slice(0, colon)
  }
  const base = host.toLowerCase()
  if (!validWildcardBaseDomain(base)) return null
  if (hostHostname && sameOrSubdomain(hostHostname, base)) return null
  return port !== undefined && port !== 443 ? `${scheme}://*.${base}:${port}` : `${scheme}://*.${base}`
}

export function mcpAppBrowserPolicy(
  ui: unknown,
  hostOrigin: string,
): { csp: McpAppCsp; permissions: McpAppPermission[]; prefersBorder: boolean } {
  const meta = record(ui) ? ui : {}
  const declared = record(meta.csp) ? meta.csp : {}
  let hostHostname = ''
  try {
    hostHostname = new URL(hostOrigin).hostname.toLowerCase()
  } catch {
    hostHostname = ''
  }
  const origins = (value: unknown, websocket: boolean) => {
    if (!Array.isArray(value)) return []
    const result = new Set<string>()
    for (const origin of value.slice(0, 12)) {
      if (typeof origin !== 'string' || origin.length > 256 || !/^[a-z0-9:/.[\]_*+-]+$/i.test(origin)) continue
      if (origin.includes('*')) {
        const wildcard = parseWildcardOrigin(origin, websocket, hostHostname)
        if (wildcard) result.add(wildcard)
        continue
      }
      if (!/^[a-z0-9:/.[\]_-]+$/i.test(origin)) continue
      try {
        const url = new URL(origin)
        const protocol = url.protocol
        const isHttp = protocol === 'http:' || protocol === 'https:'
        if (!isHttp && !(websocket && (protocol === 'ws:' || protocol === 'wss:'))) continue
        if (url.pathname !== '/' || url.username || url.password || url.search || url.hash) continue
        const httpOrigin = url.origin.replace(/^wss:/, 'https:').replace(/^ws:/, 'http:')
        if (httpOrigin !== hostOrigin) result.add(url.origin)
      } catch {
        continue
      }
    }
    return [...result]
  }
  const declaredPermissions = record(meta.permissions) ? meta.permissions : {}
  const permissions: McpAppPermission[] = []
  for (const [specKey, feature] of PERMISSION_MAP) {
    const entry = declaredPermissions[specKey]
    if (record(entry) || entry === true) permissions.push(feature)
  }
  return {
    csp: {
      connectDomains: origins(declared.connectDomains, true),
      resourceDomains: origins(declared.resourceDomains, false),
      baseUriDomains: origins(declared.baseUriDomains, false),
      frameDomains: origins(declared.frameDomains, false),
    },
    permissions,
    prefersBorder: meta.prefersBorder !== false,
  }
}

function documentPrefix(csp: McpAppCsp, frames: string) {
  const connect = csp.connectDomains.join(' ') || "'none'"
  const base = csp.baseUriDomains.join(' ') || "'none'"
  const resource = csp.resourceDomains.join(' ')
  const policy = `${SHARED_POLICY}; connect-src ${connect}; base-uri ${base}; script-src 'unsafe-inline' blob: ${resource}; style-src 'unsafe-inline' ${resource}; img-src data: blob: ${resource}; font-src data: blob: ${resource}; media-src data: blob: ${resource}; frame-src ${frames}`
  return `<!doctype html><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${policy}">`
}

const guestBootstrap = `(token) => {
  const proxy = parent;
  const channel = new MessageChannel();
  channel.port1.onmessage = ({data}) => {
    const event = new MessageEvent('message', {data, origin: 'null'});
    // Firefox can reject an opaque parent WindowProxy in MessageEventInit.
    Object.defineProperty(event, 'source', {value: proxy});
    dispatchEvent(event);
  };
  addEventListener('message', (event) => {
    const id = event.data?.agenaMcpAppChallenge;
    if (event.source === proxy && event.origin === 'null' && Number.isSafeInteger(id)) {
      event.stopImmediatePropagation();
      channel.port1.postMessage(id);
    }
  }, true);
  proxy.postMessage({agenaMcpAppPort: token}, '*', [channel.port2]);
}`

function scriptLiteral(value: string) {
  return JSON.stringify(value).replace(/</g, '\\u003c')
}

/** Blob documents inherit the proxy's CSP. Both documents allow approved
 * embeds, while the private port binds the bridge to the original App document. */
export function mcpAppProxyDocument(hostOrigin: string, csp?: McpAppCsp, permissions?: readonly McpAppPermission[]) {
  const policy = mcpAppBrowserPolicy({ csp }, hostOrigin).csp
  const allowList = [...new Set((permissions ?? []).filter((permission) => VALID_PERMISSIONS.has(permission)))].join('; ')
  const frameOrigins = policy.frameDomains.join(' ')
  const proxyPrefix = documentPrefix(policy, ['blob:', ...policy.frameDomains].join(' '))
  const guestPrefix = documentPrefix(policy, frameOrigins || "'none'")
  return `${proxyPrefix}
<style>html,body{width:100%;height:100%;margin:0}iframe{display:block;width:100%;height:100%;border:0}</style>
<body><script>
(() => {
  const origin = ${scriptLiteral(hostOrigin)};
  const guestPrefix = ${scriptLiteral(guestPrefix)};
  const guestAllow = ${scriptLiteral(allowList)};
  const bootstrap = ${scriptLiteral(guestBootstrap)};
  const send = (message) => parent.postMessage(message, origin);
  const pending = new Map();
  let guest, token, port, loads = 0, sequence = 0;
  addEventListener('message', (event) => {
    if (event.source === parent && event.origin === origin) {
      if (!guest && event.data?.method === 'ui/notifications/sandbox-resource-ready') {
        const html = event.data.params?.html;
        if (typeof html !== 'string') return;
        token = crypto.getRandomValues(new Uint32Array(4)).join('-');
        const script = '<script>(' + bootstrap + ')(' + JSON.stringify(token) + ');<\\/script>';
        const url = URL.createObjectURL(new Blob([guestPrefix, script, html], {type:'text/html;charset=utf-8'}));
        guest = document.createElement('iframe');
        guest.title = 'MCP App';
        guest.setAttribute('sandbox', 'allow-scripts');
        if (guestAllow) guest.setAttribute('allow', guestAllow);
        guest.referrerPolicy = 'no-referrer';
        guest.addEventListener('load', () => {
          URL.revokeObjectURL(url);
          if (++loads > 1) {
            port?.close();
            pending.clear();
            guest.remove();
            send({jsonrpc:'2.0', method:'agena/guest-replaced'});
          }
        });
        guest.src = url;
        document.body.appendChild(guest);
      } else if (port && guest?.isConnected && !sandboxMessage(event.data)) {
        port.postMessage(event.data);
      }
    } else if (guest?.isConnected && event.source === guest.contentWindow && event.origin === 'null') {
      if (!port && event.data?.agenaMcpAppPort === token && event.ports.length === 1) {
        token = undefined;
        port = event.ports[0];
        port.onmessage = ({data: id}) => {
          if (pending.has(id)) {
            send(pending.get(id));
            pending.delete(id);
          }
        };
      } else if (port && pending.size < 64 && event.data?.jsonrpc === '2.0' && !sandboxMessage(event.data)) {
        // A navigation keeps the same WindowProxy. Only the original document
        // can answer a new challenge through its private port.
        const id = ++sequence;
        pending.set(id, event.data);
        guest.contentWindow.postMessage({agenaMcpAppChallenge: id}, '*');
      }
    }
  });
  function sandboxMessage(message) {
    return typeof message?.method === 'string' &&
      (message.method.startsWith('ui/notifications/sandbox-') || message.method === 'agena/guest-replaced');
  }
  send({jsonrpc:'2.0', method:'ui/notifications/sandbox-proxy-ready'});
})();
</script></body>`
}

/** A data document has an independent opaque origin, including when its
 * trusted proxy frame has the sandbox's allow-same-origin flag. */
export function mcpAppProxyUrl(hostOrigin: string, csp?: McpAppCsp, permissions?: readonly McpAppPermission[]) {
  return `data:text/html;charset=utf-8,${encodeURIComponent(mcpAppProxyDocument(hostOrigin, csp, permissions))}`
}
