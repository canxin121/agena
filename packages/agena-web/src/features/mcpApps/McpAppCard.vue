<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiAppsLine } from '@remixicon/vue'
import Button from '@/components/ui/Button.vue'
import { ApiError } from '@/lib/api'
import { useChatStore } from '@/stores/chat'
import { sessionRoutingScope } from '@/stores/chat/sessionMutationSync'
import McpAppDownloadPrompt from './McpAppDownloadPrompt.vue'
import McpAppMediaPreview from './McpAppMediaPreview.vue'
import McpAppUserPrompt from './McpAppUserPrompt.vue'
import {
  callMcpApp, readMcpApp, watchMcpAppCatalog,
  type McpAppCatalog, type McpAppDocument, type McpAppViewPrompt,
} from './api'
import { mcpAppContainerDimensions, mcpAppStyleVariables } from './hostContext'
import { resolveMcpAppMessageContent } from './media'
import { record } from './resources'
import { exposeMcpAppViewTools } from './viewTools'
import {
  MCP_APP_MODES,
  MCP_APP_PROTOCOL_VERSION,
  MCP_APP_TEARDOWN_ID,
  MCP_TASKS_EXTENSION,
  mcpAppAction,
  mcpAppBrowserPolicy,
  mcpAppGuestMessage,
  mcpAppGuestResponse,
  mcpAppHeight,
  mcpAppLog,
  mcpAppModes,
  mcpAppProgressToken,
  mcpAppProxyUrl,
  mcpAppRequest,
  mcpAppSandboxPermissions,
  mcpAppServerRequest,
  mcpAppTaskExtension,
  mcpAppTeardownResponse,
  type McpAppAction,
  type McpAppCsp,
  type McpAppMode,
  type McpAppPermission,
  type RpcId,
} from './bridge'

const props = defineProps<{ sessionId: string; partId: string }>()
const { locale, t } = useI18n()
const chat = useChatStore()
const card = ref<HTMLElement>()
const frame = ref<HTMLIFrameElement>()
const app = shallowRef<McpAppDocument>()
const phase = ref<'closed' | 'loading' | 'permissions' | 'proxy' | 'initialize' | 'initialized' | 'ready' | 'closing'>(
  'closed',
)
const failure = ref('')
const height = ref(300)
const mode = ref<McpAppMode>('inline')
const action = shallowRef<{ id: RpcId; request: McpAppAction }>()
const applyingAction = ref(false)
const prompts = shallowRef<(McpAppViewPrompt & { answer: (value: string | null) => void })[]>([])
const contextAttached = computed(() => chat.hasMcpAppContext(props.sessionId, props.partId))
const hostOrigin = window.location.origin
const policy = computed(() => mcpAppBrowserPolicy(app.value?.ui, hostOrigin))
const requestedOrigins = computed(() => [...new Set(Object.values(policy.value.csp).flat())])
const requestedPermissions = computed(() => policy.value.permissions)
const approvedCsp = shallowRef<McpAppCsp>()
const approvedPermissions = shallowRef<McpAppPermission[]>()
const proxy = computed(() => mcpAppProxyUrl(hostOrigin, approvedCsp.value, approvedPermissions.value))
const frameAllow = computed(() => approvedPermissions.value?.join('; ') || undefined)
let reading: AbortController | undefined
const catalogWatches = new Map<McpAppCatalog, () => void>()
let openedScope: number | undefined
let actionController: AbortController | undefined
let viewTools: ReturnType<typeof exposeMcpAppViewTools> | undefined
let guestTools: { listChanged: boolean } | undefined
let guestTaskExtension = false
const requests = new Map<string | number, AbortController>()
let timer: ReturnType<typeof setTimeout> | undefined
let themeObserver: MutationObserver | undefined
let sizeObserver: ResizeObserver | undefined
let lastHostContext = ''
let logWindow = 0
let logCount = 0
const hostInfo = { name: 'Agena', version: '0.1.0' }
const contentModalities = { text: {}, image: {}, audio: {}, resource: {}, resourceLink: {} }
const hover = window.matchMedia('(any-hover: hover)')
const timeZone = Intl.DateTimeFormat().resolvedOptions().timeZone
const hostModes: McpAppMode[] = typeof HTMLElement.prototype.showPopover === 'function' ? MCP_APP_MODES : ['inline']
let guestModes: McpAppMode[] = ['inline']
const overlayOpen = computed(() => mode.value !== 'inline' && (phase.value === 'ready' || phase.value === 'closing'))

