<script setup lang="ts">
import { computed, defineAsyncComponent, nextTick, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import {
  RiArrowLeftLine,
  RiArrowRightUpLine,
  RiApps2Line,
  RiErrorWarningLine,
  RiLoader4Line,
  RiRefreshLine,
  RiSearchLine,
  RiSettings3Line,
  RiSendPlane2Line,
} from '@remixicon/vue'
import Button from '@/components/ui/Button.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import { useWorkspacePaneContext } from '@/app/workspace/workspacePaneContext'
import { isEmbeddedWorkspacePaneContext } from '@/app/windowScope'
import { useWorkspaceNavigation } from '@/app/navigation/useWorkspaceNavigation'
import { WORKSPACE_SIDEBAR_PANEL_HOST_SELECTOR } from '@/layout/workspaceSidebarHost'
import { useUiStore } from '@/stores/ui'
import { useChatStore } from '@/stores/chat'
import { useToastsStore } from '@/stores/toasts'
import { apiJson, apiUrl } from '@/lib/api'
import { usePluginWebCatalog } from '@/features/pluginWeb/catalog'
import { pluginHubEntries, TELEGRAM_PAGE_ID, TELEGRAM_PLUGIN_ID, type PluginHubEntry } from '@/features/pluginWeb/hub'
import { createPluginUI } from '@/features/pluginWeb/createSdk'
import { PluginPageMount } from '@/features/pluginWeb/mount'
import { usePluginSessionRefresh } from '@/features/pluginWeb/usePluginSessionRefresh'
import { pluginAssetUrl, pluginPageKey, pluginPagePath, pluginPageSession } from '@/features/pluginWeb/model'
import type { PluginPageCatalogItem, PluginUIEnvironment } from '@/features/pluginWeb/sdk'

const TelegramManager = defineAsyncComponent(() => import('@/features/telegram/TelegramManager.vue'))

const route = useRoute()
const router = useRouter()
const pane = useWorkspacePaneContext()
const location = computed(() => pane?.route.value || route)
const navigation = useWorkspaceNavigation()
const ui = useUiStore()
const chat = useChatStore()
const toasts = useToastsStore()
const catalog = usePluginWebCatalog()
const { t, locale } = useI18n()
const pluginId = computed(() => String(location.value.params.pluginId || ''))
const pageId = computed(() => String(location.value.params.pageId || ''))
const selected = computed(() =>
  catalog.pages.find((page) => page.plugin_id === pluginId.value && page.page_id === pageId.value),
)
const sessionId = computed(() =>
  selected.value?.scope === 'session' ? pluginPageSession(location.value.query.sessionId) : null,
)
const search = ref('')
const pages = computed(() => {
  const query = search.value.trim().toLocaleLowerCase()
  return [...catalog.pages]
    .filter((page) => !query || `${page.title} ${page.plugin_id}`.toLocaleLowerCase().includes(query))
    .sort((a, b) => a.title.localeCompare(b.title, String(locale.value)))
})
const allPlugins = computed(() => pluginHubEntries(catalog.plugins, catalog.pages, String(locale.value)))
const entries = computed(() => {
  const query = search.value.trim().toLocaleLowerCase()
  return allPlugins.value.filter(
    (entry) =>
      !query ||
      `${entry.plugin_id} ${entry.title} ${entry.summary} ${entry.state} ${entry.pages.map((page) => page.title).join(' ')}`
        .toLocaleLowerCase()
        .includes(query),
  )
})
const isTelegram = computed(
  () =>
    pluginId.value === TELEGRAM_PLUGIN_ID &&
    pageId.value === TELEGRAM_PAGE_ID &&
    catalog.plugins.some((plugin) => plugin.plugin_id === TELEGRAM_PLUGIN_ID),
)
const telegramStatus = computed(() => catalog.plugins.find((plugin) => plugin.plugin_id === TELEGRAM_PLUGIN_ID))
const telegramSource = computed(() => pluginPageSession(location.value.query.source))
const showSidebar = computed(
  () => !ui.isCompactLayout && !isEmbeddedWorkspacePaneContext(location.value.query) && (!pane || pane.isFocused.value),
)
const container = ref<HTMLElement>()
const mountError = ref('')
const mounting = ref(false)
const ready = ref(false)
const retryCount = ref(0)
const title = ref('')
const sessionError = usePluginSessionRefresh(
  () => selected.value?.scope,
  () => chat.refreshSessions(),
)
const environment = shallowRef<PluginUIEnvironment>(
  Object.freeze({
    locale: String(locale.value),
    theme: 'light',
  }),
)
const environmentListeners = new Set<(environment: PluginUIEnvironment) => void>()
let releaseCatalog: (() => void) | undefined
let themeObserver: MutationObserver | undefined
let version = 0
let disposed = false
const mounts = new PluginPageMount(() => {
  toasts.push('error', String(t('pluginWeb.cleanupFailed')))
})

const state = computed(() => {
  if (!pluginId.value) return 'library'
  if (!catalog.loaded) return catalog.error ? 'error' : 'loading'
  if (isTelegram.value) return 'telegram'
  if (!selected.value) return 'unavailable'
  if (selected.value.scope === 'session' && sessionId.value === null) return 'no-session'
  if (mountError.value) return 'error'
  return ready.value && !mounting.value ? 'ready' : 'loading'
})
const sessionOptions = computed(() => {
  const options = chat.sessions
    .filter((session) => Number(session.workspace_id) > 0)
    .map((session) => ({
      value: session.id,
      label: session.title || String(t('pluginWeb.sessionLabel', { id: session.id })),
    }))
  if (sessionId.value !== null && !options.some((option) => option.value === String(sessionId.value)))
    options.unshift({
      value: String(sessionId.value),
      label: String(t('pluginWeb.sessionLabel', { id: sessionId.value })),
    })
  return options
})

function setTitle(value: string) {
  title.value = value || selected.value?.title || String(t('pluginWeb.title'))
  if (pane) ui.setWorkspaceWindowTitle(pane.windowId.value, title.value)
  else ui.setWorkspaceWindowTitleFromRoute(location.value.query, title.value)
}

async function navigate(path: string, query: Record<string, string> = {}) {
  if (pane) await pane.navigate({ path, query })
  else await router.push({ path, query })
}

async function openPage(page: PluginPageCatalogItem) {
  const query = page.scope === 'session' && sessionId.value !== null ? { sessionId: String(sessionId.value) } : {}
  await navigation.openWorkspaceLocation('plugins', {
    path: pluginPagePath(page.plugin_id, page.page_id),
    query,
    title: page.title,
    matchKeys: page.scope === 'session' ? ['sessionId'] : [],
  })
}

async function managePlugin(id: string) {
  await navigation.openMainTab('settings', {
    path: '/settings/plugins-tools',
    query: { view: 'plugin-workbench', ...(id ? { plugin: id } : {}) },
  })
}
async function managePlugins() {
  await managePlugin(pluginId.value)
}
async function openPlugin(entry: PluginHubEntry) {
  if (entry.native) {
    await navigation.openWorkspaceLocation('plugins', {
      path: pluginPagePath(entry.plugin_id, TELEGRAM_PAGE_ID),
      title: entry.title,
    })
  } else if (entry.pages.length) {
    await openPage(entry.pages[0]!)
  } else {
    await managePlugin(entry.plugin_id)
  }
}
async function updateTelegramQuery(patch: Record<string, string>) {
  const query: Record<string, string> = {}
  for (const [key, value] of Object.entries(location.value.query))
    if (typeof value === 'string' && !['windowId', 'ocEmbed'].includes(key)) query[key] = value
  for (const [key, value] of Object.entries(patch)) {
    if (value) query[key] = value
    else delete query[key]
  }
  await navigate(pluginPagePath(TELEGRAM_PLUGIN_ID, TELEGRAM_PAGE_ID), query)
}
async function chooseTelegramSource(value: string) {
  await updateTelegramQuery({ source: value })
}
async function openTelegramSession(id: number) {
  await navigation.openWorkspaceLocation('chat', {
    path: '/chat',
    query: { sessionId: String(id) },
    matchKeys: ['sessionId'],
  })
}
async function openIntegrations() {
  await router.push({
    path: '/integrations',
    query: telegramSource.value ? { source: String(telegramSource.value) } : {},
  })
}

function updateEnvironment() {
  const next = Object.freeze({
    locale: String(locale.value),
    theme: document.documentElement.classList.contains('dark') ? ('dark' as const) : ('light' as const),
  })
  if (next.locale === environment.value.locale && next.theme === environment.value.theme) return
  environment.value = next
  for (const listener of [...environmentListeners]) {
    try {
      listener(next)
    } catch (error) {
      toasts.push('error', error instanceof Error ? error.message : String(error))
    }
  }
}

async function loadPage() {
  const currentVersion = ++version
  mounts.dispose()
  ready.value = false
  mounting.value = false
  mountError.value = ''
  const page = selected.value
  setTitle(isTelegram.value ? 'Telegram' : page?.title || String(t('pluginWeb.title')))
  if (!page || !container.value || (page.scope === 'session' && sessionId.value === null)) return
  const ownerSession = sessionId.value
  const pathParts = location.value.params.pathMatch
  const childPath = Array.isArray(pathParts) ? pathParts.join('/') : String(pathParts || '')
  mounting.value = true
  await nextTick()
  if (disposed || currentVersion !== version || !container.value) return
  try {
    const origin = new URL(apiUrl('/')).origin
    const entryUrl = pluginAssetUrl(page, page.entry_url, origin)
    const styleUrls = page.style_urls.map((url) => pluginAssetUrl(page, url, origin))
    const mounted = await mounts.mount({
      container: container.value,
      load: () => import(/* @vite-ignore */ entryUrl),
      styles: (signal) => catalog.acquireStyles(styleUrls, signal),
      sdk: (signal) =>
        createPluginUI({
          page,
          signal,
          sessionId: ownerSession,
          path: childPath ? `/${childPath}` : '/',
          request: apiJson,
          environment: () => environment.value,
          subscribeEnvironment(listener) {
            environmentListeners.add(listener)
            try {
              listener(environment.value)
            } catch (error) {
              environmentListeners.delete(listener)
              throw error
            }
            return () => environmentListeners.delete(listener)
          },
          async openPage(nextPageId) {
            const target = catalog.pages.find(
              (candidate) => candidate.plugin_id === page.plugin_id && candidate.page_id === nextPageId,
            )
            if (!target) throw new Error(String(t('pluginWeb.unavailableTitle')))
            await navigate(
              pluginPagePath(page.plugin_id, target.page_id),
              target.scope === 'session' && ownerSession !== null ? { sessionId: String(ownerSession) } : {},
            )
          },
          setTitle,
          notify: (message, tone) => toasts.push(tone, message),
          refresh: () => {
            void catalog.refresh()
          },
          messages: {
            inactive: String(t('pluginWeb.pageClosed')),
            unavailable: String(t('pluginWeb.operationUnavailable')),
            sessionRequired: String(t('pluginWeb.sessionRequired')),
          },
        }),
    })
    if (currentVersion === version && !disposed) ready.value = mounted
  } catch (error) {
    if (currentVersion === version && !disposed)
      mountError.value = error instanceof Error ? error.message : String(error)
  } finally {
    if (currentVersion === version && !disposed) mounting.value = false
  }
}

async function retry() {
  await catalog.refresh()
  retryCount.value++
}

async function selectSession(value: string) {
  if (!selected.value) return
  await navigate(location.value.path, value ? { sessionId: value } : {})
}

watch(
  () =>
    JSON.stringify([
      selected.value && [
        selected.value.plugin_id,
        selected.value.page_id,
        selected.value.generation,
        selected.value.revision,
      ],
      location.value.path,
      sessionId.value,
      retryCount.value,
      isTelegram.value,
      Boolean(container.value),
    ]),
  () => void loadPage(),
  { immediate: true, flush: 'post' },
)
watch(locale, updateEnvironment)

onMounted(() => {
  releaseCatalog = catalog.retain()
  updateEnvironment()
  themeObserver = new MutationObserver(updateEnvironment)
  themeObserver.observe(document.documentElement, { attributes: true, attributeFilter: ['class'] })
})
onBeforeUnmount(() => {
  disposed = true
  version++
  mounts.dispose()
  releaseCatalog?.()
  themeObserver?.disconnect()
  environmentListeners.clear()
})
</script>

<template>
  <section
    class="flex h-full min-h-0 min-w-0 flex-col overflow-hidden bg-background text-foreground"
    data-plugin-web-page
    :data-plugin-web-state="state"
  >
    <Teleport v-if="showSidebar" :to="WORKSPACE_SIDEBAR_PANEL_HOST_SELECTOR">
      <aside class="flex h-full min-h-0 flex-col" :aria-label="t('pluginWeb.title')">
        <div class="border-b border-sidebar-border/60 px-4 pb-4 pt-5">
          <div class="mb-3 flex items-center gap-2">
            <RiApps2Line class="h-4 w-4 text-primary" aria-hidden="true" />
            <h2 class="text-sm font-semibold">{{ t('pluginWeb.title') }}</h2>
          </div>
          <label class="relative block">
            <RiSearchLine
              class="pointer-events-none absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground"
              aria-hidden="true"
            />
            <input
              v-model="search"
              type="search"
              :aria-label="t('pluginWeb.search')"
              :placeholder="t('pluginWeb.search')"
              class="h-9 w-full rounded-lg border border-border/70 bg-background/70 pl-8 pr-2 text-xs outline-none focus:ring-2 focus:ring-ring"
            />
          </label>
        </div>
        <nav class="min-h-0 flex-1 space-y-1 overflow-y-auto p-2">
          <div v-for="entry in entries" :key="entry.plugin_id">
            <button
              type="button"
              :data-plugin-nav="entry.plugin_id"
              class="flex w-full min-w-0 items-center gap-2 rounded-lg px-3 py-2.5 text-left transition-colors hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              :class="entry.plugin_id === pluginId ? 'bg-accent text-accent-foreground' : 'text-muted-foreground'"
              :aria-current="entry.plugin_id === pluginId ? 'page' : undefined"
              @click="openPlugin(entry)"
            >
              <RiSendPlane2Line v-if="entry.native" class="h-4 w-4 shrink-0 text-sky-500" aria-hidden="true" />
              <RiApps2Line v-else class="h-4 w-4 shrink-0 opacity-60" aria-hidden="true" />
              <span class="min-w-0 flex-1">
                <span class="block truncate text-sm font-medium">{{ entry.title }}</span>
                <span class="mt-0.5 block truncate font-mono text-[10px] opacity-70">{{ entry.state }}</span>
              </span>
            </button>
            <button
              v-for="page in entry.pages.slice(1)"
              :key="pluginPageKey(page)"
              type="button"
              class="ml-6 flex w-[calc(100%-1.5rem)] min-w-0 items-center rounded-lg px-3 py-2 text-left text-xs text-muted-foreground hover:bg-accent"
              :class="page.plugin_id === pluginId && page.page_id === pageId ? 'bg-accent text-foreground' : ''"
              @click="openPage(page)"
            >
              <span class="truncate">{{ page.title }}</span>
            </button>
          </div>
          <p v-if="!entries.length" class="p-3 text-xs leading-relaxed text-muted-foreground">
            {{ t(allPlugins.length ? 'pluginWeb.noMatches' : 'pluginWeb.emptyDescription') }}
          </p>
        </nav>
        <div class="border-t border-sidebar-border/60 p-3">
          <Button variant="ghost" size="sm" class="w-full justify-start gap-2" @click="managePlugins">
            <RiSettings3Line class="h-4 w-4" aria-hidden="true" />{{ t('pluginWeb.manage') }}
          </Button>
        </div>
      </aside>
    </Teleport>

    <header v-if="pluginId" class="flex shrink-0 flex-wrap items-center gap-2 border-b border-border/60 px-4 py-3">
      <Button
        variant="ghost"
        size="sm"
        :title="t('pluginWeb.back')"
        :aria-label="t('pluginWeb.back')"
        @click="navigate('/plugins')"
      >
        <RiArrowLeftLine class="h-4 w-4" aria-hidden="true" />
      </Button>
      <div class="min-w-0 flex-1">
        <h1 class="truncate text-sm font-semibold">
          {{ isTelegram ? 'Telegram' : title || selected?.title || pageId }}
        </h1>
        <p class="truncate font-mono text-[10px] text-muted-foreground">{{ pluginId }}</p>
      </div>
      <div v-if="selected?.scope === 'session'" class="w-full sm:w-56">
        <OptionPicker
          :model-value="sessionId === null ? '' : String(sessionId)"
          :options="sessionOptions"
          :title="t('pluginWeb.chooseSession')"
          :search-placeholder="t('pluginWeb.chooseSession')"
          @update:model-value="selectSession"
        />
      </div>
      <Button
        variant="ghost"
        size="sm"
        :disabled="mounting"
        :title="t('pluginWeb.refresh')"
        :aria-label="t('pluginWeb.refresh')"
        @click="retry"
      >
        <RiRefreshLine class="h-4 w-4" :class="mounting ? 'animate-spin' : ''" aria-hidden="true" />
      </Button>
    </header>

    <div
      v-if="catalog.error && catalog.loaded"
      class="flex shrink-0 items-center justify-between gap-3 border-b border-destructive/20 bg-destructive/5 px-4 py-2 text-xs text-destructive"
      role="status"
    >
      <span>{{ catalog.error }}</span>
      <button type="button" class="shrink-0 underline underline-offset-2" @click="catalog.refresh()">
        {{ t('pluginWeb.retry') }}
      </button>
    </div>

    <div v-if="state === 'library'" class="min-h-0 flex-1 overflow-y-auto">
      <div class="mx-auto max-w-6xl px-5 py-7 sm:px-8 sm:py-10">
        <div class="mb-7 flex flex-wrap items-start justify-between gap-4">
          <div class="max-w-xl">
            <div
              class="mb-4 flex h-11 w-11 items-center justify-center rounded-2xl border border-primary/15 bg-primary/8 text-primary"
            >
              <RiApps2Line class="h-6 w-6" aria-hidden="true" />
            </div>
            <h1 class="text-2xl font-semibold tracking-tight">{{ t('pluginWeb.title') }}</h1>
            <p class="mt-2 text-sm leading-relaxed text-muted-foreground">{{ t('pluginWeb.description') }}</p>
          </div>
          <Button variant="outline" size="sm" class="gap-2" @click="managePlugins"
            ><RiSettings3Line class="h-4 w-4" aria-hidden="true" />{{ t('pluginWeb.manage') }}</Button
          >
        </div>
        <div class="mb-5 flex flex-wrap items-center justify-between gap-3">
          <p class="text-xs text-muted-foreground">{{ t('pluginWeb.count', { count: allPlugins.length }) }}</p>
          <div class="flex items-center gap-2">
            <input
              v-model="search"
              type="search"
              :aria-label="t('pluginWeb.search')"
              :placeholder="t('pluginWeb.search')"
              class="h-9 w-52 max-w-full rounded-lg border border-border bg-background px-3 text-xs outline-none focus:ring-2 focus:ring-ring"
            />
            <Button
              variant="ghost"
              size="sm"
              :disabled="catalog.loading"
              :title="t('pluginWeb.refresh')"
              :aria-label="t('pluginWeb.refresh')"
              @click="catalog.refresh()"
            >
              <RiRefreshLine class="h-4 w-4" :class="catalog.loading ? 'animate-spin' : ''" aria-hidden="true" />
            </Button>
          </div>
        </div>
        <div v-if="entries.length" class="grid gap-4 md:grid-cols-2 xl:grid-cols-3" data-plugin-inventory>
          <button
            v-for="entry in entries"
            :key="entry.plugin_id"
            type="button"
            :data-plugin-entry="entry.plugin_id"
            class="group flex min-w-0 flex-col rounded-2xl border border-border/70 bg-card p-5 text-left transition-colors hover:border-primary/40 hover:bg-accent/30 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            @click="openPlugin(entry)"
          >
            <div class="mb-5 flex w-full items-start justify-between gap-3">
              <span class="flex h-10 w-10 items-center justify-center rounded-xl bg-muted/60 text-primary">
                <RiSendPlane2Line v-if="entry.native" class="h-5 w-5 text-sky-500" aria-hidden="true" />
                <RiApps2Line v-else class="h-5 w-5" aria-hidden="true" />
              </span>
              <span class="rounded-md bg-muted/60 px-2 py-1 text-[10px] text-muted-foreground">{{ entry.state }}</span>
            </div>
            <h2 class="w-full truncate text-base font-semibold">{{ entry.title }}</h2>
            <p
              v-if="entry.title !== entry.plugin_id"
              class="mt-1 w-full truncate font-mono text-[11px] text-muted-foreground"
            >
              {{ entry.plugin_id }}
            </p>
            <p class="mt-2 line-clamp-2 min-h-9 text-xs leading-relaxed text-muted-foreground">{{ entry.summary }}</p>
            <span class="mt-5 inline-flex items-center gap-2 text-xs font-medium text-primary">
              {{ entry.native || entry.pages.length ? t('pluginWeb.open') : t('pluginWeb.manage') }}
              <RiArrowRightUpLine
                class="h-3.5 w-3.5 transition-transform group-hover:-translate-y-0.5 group-hover:translate-x-0.5"
                aria-hidden="true"
              />
            </span>
          </button>
        </div>
        <div v-if="pages.length" class="mt-8">
          <h2 class="mb-4 text-sm font-semibold">{{ t('pluginWeb.workspaces') }}</h2>
          <div class="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
            <button
              v-for="page in pages"
              :key="pluginPageKey(page)"
              type="button"
              data-plugin-page-link
              :data-plugin-id="page.plugin_id"
              :data-page-id="page.page_id"
              class="group flex min-w-0 flex-col rounded-2xl border border-border/70 bg-card p-5 text-left transition-colors hover:border-primary/40 hover:bg-accent/30 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              @click="openPage(page)"
            >
              <div class="mb-5 flex w-full items-start justify-between gap-3">
                <span class="flex h-10 w-10 items-center justify-center rounded-xl bg-muted/60 text-primary"
                  ><RiApps2Line class="h-5 w-5" aria-hidden="true"
                /></span>
                <span class="rounded-md bg-muted/60 px-2 py-1 text-[10px] text-muted-foreground">{{
                  t(page.scope === 'session' ? 'pluginWeb.sessionScope' : 'pluginWeb.instanceScope')
                }}</span>
              </div>
              <h2 class="w-full truncate text-base font-semibold">{{ page.title }}</h2>
              <p class="mt-1 w-full truncate font-mono text-[11px] text-muted-foreground">{{ page.plugin_id }}</p>
              <span class="mt-6 inline-flex items-center gap-2 text-xs font-medium text-primary"
                >{{ t('pluginWeb.open')
                }}<RiArrowRightUpLine
                  class="h-3.5 w-3.5 transition-transform group-hover:-translate-y-0.5 group-hover:translate-x-0.5"
                  aria-hidden="true"
              /></span>
            </button>
          </div>
        </div>
        <div v-else-if="!entries.length" class="rounded-2xl border border-dashed border-border px-6 py-14 text-center">
          <RiLoader4Line
            v-if="catalog.loading && !catalog.loaded"
            class="mx-auto mb-4 h-7 w-7 animate-spin text-primary"
            aria-hidden="true"
          />
          <RiApps2Line v-else class="mx-auto mb-4 h-9 w-9 text-muted-foreground/50" aria-hidden="true" />
          <h2 class="text-base font-semibold">
            {{ t(allPlugins.length ? 'pluginWeb.noMatches' : 'pluginWeb.emptyTitle') }}
          </h2>
          <p class="mx-auto mt-2 max-w-sm text-sm leading-relaxed text-muted-foreground">
            {{ catalog.error || t('pluginWeb.emptyDescription') }}
          </p>
          <Button v-if="search" variant="ghost" size="sm" class="mt-4" @click="search = ''">{{
            t('pluginWeb.clearSearch')
          }}</Button>
          <Button v-else variant="outline" size="sm" class="mt-5" @click="managePlugins">{{
            t('pluginWeb.manage')
          }}</Button>
        </div>
      </div>
    </div>

    <div v-if="state === 'telegram'" class="min-h-0 flex-1 overflow-y-auto">
      <TelegramManager
        :status="telegramStatus"
        :source="telegramSource"
        :bot-id="typeof location.query.bot === 'string' ? location.query.bot : undefined"
        :view="typeof location.query.view === 'string' ? location.query.view : undefined"
        @select-bot="updateTelegramQuery({ bot: $event })"
        @select-view="updateTelegramQuery({ view: $event })"
        @open-session="openTelegramSession"
        @source-change="chooseTelegramSource"
        @refresh="catalog.refresh()"
        @manage="managePlugins"
        @integrations="openIntegrations"
      />
    </div>
    <div v-show="state !== 'library' && state !== 'telegram'" class="relative min-h-0 flex-1">
      <div
        ref="container"
        data-plugin-web-mount
        class="h-full min-h-0 overflow-auto"
        :class="{ invisible: state !== 'ready' }"
        :aria-hidden="state !== 'ready'"
        :inert="state !== 'ready' ? true : undefined"
      />
      <div
        v-if="state !== 'ready' && state !== 'library'"
        class="absolute inset-0 flex items-center justify-center overflow-auto p-6"
        role="status"
      >
        <div class="max-w-md text-center">
          <RiLoader4Line
            v-if="state === 'loading'"
            class="mx-auto mb-4 h-7 w-7 animate-spin text-primary"
            aria-hidden="true"
          />
          <RiErrorWarningLine
            v-else-if="state === 'error'"
            class="mx-auto mb-4 h-8 w-8 text-destructive"
            aria-hidden="true"
          />
          <RiApps2Line v-else class="mx-auto mb-4 h-9 w-9 text-muted-foreground/60" aria-hidden="true" />
          <h2 class="text-base font-semibold">
            {{
              t(
                state === 'loading'
                  ? 'pluginWeb.loading'
                  : state === 'no-session'
                    ? 'pluginWeb.chooseSession'
                    : state === 'unavailable'
                      ? 'pluginWeb.unavailableTitle'
                      : 'pluginWeb.loadFailed',
              )
            }}
          </h2>
          <p v-if="state !== 'loading'" class="mt-2 break-words text-sm leading-relaxed text-muted-foreground">
            {{
              state === 'error'
                ? mountError || catalog.error
                : state === 'no-session'
                  ? sessionError || t(sessionOptions.length ? 'pluginWeb.sessionRequired' : 'pluginWeb.noSessions')
                  : t('pluginWeb.unavailableDescription')
            }}
          </p>
          <div v-if="state === 'error' || state === 'unavailable'" class="mt-5 flex flex-wrap justify-center gap-2">
            <Button variant="outline" size="sm" @click="retry">{{ t('pluginWeb.retry') }}</Button>
            <Button variant="ghost" size="sm" @click="managePlugins">{{ t('pluginWeb.manage') }}</Button>
          </div>
        </div>
      </div>
    </div>
  </section>
</template>
