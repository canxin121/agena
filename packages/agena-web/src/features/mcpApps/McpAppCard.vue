<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiAppsLine } from '@remixicon/vue'
import Button from '@/components/ui/Button.vue'
import { callMcpApp, readMcpApp, type McpAppDocument } from './api'
import {
  MCP_APP_MAX_HEIGHT,
  MCP_APP_PROTOCOL_VERSION,
  mcpAppGuestDocument,
  mcpAppGuestMessage,
  mcpAppHeight,
  mcpAppProxyDocument,
  mcpAppRequest,
} from './bridge'

const props = defineProps<{ sessionId: string; partId: string }>()
const { locale, t } = useI18n()
const frame = ref<HTMLIFrameElement>()
const app = shallowRef<McpAppDocument>()
const phase = ref<'closed' | 'loading' | 'proxy' | 'initialize' | 'initialized' | 'ready' | 'closing'>('closed')
const failure = ref('')
const height = ref(300)
const proxy = mcpAppProxyDocument(window.location.origin)
let reading: AbortController | undefined
const requests = new Map<string | number, AbortController>()
let timer: ReturnType<typeof setTimeout> | undefined
let themeObserver: MutationObserver | undefined

function send(message: Record<string, unknown>) {
  frame.value?.contentWindow?.postMessage({ jsonrpc: '2.0', ...message }, '*')
}

function notify(method: string, params: Record<string, unknown>) {
  send({ method, params })
}

function hostContext() {
  return {
    locale: locale.value,
    theme: document.documentElement.classList.contains('dark') ? 'dark' : 'light',
    platform: 'web',
    displayMode: 'inline',
    availableDisplayModes: ['inline'],
    containerDimensions: { maxHeight: MCP_APP_MAX_HEIGHT },
  }
}

function syncHostContext() {
  if (phase.value === 'ready') notify('ui/notifications/host-context-changed', hostContext())
}
watch(locale, syncHostContext)

function abortRequests() {
  for (const request of requests.values()) request.abort()
  requests.clear()
}

function reset() {
  clearTimeout(timer)
  reading?.abort()
  abortRequests()
  reading = undefined
  app.value = undefined
  phase.value = 'closed'
  height.value = 300
}

function fail(message: string) {
  reset()
  failure.value = message
}

async function open() {
  reset()
  failure.value = ''
  phase.value = 'loading'
  const controller = new AbortController()
  reading = controller
  timer = setTimeout(() => fail(t('chat.mcpApp.timeout')), 45_000)
  try {
    const value = await readMcpApp(props.sessionId, props.partId, controller.signal)
    if (reading !== controller) return
    clearTimeout(timer)
    app.value = value
    phase.value = 'proxy'
    timer = setTimeout(() => fail(t('chat.mcpApp.timeout')), 10_000)
  } catch (error) {
    if (reading === controller) fail(error instanceof Error ? error.message : t('chat.mcpApp.unavailable'))
  } finally {
    if (reading === controller) reading = undefined
  }
}

function close() {
  failure.value = ''
  if (phase.value !== 'ready') {
    reset()
    return
  }
  abortRequests()
  phase.value = 'closing'
  send({ method: 'ui/resource-teardown', id: 'agena-teardown', params: {} })
  timer = setTimeout(reset, 100)
}