// Keep the iframe in place while lifting the card above the transcript's paint containment.
watch(
  overlayOpen,
  (open) => {
    const element = card.value
    if (!element?.isConnected) return
    if (open) {
      element.popover = 'manual'
      element.showPopover()
    } else if (element.popover === 'manual') {
      element.hidePopover()
      element.removeAttribute('popover')
    }
  },
  { flush: 'post' },
)

function send(message: Record<string, unknown>) {
  frame.value?.contentWindow?.postMessage({ jsonrpc: '2.0', ...message }, '*')
}

function notify(method: string, params: Record<string, unknown>) {
  send({ method, params })
}

function hostContext() {
  const root = document.documentElement
  const style = getComputedStyle(root)
  const width = frame.value?.clientWidth || (mode.value === 'inline' ? card.value?.clientWidth : 0) || 0
  return {
    locale: locale.value,
    theme: root.classList.contains('dark') ? 'dark' : 'light',
    styles: { variables: mcpAppStyleVariables(style) },
    timeZone,
    userAgent: `${hostInfo.name}/${hostInfo.version}`,
    platform: 'web',
    displayMode: mode.value,
    availableDisplayModes: hostModes,
    containerDimensions: mcpAppContainerDimensions(mode.value, width, frame.value?.clientHeight ?? 0),
    deviceCapabilities: { hover: hover.matches, touch: navigator.maxTouchPoints > 0 },
  }
}

function syncHostContext() {
  if (phase.value !== 'ready') return
  const context = hostContext()
  const fingerprint = JSON.stringify(context)
  if (lastHostContext === fingerprint) return
  lastHostContext = fingerprint
  notify('ui/notifications/host-context-changed', context)
}
watch([locale, mode], syncHostContext, { flush: 'post' })
watch(
  frame,
  (current, previous) => {
    if (previous) sizeObserver?.unobserve(previous)
    if (current) sizeObserver?.observe(current)
  },
  { flush: 'post' },
)

function abortRequests() {
  stopViewTools()
  for (const stop of catalogWatches.values()) stop()
  catalogWatches.clear()
  for (const request of requests.values()) request.abort()
  requests.clear()
}

function stopViewTools() {
  viewTools?.close()
  viewTools = undefined
}

function askUser(request: McpAppViewPrompt, signal: AbortSignal) {
  return new Promise<string | null>((resolve) => {
    if (signal.aborted) {
      resolve(null)
      return
    }
    const answer = (value: string | null) => {
      signal.removeEventListener('abort', abort)
      prompts.value = prompts.value.filter((prompt) => prompt.id !== request.id)
      resolve(value)
    }
    const abort = () => answer(null)
    signal.addEventListener('abort', abort, { once: true })
    prompts.value = [...prompts.value, { ...request, answer }]
  })
}

function answerPrompt(id: string, answer: string | null) {
  prompts.value.find((prompt) => prompt.id === id)?.answer(answer)
}

function pageHidden(event: PageTransitionEvent) {
  if (!event.persisted) stopViewTools()
}

function watchCatalog(snapshot: McpAppDocument, catalog: McpAppCatalog, after: string, delay = 0) {
  const controller = new AbortController()
  const current = () =>
    catalogWatches.get(catalog) === stop &&
    app.value === snapshot &&
    phase.value === 'ready' &&
    openedScope === sessionRoutingScope()
  const timer = setTimeout(async () => {
    if (!current()) {
      if (catalogWatches.get(catalog) === stop) catalogWatches.delete(catalog)
      return
    }
    const deadline = setTimeout(() => controller.abort(), 45_000)
    try {
      const result = await watchMcpAppCatalog(
        props.sessionId, props.partId, snapshot.revision, catalog, after, controller.signal,
      )
      if (!current()) return
      if (result.revision !== after) notify(`notifications/${catalog}/list_changed`, {})
      watchCatalog(snapshot, catalog, result.revision, result.retry_after_ms)
    } catch (error) {
      if (current() && (!(error instanceof ApiError) || error.status === 429 || error.status >= 500)) {
        watchCatalog(snapshot, catalog, after, 4_000)
      }
    } finally {
      clearTimeout(deadline)
      if (catalogWatches.get(catalog) === stop) catalogWatches.delete(catalog)
    }
  }, delay)
  const stop = () => {
    clearTimeout(timer)
    controller.abort()
  }
  catalogWatches.set(catalog, stop)
}

