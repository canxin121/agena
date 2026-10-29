<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import {
  RiPlayLine,
  RiStopLine,
  RiExternalLinkLine,
  RiCodeLine,
  RiRefreshLine,
  RiFullscreenLine,
  RiTerminalBoxLine,
} from '@remixicon/vue'
import { apiJson } from '@/lib/api'
import { useUiStore } from '@/stores/ui'
import { useWorkspacePreviewStore } from '@/stores/workspacePreview'
import CodeBlock from '@/components/ui/CodeBlock.vue'
import { APP_CHANNEL, appFrameDocument } from './frame'
import { appApiPath, bridgeFetch, saveApp, type AppSnapshot } from './api'
import type { AppManifest } from './model'
import { useAppLifecycle } from './useAppLifecycle'

const props = withDefaults(
  defineProps<{ manifest: AppManifest; owner: string; directory?: string; standalone?: boolean }>(),
  { directory: '', standalone: false },
)
const { t, locale } = useI18n()
const ui = useUiStore()
const preview = useWorkspacePreviewStore()
const root = ref<HTMLElement>()
const frame = ref<HTMLIFrameElement>()
const snapshot = shallowRef<AppSnapshot | null>(null)
const documentSource = ref('')
const token = ref('')
const lifecycle = useAppLifecycle()
const busy = computed(() => lifecycle.phase.value !== '')
const starting = computed(() => lifecycle.phase.value === 'run')
const error = ref('')
const sourceVisible = ref(false)
const logsVisible = ref(false)
const logs = ref<Record<string, string>>({})
const frameHeight = ref(props.manifest.height || 480)
const ready = ref(false)
const configurationSaved = ref(false)
const hasServices = computed(() => !!props.manifest.services?.length)
const running = computed(() =>
  Object.values(snapshot.value?.services || {}).some((s) => !!s.pid && ['running', 'ready'].includes(s.state)),
)
const apiPath = computed(() => appApiPath(props.owner, props.manifest.id))
const controllers = new Set<AbortController>()
const lifecycleReads = new Set<AbortController>()
let initialSave: AbortController | undefined
let generation = 0
let disposed = false
let mounted = false
let currentScope = ''
let releasePreview: (() => void) | undefined
const theme = () => (document.documentElement.classList.contains('dark') ? 'dark' : 'light')
let themeObserver: MutationObserver | undefined

function abortRequests() {
  for (const c of controllers) c.abort()
  controllers.clear()
}
function abortLifecycleRequests() {
  lifecycle.cancel()
  for (const controller of lifecycleReads) controller.abort()
  lifecycleReads.clear()
}
function renderFrame() {
  abortRequests()
  ready.value = false
  token.value = crypto.randomUUID()
  documentSource.value = appFrameDocument(
    props.manifest,
    token.value,
    snapshot.value?.widgetState ?? null,
    locale.value,
    theme(),
  )
}
async function initialize() {
  const version = ++generation
  abortRequests()
  abortLifecycleRequests()
  const controller = new AbortController()
  initialSave = controller
  lifecycleReads.add(controller)
  configurationSaved.value = false
  logs.value = {}
  const scope = `${props.owner}/${props.manifest.id}`
  if (scope !== currentScope) {
    snapshot.value = null
    currentScope = scope
  }
  documentSource.value = ''
  token.value = ''
  error.value = ''
  try {
    if (!props.owner) throw new Error(t('interactive.missingSession'))
    const result = await saveApp(props.owner, props.directory, props.manifest, controller.signal)
    if (disposed || version !== generation || controller.signal.aborted) return
    snapshot.value = result
    configurationSaved.value = true
  } catch (err) {
    if (disposed || version !== generation || controller.signal.aborted) return
    error.value = err instanceof Error ? err.message : String(err)
  } finally {
    lifecycleReads.delete(controller)
    if (initialSave === controller) initialSave = undefined
  }
  renderFrame()
}
watch(
  () => JSON.stringify([props.owner, props.directory, props.manifest]),
  () => {
    if (mounted) void initialize()
  },
)
watch(
  () => preview.sessions,
  (sessions) => {
    if (!snapshot.value) return
    const next = { ...snapshot.value.services }
    let changed = false
    for (const [name, service] of Object.entries(next)) {
      const live = sessions.find((s) => s.id === service.id)
      if (live && JSON.stringify(live) !== JSON.stringify(service)) {
        next[name] = live
        changed = true
      }
    }
    if (changed) snapshot.value = { ...snapshot.value, services: next }
  },
)