async function receive(event: MessageEvent) {
  const message = mcpAppGuestMessage(event, frame.value?.contentWindow)
  const snapshot = app.value
  if (!message || !snapshot || phase.value === 'closing') return
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
    notify('ui/notifications/sandbox-resource-ready', { html: mcpAppGuestDocument(snapshot.html) })
    return
  }
  if (method === 'ui/initialize' && id !== undefined && phase.value === 'initialize') {
    if (typeof params.protocolVersion !== 'string') {
      reject(-32602, 'Invalid initialization request.')
      return
    }
    phase.value = 'initialized'
    send({
      id,
      result: {
        protocolVersion: MCP_APP_PROTOCOL_VERSION,
        hostInfo: { name: 'Agena', version: '0.1.0' },
        hostCapabilities: { serverTools: {}, serverResources: {} },
        hostContext: hostContext(),
      },
    })
    return
  }
  if (method === 'ui/notifications/initialized' && id === undefined && phase.value === 'initialized') {
    clearTimeout(timer)
    phase.value = 'ready'
    notify('ui/notifications/tool-input', { arguments: snapshot.arguments })
    notify('ui/notifications/tool-result', snapshot.tool_result)
    return
  }
  if (phase.value !== 'ready') {
    reject(-32000, 'The MCP App has not initialized.')
    return
  }
  if (method === 'ui/notifications/size-changed' && id === undefined) {
    height.value = mcpAppHeight(params.height) ?? height.value
    return
  }
  if (id === undefined) return
  if (method === 'ping') {
    send({ id, result: {} })
    return
  }
  if (method !== 'tools/call' && method !== 'resources/read') {
    reject(-32601, 'This MCP App method is unavailable.')
    return
  }
  const request = mcpAppRequest(message)
  if (!request) {
    reject(-32602, 'Invalid MCP App request.')
    return
  }
  if (requests.has(id) || requests.size >= 4) {
    reject(-32000, 'The MCP App has too many requests in progress.')
    return
  }
  const controller = new AbortController()
  requests.set(id, controller)
  const deadline = setTimeout(() => controller.abort(), 45_000)
  try {
    const result = await callMcpApp(props.sessionId, props.partId, snapshot.revision, request, controller.signal)
    if (requests.get(id) === controller && phase.value === 'ready') send({ id, result })
  } catch (error) {
    if (requests.get(id) === controller && phase.value === 'ready') {
      reject(-32000, error instanceof Error ? error.message : 'MCP App request failed.')
    }
  } finally {
    clearTimeout(deadline)
    if (requests.get(id) === controller) requests.delete(id)
  }
}

function onHidden() {
  if (document.hidden) reset()
}

onMounted(() => {
  window.addEventListener('message', receive)
  document.addEventListener('visibilitychange', onHidden)
  themeObserver = new MutationObserver(syncHostContext)
  themeObserver.observe(document.documentElement, { attributes: true, attributeFilter: ['class'] })
})
onBeforeUnmount(() => {
  window.removeEventListener('message', receive)
  document.removeEventListener('visibilitychange', onHidden)
  themeObserver?.disconnect()
  reset()
})
</script>

<template>
  <section class="my-1 min-w-0 overflow-hidden rounded-md border bg-background" data-mcp-app-card>
    <div class="flex items-center gap-2 px-2 py-1">
      <RiAppsLine class="h-4 w-4 shrink-0 text-muted-foreground" aria-hidden="true" />
      <span class="min-w-0 flex-1 truncate text-xs font-medium">{{ app?.name || 'MCP App' }}</span>
      <Button v-if="phase === 'closed'" variant="ghost" size="sm" @click="open">
        {{ failure ? t('common.retry') : t('chat.mcpApp.open') }}
      </Button>
      <Button v-else variant="ghost" size="sm" :disabled="phase === 'closing'" @click="close">
        {{ t('common.close') }}
      </Button>
    </div>
    <p v-if="failure" class="px-2 pb-2 text-xs text-destructive" role="alert">{{ failure }}</p>
    <p
      v-else-if="phase !== 'closed' && phase !== 'ready' && phase !== 'closing'"
      class="px-2 pb-2 text-xs text-muted-foreground"
      role="status"
    >
      {{ t('chat.mcpApp.loading') }}
    </p>
    <iframe
      v-if="app"
      v-show="phase === 'ready' || phase === 'closing'"
      ref="frame"
      :srcdoc="proxy"
      :title="app.name"
      sandbox="allow-scripts"
      referrerpolicy="no-referrer"
      class="block w-full border-0 border-t"
      :style="{ height: `${height}px` }"
    />
  </section>
</template>