function clearAction() {
  actionController?.abort()
  actionController = undefined
  applyingAction.value = false
  action.value = undefined
}

function reset() {
  clearTimeout(timer)
  reading?.abort()
  abortRequests()
  reading = undefined
  openedScope = undefined
  app.value = undefined
  phase.value = 'closed'
  height.value = 300
  mode.value = 'inline'
  guestModes = ['inline']
  guestTools = undefined
  guestTaskExtension = false
  lastHostContext = ''
  logWindow = 0
  logCount = 0
  clearAction()
  approvedCsp.value = undefined
  approvedPermissions.value = undefined
}
watch(
  () => [props.sessionId, props.partId],
  () => {
    reset()
    failure.value = ''
  },
)

function fail(message: string) {
  reset()
  failure.value = message
}

async function open() {
  reset()
  failure.value = ''
  phase.value = 'loading'
  const controller = new AbortController()
  const scope = sessionRoutingScope()
  reading = controller
  openedScope = scope
  timer = setTimeout(() => fail(t('chat.mcpApp.timeout')), 45_000)
  try {
    const value = await readMcpApp(props.sessionId, props.partId, controller.signal)
    if (reading !== controller) return
    if (sessionRoutingScope() !== scope) {
      fail(t('chat.mcpApp.unavailable'))
      return
    }
    clearTimeout(timer)
    app.value = value
    if (requestedOrigins.value.length || requestedPermissions.value.length) phase.value = 'permissions'
    else startProxy(false)
  } catch (error) {
    if (reading === controller) fail(error instanceof Error ? error.message : t('chat.mcpApp.unavailable'))
  } finally {
    if (reading === controller) reading = undefined
  }
}

function startProxy(allow: boolean) {
  if (openedScope !== sessionRoutingScope()) {
    fail(t('chat.mcpApp.unavailable'))
    return
  }
  approvedCsp.value = allow ? policy.value.csp : undefined
  approvedPermissions.value = allow && policy.value.permissions.length ? policy.value.permissions : undefined
  phase.value = 'proxy'
  timer = setTimeout(() => fail(t('chat.mcpApp.timeout')), 10_000)
}

function requestTeardown() {
  send({ method: 'ui/resource-teardown', id: MCP_APP_TEARDOWN_ID, params: {} })
}

function close() {
  failure.value = ''
  if (phase.value !== 'ready') {
    reset()
    return
  }
  abortRequests()
  clearAction()
  phase.value = 'closing'
  requestTeardown()
  timer = setTimeout(reset, 2_000)
}

function showInline(event: KeyboardEvent) {
  if (event.key !== 'Escape' || mode.value === 'inline' || phase.value !== 'ready') return
  dock()
}

function dock() {
  if (!guestModes.includes('inline')) {
    close()
    return
  }
  mode.value = 'inline'
}

function declineAction() {
  if (!action.value || applyingAction.value) return
  send({ id: action.value.id, error: { code: -32000, message: 'The user declined this request.' } })
  action.value = undefined
}

async function acceptAction() {
  const pending = action.value
  if (!pending || pending.request.kind === 'download' || applyingAction.value) return
  if (openedScope !== sessionRoutingScope()) {
    fail(t('chat.mcpApp.unavailable'))
    return
  }
  const controller = new AbortController()
  actionController = controller
  applyingAction.value = true
  const resourceLinks = pending.request.kind === 'link' ? undefined : pending.request.resourceLinks
  const deadline = resourceLinks ? setTimeout(() => controller.abort(), 45_000) : undefined
  try {
    if (pending.request.kind !== 'link' && resourceLinks) {
      const resolved = await resolveMcpAppMessageContent(resourceLinks, readAppResource, controller.signal)
      if (action.value !== pending || phase.value !== 'ready') return
      if (!resolved || (!resolved.text && !resolved.media?.length)) throw new Error(t('chat.mcpApp.unavailable'))
      action.value = { id: pending.id, request: { kind: pending.request.kind, ...resolved } }
      return
    }
    switch (pending.request.kind) {
      case 'message':
        await chat.sendMessage(props.sessionId, {
          text: pending.request.text,
          appMedia: pending.request.media,
          signal: controller.signal,
        })
        break
      case 'context':
        if (!chat.setMcpAppContext(props.sessionId, props.partId, pending.request.text, pending.request.media)) {
          throw new Error(t('chat.mcpApp.contextLimit'))
        }
        break
      case 'link':
        // The displayed anchor opens the approved address under the user gesture.
        break
    }
    if (action.value === pending && phase.value === 'ready') {
      send({ id: pending.id, result: {} })
    }
  } catch (error) {
    if (action.value === pending && phase.value === 'ready') {
      const message = error instanceof Error ? error.message : t('chat.mcpApp.unavailable')
      send({ id: pending.id, error: { code: -32000, message } })
      failure.value = message
    }
  } finally {
    clearTimeout(deadline)
    if (actionController === controller) {
      if (action.value === pending) clearAction()
      else {
        actionController = undefined
        applyingAction.value = false
      }
    }
  }
}