async function run() {
  const operation = lifecycle.begin('run')
  if (!operation) return
  initialSave?.abort()
  const version = generation
  const path = apiPath.value
  error.value = ''
  try {
    await saveApp(props.owner, props.directory, props.manifest, operation.signal)
    if (disposed || version !== generation || !operation.isCurrent()) return
    configurationSaved.value = true
    const result = await apiJson<AppSnapshot>(`${path}/run`, { method: 'POST', signal: operation.signal })
    if (disposed || version !== generation || !operation.isCurrent()) return
    snapshot.value = result
    // A fresh document boots against ready services and restored widget state.
    renderFrame()
  } catch (err) {
    if (!disposed && version === generation && operation.isCurrent()) {
      error.value = err instanceof Error ? err.message : String(err)
      await refreshStatus(operation.signal)
    }
  } finally {
    operation.finish()
  }
}
async function refreshStatus(signal?: AbortSignal) {
  const version = generation
  try {
    const result = await apiJson<AppSnapshot>(apiPath.value, { signal })
    if (!disposed && version === generation && !signal?.aborted) snapshot.value = result
  } catch {
    /* Keep the actionable lifecycle error. */
  }
}
async function stop() {
  const operation = lifecycle.begin('stop')
  if (!operation) return
  initialSave?.abort()
  const version = generation
  error.value = ''
  abortRequests()
  try {
    const result = await apiJson<AppSnapshot>(`${apiPath.value}/stop`, { method: 'POST', signal: operation.signal })
    if (!disposed && version === generation && operation.isCurrent()) snapshot.value = result
  } catch (err) {
    if (!disposed && version === generation && operation.isCurrent())
      error.value = err instanceof Error ? err.message : String(err)
  } finally {
    operation.finish()
  }
}
async function showLogs() {
  logsVisible.value = !logsVisible.value
  if (!logsVisible.value) return
  const version = generation
  const controller = new AbortController()
  lifecycleReads.add(controller)
  try {
    const result = await apiJson<Record<string, string>>(`${apiPath.value}/logs`, { signal: controller.signal })
    if (!disposed && version === generation && !controller.signal.aborted) logs.value = result
  } catch (err) {
    if (!disposed && version === generation && !controller.signal.aborted)
      error.value = err instanceof Error ? err.message : String(err)
  } finally {
    lifecycleReads.delete(controller)
  }
}
async function openPreview() {
  const version = generation
  const controller = new AbortController()
  lifecycleReads.add(controller)
  try {
    await saveApp(props.owner, props.directory, props.manifest, controller.signal)
    if (disposed || version !== generation || controller.signal.aborted) return
    ui.openWorkspaceWindow('preview', {
      title: props.manifest.title,
      query: { appOwner: props.owner, appId: props.manifest.id, directory: props.directory },
      matchKeys: ['appOwner', 'appId'],
    })
  } catch (err) {
    if (!disposed && version === generation && !controller.signal.aborted)
      error.value = err instanceof Error ? err.message : String(err)
  } finally {
    lifecycleReads.delete(controller)
  }
}
function fullscreen() {
  void root.value?.requestFullscreen().catch((err) => {
    error.value = String(err)
  })
}