function readAppResource(uri: string, signal: AbortSignal) {
  if (!viewTools || openedScope !== sessionRoutingScope()) throw new Error(t('chat.mcpApp.unavailable'))
  return viewTools.requestServer({ method: 'resources/read', params: { uri } }, signal)
}

function finishDownload(error?: string) {
  if (action.value?.request.kind !== 'download') return
  const { id } = action.value
  action.value = undefined
  if (error) {
    failure.value = error
    send({ id, error: { code: -32000, message: error } })
  } else {
    send({ id, result: {} })
  }
}

async function receive(event: MessageEvent) {
  const guest = frame.value?.contentWindow
  if (phase.value === 'closing') {
    if (mcpAppTeardownResponse(event, guest)) reset()
    return
  }
  const message = mcpAppGuestMessage(event, guest)
  const snapshot = app.value
  if (!snapshot) return
  if (!message) {
    if (viewTools && phase.value === 'ready' && openedScope === sessionRoutingScope()) {
      const response = mcpAppGuestResponse(event, guest)
      if (response) viewTools.receive(response)
    }
    return
  }
  if (openedScope !== sessionRoutingScope()) {
    fail(t('chat.mcpApp.unavailable'))
    return
  }
  const { id, method, params } = message
  const reject = (code: number, message: string) => {
    if (id !== undefined) send({ id, error: { code, message } })
  }
  if (method === 'agena/guest-replaced') {
    fail(t('chat.mcpApp.unavailable'))
    return
  }
  if (phase.value === 'proxy') {
    if (method !== 'ui/notifications/sandbox-proxy-ready' || id !== undefined) return
    phase.value = 'initialize'
    notify('ui/notifications/sandbox-resource-ready', { html: snapshot.html })
    return
  }
  if (method === 'ui/initialize' && id !== undefined && phase.value === 'initialize') {
    if (typeof params.protocolVersion !== 'string') {
      reject(-32602, 'Invalid initialization request.')
      return
    }
    guestModes = mcpAppModes(params.appCapabilities).filter((mode) => hostModes.includes(mode))
    if (!guestModes.length) {
      reject(-32602, 'The MCP App has no supported display mode.')
      return
    }
    mode.value = guestModes[0]!
    const capabilities = params.appCapabilities
    const tools = record(capabilities) ? capabilities.tools : undefined
    guestTools = record(tools) ? { listChanged: tools.listChanged === true } : undefined
    guestTaskExtension = mcpAppTaskExtension(capabilities)
    phase.value = 'initialized'
    const context = hostContext()
    lastHostContext = JSON.stringify(context)
    send({
      id,
      result: {
        protocolVersion: MCP_APP_PROTOCOL_VERSION,
        hostInfo,
        hostCapabilities: {
          ...(guestTaskExtension && { experimental: { [MCP_TASKS_EXTENSION]: {} } }),
          openLinks: {},
          downloadFile: {},
          serverTools: snapshot.tool_catalog_revision ? { listChanged: true } : {},
          serverResources: snapshot.resource_catalog_revision ? { listChanged: true } : {},
          sampling: {},
          logging: {},
          message: contentModalities,
          updateModelContext: { ...contentModalities, structuredContent: {} },
          sandbox: {
            permissions: mcpAppSandboxPermissions(approvedPermissions.value),
            csp: approvedCsp.value ?? mcpAppBrowserPolicy(undefined, hostOrigin).csp,
          },
        },
        hostContext: context,
      },
    })
    return
  }
  if (method === 'ui/notifications/initialized' && id === undefined && phase.value === 'initialized') {
    clearTimeout(timer)
    phase.value = 'ready'
    await nextTick()
    if (app.value !== snapshot || phase.value !== 'ready') return
    syncHostContext()
    notify('ui/notifications/tool-input', { arguments: snapshot.arguments })
    notify('ui/notifications/tool-result', snapshot.tool_result)
    if (snapshot.tool_catalog_revision) watchCatalog(snapshot, 'tools', snapshot.tool_catalog_revision)
    if (snapshot.resource_catalog_revision) watchCatalog(snapshot, 'resources', snapshot.resource_catalog_revision)
    const scope = openedScope
    viewTools = exposeMcpAppViewTools(
      { sessionId: props.sessionId, partId: props.partId, revision: snapshot.revision },
      send,
      () => sessionRoutingScope() === scope,
      guestTools !== undefined,
      guestTaskExtension,
      askUser,
    )
    return
  }
  if (phase.value !== 'ready') {
    reject(-32000, 'The MCP App has not initialized.')
    return
  }
  if (method === 'notifications/tools/list_changed' && id === undefined) {
    if (guestTools?.listChanged) viewTools?.refresh()
    return
  }
  if (method === 'notifications/message' && id === undefined) {
    const now = performance.now()
    if (now - logWindow >= 10_000) {
      logWindow = now
      logCount = 0
    }
    if (logCount < 10) {
      const log = mcpAppLog(message)
      if (log) {
        logCount++
        console[log.level]('[MCP App]', snapshot.name, log.logger, log.data)
      }
    }
    return
  }
  if (method === 'ui/notifications/size-changed' && id === undefined) {
    height.value = mcpAppHeight(params.height) ?? height.value
    return
  }
  if (method === 'ui/notifications/request-teardown' && id === undefined) {
    close()
    return
  }
  if (method === 'notifications/cancelled' && id === undefined) {
    const requestId = params.requestId
    if (typeof requestId === 'string' || typeof requestId === 'number') requests.get(requestId)?.abort()
    if (action.value && action.value.id === requestId) clearAction()
    return
  }
  if (id === undefined) return
  if (method === 'ping') {
    send({ id, result: {} })
    return
  }
  if (method === 'ui/request-display-mode') {
    if (params.mode === 'inline' || params.mode === 'fullscreen' || params.mode === 'pip') {
      if (guestModes.includes(params.mode)) mode.value = params.mode
      send({ id, result: { mode: mode.value } })
    } else {
      reject(-32602, 'Invalid display mode.')
    }
    return
  }
  if (
    method === 'ui/open-link' ||
    method === 'ui/message' ||
    method === 'ui/update-model-context' ||
    method === 'ui/download-file'
  ) {
    if (requests.has(id) || action.value || applyingAction.value) {
      reject(-32000, 'The MCP App is already waiting for an interaction.')
      return
    }
    const request = mcpAppAction(message)
    if (!request) {
      reject(-32602, 'Invalid MCP App interaction.')
      return
    }
    failure.value = ''
    action.value = { id, request }
    return
  }
  if (
    method !== 'tools/list' &&
    method !== 'tools/call' &&
    method !== 'resources/read' &&
    method !== 'resources/list' &&
    method !== 'resources/templates/list' &&
    method !== 'sampling/createMessage' &&
    method !== 'tasks/get' &&
    method !== 'tasks/update' &&
    method !== 'tasks/result' &&
    method !== 'tasks/cancel'
  ) {
    reject(-32601, 'This MCP App method is unavailable.')
    return
  }
  const serverRequest = mcpAppServerRequest(message, guestTaskExtension)
  const request = serverRequest ? null : mcpAppRequest(message)
  if (!serverRequest && !request) {
    reject(-32602, 'Invalid MCP App request.')
    return
  }
  if (requests.has(id) || action.value?.id === id || requests.size >= 4) {
    reject(-32000, 'The MCP App has too many requests in progress.')
    return
  }
  const controller = new AbortController()
  requests.set(id, controller)
  const longRunning =
    method === 'tools/call' || method === 'tasks/result' ||
    method === 'resources/read' || method === 'sampling/createMessage'
  const deadline = setTimeout(() => controller.abort(), longRunning ? 300_000 : 45_000)
  const current = () =>
    requests.get(id) === controller &&
    phase.value === 'ready' &&
    openedScope === sessionRoutingScope()
  const progressToken = longRunning ? mcpAppProgressToken(params._meta) : undefined
  let progress = 0
  const heartbeat = progressToken === undefined ? undefined : setInterval(() => {
    if (current() && !controller.signal.aborted) {
      notify('notifications/progress', { progressToken, progress: ++progress })
    }
  }, 10_000)
  try {
    const work = serverRequest
      ? viewTools?.requestServer(serverRequest, controller.signal)
      : request && callMcpApp(props.sessionId, props.partId, snapshot.revision, request, controller.signal)
    if (!work) throw new Error(t('chat.mcpApp.unavailable'))
    const result = await work
    if (current()) send({ id, result })
  } catch (error) {
    if (current()) {
      const code = error instanceof ApiError && error.status === 400 ? -32602 : -32000
      reject(code, error instanceof Error ? error.message : 'MCP App request failed.')
    }
  } finally {
    clearInterval(heartbeat)
    clearTimeout(deadline)
    if (requests.get(id) === controller) requests.delete(id)
  }
}

onMounted(() => {
  window.addEventListener('message', receive)
  window.addEventListener('pagehide', pageHidden)
  window.addEventListener('keydown', showInline)
  window.addEventListener('resize', syncHostContext)
  hover.addEventListener?.('change', syncHostContext)
  if (typeof ResizeObserver === 'function') {
    sizeObserver = new ResizeObserver(syncHostContext)
    if (frame.value) sizeObserver.observe(frame.value)
  }
  themeObserver = new MutationObserver(syncHostContext)
  themeObserver.observe(document.documentElement, { attributes: true, attributeFilter: ['class', 'style'] })
})
onBeforeUnmount(() => {
  if (phase.value === 'ready') requestTeardown()
  window.removeEventListener('message', receive)
  window.removeEventListener('pagehide', pageHidden)
  window.removeEventListener('keydown', showInline)
  window.removeEventListener('resize', syncHostContext)
  hover.removeEventListener?.('change', syncHostContext)
  sizeObserver?.disconnect()
  themeObserver?.disconnect()
  reset()
})
</script>

<template>
  <section
    ref="card"
    class="mcp-app-card my-1 min-w-0 overflow-hidden rounded-md border bg-background"
    :class="phase === 'ready' || phase === 'closing' ? `mcp-app-${mode}` : undefined"
    data-mcp-app-card
  >
    <div class="flex shrink-0 items-center gap-2 px-2 py-1">
      <RiAppsLine class="h-4 w-4 shrink-0 text-muted-foreground" aria-hidden="true" />
      <span class="min-w-0 flex-1 truncate text-xs font-medium">{{ app?.name || 'MCP App' }}</span>
      <Button v-if="contextAttached" variant="ghost" size="sm" @click="chat.setMcpAppContext(sessionId, partId, '')">
        {{ t('chat.mcpApp.removeContext') }}
      </Button>
      <Button
        v-if="phase === 'ready' && mode !== 'inline' && guestModes.includes('inline')"
        variant="ghost"
        size="sm"
        @click="dock"
      >
        {{ t('chat.mcpApp.inline') }}
      </Button>
      <Button v-if="phase === 'closed'" variant="ghost" size="sm" @click="open">
        {{ failure ? t('common.retry') : t('chat.mcpApp.open') }}
      </Button>
      <Button v-else variant="ghost" size="sm" :disabled="phase === 'closing'" @click="close">
        {{ t('common.close') }}
      </Button>
    </div>
    <p v-if="failure" class="px-2 pb-2 text-xs text-destructive" role="alert">{{ failure }}</p>
    <p
      v-else-if="phase !== 'closed' && phase !== 'ready' && phase !== 'closing' && phase !== 'permissions'"
      class="px-2 pb-2 text-xs text-muted-foreground"
      role="status"
    >
      {{ t('chat.mcpApp.loading') }}
    </p>
    <div v-if="phase === 'permissions'" class="border-t px-3 py-2 text-xs">
      <p class="font-medium">{{ t('chat.mcpApp.capabilityPrompt') }}</p>
      <ul class="mt-1 max-h-36 list-disc space-y-1 overflow-auto pl-4">
        <li v-for="origin in requestedOrigins" :key="origin" class="break-all">{{ origin }}</li>
        <li v-for="permission in requestedPermissions" :key="`perm:${permission}`">{{ permission }}</li>
      </ul>
      <div class="mt-2 flex flex-wrap gap-2">
        <Button size="sm" @click="startProxy(true)">{{ t('chat.mcpApp.sandboxEnable') }}</Button>
        <Button size="sm" variant="ghost" @click="startProxy(false)">{{ t('chat.mcpApp.sandboxContinue') }}</Button>
      </div>
    </div>
    <McpAppDownloadPrompt
      v-if="action?.request.kind === 'download'"
      :files="action.request.files"
      :read="readAppResource"
      @cancel="declineAction"
      @complete="finishDownload()"
      @failure="finishDownload"
    />
    <div v-else-if="action" class="shrink-0 border-t px-3 py-2 text-xs" role="group">
      <p class="font-medium">{{ t(`chat.mcpApp.actions.${action.request.kind}`) }}</p>
      <p
        v-if="action.request.kind === 'link' || action.request.text"
        class="mt-1 max-h-36 overflow-auto whitespace-pre-wrap break-all"
      >
        {{ action.request.kind === 'link' ? action.request.uri : action.request.text }}
      </p>
      <McpAppMediaPreview
        v-if="action.request.kind !== 'link' && action.request.media?.length"
        :media="action.request.media"
      />
      <ul v-if="action.request.kind !== 'link' && action.request.resourceLinks" class="mt-1 max-h-36 overflow-auto">
        <li v-for="(uri, index) in action.request.resourceLinks.uris" :key="index" class="break-all">{{ uri }}</li>
      </ul>
      <div class="mt-2 flex flex-wrap items-center gap-2">
        <a
          v-if="action.request.kind === 'link'"
          :href="action.request.uri"
          target="_blank"
          rel="noopener noreferrer"
          class="rounded bg-primary px-2 py-1 text-primary-foreground focus-visible:ring"
          @click="acceptAction"
          @auxclick.middle="acceptAction"
          >{{ t('chat.mcpApp.openLink') }}</a
        >
        <Button v-else-if="action.request.resourceLinks" size="sm" :disabled="applyingAction" @click="acceptAction">
          {{ t(applyingAction ? 'common.loading' : 'nav.preview') }}
        </Button>
        <Button v-else size="sm" :disabled="applyingAction" @click="acceptAction">
          {{ t(action.request.kind === 'context' ? 'chat.mcpApp.useContext' : 'chat.mcpApp.sendMessage') }}
        </Button>
        <Button variant="ghost" size="sm" :disabled="applyingAction" @click="declineAction">
          {{ t('common.cancel') }}
        </Button>
      </div>
    </div>
    <McpAppUserPrompt
      v-if="prompts[0]"
      :key="prompts[0].id"
      :request-id="prompts[0].id"
      :prompt="prompts[0].prompt"
      @answer="answerPrompt"
    />
    <iframe
      v-if="app && phase !== 'permissions'"
      v-show="phase === 'ready' || phase === 'closing'"
      ref="frame"
      :src="proxy"
      :title="app.name"
      :allow="frameAllow"
      sandbox="allow-scripts allow-same-origin"
      referrerpolicy="no-referrer"
      class="box-content block w-full border-0 border-t"
      :class="{ 'border-t-0': !policy.prefersBorder }"
      :style="{ height: mode === 'inline' ? `${height}px` : undefined }"
    />
  </section>
</template>

<style scoped>
.mcp-app-fullscreen,
.mcp-app-pip {
  position: fixed;
  margin: 0;
  padding: 0;
  z-index: 60;
  display: flex;
  flex-direction: column;
  color: inherit;
  box-shadow: 0 16px 46px #0004;
}

.mcp-app-fullscreen {
  inset: 0.75rem;
  width: auto;
  height: auto;
}

.mcp-app-pip {
  inset-block: auto 1rem;
  inset-inline: auto 1rem;
  width: min(28rem, calc(100vw - 2rem));
  height: min(24rem, 50vh);
}

.mcp-app-fullscreen iframe,
.mcp-app-pip iframe {
  flex: 1;
  min-height: 0;
}
</style>