async function message(event: MessageEvent) {
  const data = event.data
  if (
    event.source !== frame.value?.contentWindow ||
    !data ||
    data.channel !== APP_CHANNEL ||
    data.token !== token.value
  )
    return
  if (data.type === 'resize' && Number.isFinite(data.height)) {
    frameHeight.value = Math.max(props.manifest.height || 160, Math.min(1200, data.height))
    return
  }
  if (data.type === 'ready') {
    ready.value = true
    return
  }
  if (data.type === 'error') {
    error.value = String(data.message || '').slice(0, 1000)
    return
  }
  if (data.type !== 'request' || typeof data.id !== 'string' || data.id.length > 100) return
  const source = event.source as Window
  const currentToken = token.value
  const respond = (result?: unknown, failure?: string) => {
    if (!disposed && currentToken === token.value)
      source.postMessage(
        { channel: APP_CHANNEL, token: currentToken, type: 'response', id: data.id, result, error: failure },
        '*',
      )
  }
  if (controllers.size >= 16) {
    respond(undefined, 'Too many concurrent app requests')
    return
  }
  const controller = new AbortController()
  controllers.add(controller)
  const timeout = setTimeout(() => controller.abort(), 20000)
  try {
    if (data.method === 'fetch') {
      if (!configurationSaved.value) throw new Error('Save the app configuration before using its services')
      respond(await bridgeFetch(snapshot.value, props.manifest, data.payload, controller.signal))
    } else if (data.method === 'state') {
      const value = data.payload?.value ?? null
      if (new TextEncoder().encode(JSON.stringify(value)).byteLength > 64 * 1024)
        throw new Error('Widget state exceeds 64 KiB')
      const saved = await apiJson<unknown>(`${apiPath.value}/state`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(value),
        signal: controller.signal,
      })
      if (!disposed && currentToken === token.value && snapshot.value)
        snapshot.value = { ...snapshot.value, widgetState: saved }
      respond(saved)
    } else throw new Error('Unsupported application capability')
  } catch (err) {
    respond(undefined, err instanceof Error ? err.message : String(err))
  } finally {
    clearTimeout(timeout)
    controllers.delete(controller)
  }
}
function sendEnvironment() {
  frame.value?.contentWindow?.postMessage(
    { channel: APP_CHANNEL, token: token.value, type: 'environment', theme: theme(), locale: locale.value },
    '*',
  )
}
watch(locale, sendEnvironment)
watch(hasServices, (enabled) => {
  if (!mounted || disposed) return
  releasePreview?.()
  releasePreview = enabled ? preview.retainLiveSessions() : undefined
})
onMounted(() => {
  mounted = true
  window.addEventListener('message', message)
  themeObserver = new MutationObserver(sendEnvironment)
  themeObserver.observe(document.documentElement, { attributes: true, attributeFilter: ['class'] })
  if (hasServices.value) releasePreview = preview.retainLiveSessions()
  void initialize()
})
onBeforeUnmount(() => {
  disposed = true
  generation++
  abortRequests()
  abortLifecycleRequests()
  releasePreview?.()
  themeObserver?.disconnect()
  if (mounted) window.removeEventListener('message', message)
})
</script>

<template>
  <section
    ref="root"
    class="interactive-app my-4 overflow-hidden rounded-xl border border-border/70 bg-background shadow-sm"
    :class="standalone ? 'standalone' : ''"
    :aria-label="manifest.title"
    data-interactive-app
  >
    <header class="app-header flex min-h-12 items-center gap-2 border-b border-border/60 bg-muted/20 px-3 py-2">
      <div class="app-heading flex min-w-0 flex-1 items-center gap-2">
        <span
          class="h-2 w-2 shrink-0 rounded-full"
          :class="running || (!hasServices && ready) ? 'bg-emerald-500' : 'bg-muted-foreground/40'"
        />
        <div class="min-w-0 flex-1">
          <div class="truncate text-sm font-semibold">{{ manifest.title }}</div>
          <p v-if="manifest.description" class="truncate text-xs text-muted-foreground">{{ manifest.description }}</p>
        </div>
      </div>
      <div class="app-actions flex shrink-0 items-center gap-1" role="group" :aria-label="manifest.title">
        <span v-if="busy" class="text-xs text-muted-foreground" role="status">{{
          t(starting ? 'interactive.starting' : 'interactive.loading')
        }}</span>
        <button
          v-if="hasServices && !running && !starting"
          class="app-button primary"
          :disabled="busy || !owner || !directory"
          @click="run"
        >
          <RiPlayLine class="h-4 w-4" />{{ t('interactive.run') }}
        </button>
        <button
          v-if="hasServices && (running || starting)"
          class="app-button"
          :disabled="busy && !starting"
          @click="stop"
        >
          <RiStopLine class="h-4 w-4" />{{ t('interactive.stop') }}
        </button>
        <button
          class="app-icon"
          :title="t('interactive.reload')"
          :aria-label="t('interactive.reload')"
          @click="renderFrame"
        >
          <RiRefreshLine class="h-4 w-4" />
        </button>
        <button
          class="app-icon"
          :title="t('interactive.source')"
          :aria-label="t('interactive.source')"
          :aria-pressed="sourceVisible"
          @click="sourceVisible = !sourceVisible"
        >
          <RiCodeLine class="h-4 w-4" />
        </button>
        <button
          v-if="hasServices"
          class="app-icon"
          :title="t('interactive.logs')"
          :aria-label="t('interactive.logs')"
          :aria-pressed="logsVisible"
          @click="showLogs"
        >
          <RiTerminalBoxLine class="h-4 w-4" />
        </button>
        <button
          v-if="!standalone"
          class="app-icon"
          :title="t('interactive.openPreview')"
          :aria-label="t('interactive.openPreview')"
          @click="openPreview"
        >
          <RiExternalLinkLine class="h-4 w-4" />
        </button>
        <button
          class="app-icon"
          :title="t('interactive.fullscreen')"
          :aria-label="t('interactive.fullscreen')"
          @click="fullscreen"
        >
          <RiFullscreenLine class="h-4 w-4" />
        </button>
      </div>
    </header>
    <div
      v-if="error"
      class="border-b border-border/60 bg-destructive/5 px-3 py-2 text-xs text-destructive"
      role="alert"
    >
      {{ error }}
    </div>
    <details v-if="hasServices" class="border-b border-border/60 px-3 py-1.5 text-xs text-muted-foreground">
      <summary class="cursor-pointer">
        {{ t('interactive.services', { count: manifest.services?.length }) }} ·
        {{ running ? t('interactive.running') : t('interactive.stopped') }}
      </summary>
      <div v-for="service in manifest.services" :key="service.name" class="my-2 flex flex-wrap items-baseline gap-x-2">
        <strong class="text-foreground">{{ service.name }}</strong
        ><code class="break-all">{{ [service.command, ...(service.args || [])].join(' ') }}</code
        ><span>{{ service.cwd || '.' }}</span>
      </div>
    </details>
    <CodeBlock v-if="sourceVisible" :code="JSON.stringify(manifest, null, 2)" lang="json" />
    <div v-if="logsVisible" class="max-h-64 overflow-auto border-b border-border bg-muted/20 p-3">
      <div v-for="(text, name) in logs" :key="name">
        <strong class="text-xs">{{ name }}</strong>
        <pre class="whitespace-pre-wrap break-words font-mono text-xs">{{ text || t('interactive.noLogs') }}</pre>
      </div>
      <p v-if="!Object.keys(logs).length" class="text-xs text-muted-foreground">{{ t('interactive.noLogs') }}</p>
    </div>
    <iframe
      v-if="documentSource"
      ref="frame"
      class="block w-full border-0"
      sandbox="allow-scripts allow-forms"
      referrerpolicy="no-referrer"
      :title="manifest.title"
      :srcdoc="documentSource"
      :style="{ height: `${frameHeight}px` }"
    />
  </section>
</template>

<style scoped>
.interactive-app {
  container-type: inline-size;
}
@container (max-width: 480px) {
  .app-header {
    flex-direction: column;
    align-items: stretch;
  }
  .app-actions {
    justify-content: flex-end;
    flex-wrap: wrap;
  }
}
.app-button,
.app-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 0.3rem;
  min-height: 30px;
  border-radius: 0.4rem;
  font-size: 12px;
  color: var(--color-muted-foreground);
}
.app-button {
  padding: 0.25rem 0.6rem;
  border: 1px solid var(--color-border);
}
.app-icon {
  width: 30px;
}
.app-button:hover,
.app-icon:hover {
  background: var(--color-muted);
  color: var(--color-foreground);
}
.app-button:focus-visible,
.app-icon:focus-visible {
  outline: 2px solid var(--color-ring);
  outline-offset: 2px;
}
.app-button:disabled {
  opacity: 0.45;
  cursor: default;
}
.app-button.primary {
  background: var(--color-primary);
  color: var(--color-primary-foreground);
  border-color: transparent;
}
.interactive-app:fullscreen {
  overflow: auto;
  margin: 0;
  border: 0;
  border-radius: 0;
  background: var(--color-background);
}
.interactive-app:fullscreen iframe {
  min-height: calc(100vh - 100px);
}
.standalone {
  margin: 0;
  min-height: 100%;
  border: 0;
  border-radius: 0;
}
</style>
