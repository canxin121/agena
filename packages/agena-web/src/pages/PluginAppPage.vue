<script setup lang="ts">
import { computed, defineAsyncComponent, nextTick, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { RiArrowLeftLine, RiApps2Line, RiErrorWarningLine, RiLoader4Line, RiRefreshLine } from '@remixicon/vue'
import Button from '@/components/ui/Button.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import { useWorkspacePaneContext } from '@/app/workspace/workspacePaneContext'
import { useWorkspaceNavigation } from '@/app/navigation/useWorkspaceNavigation'
import { useUiStore } from '@/stores/ui'
import { useChatStore } from '@/stores/chat'
import { useToastsStore } from '@/stores/toasts'
import { apiJson, apiUrl } from '@/lib/api'
import { usePluginWebCatalog } from '@/features/pluginWeb/catalog'
import { TELEGRAM_PAGE_ID, TELEGRAM_PLUGIN_ID } from '@/features/pluginWeb/hub'
import {
  pluginInteractionPages,
  pluginInteractionLocation,
  pluginSettingsLocation,
  pluginReturnTab,
  type PluginInteractionPage,
} from '@/features/pluginWeb/settingsEntry'
import { createPluginUI } from '@/features/pluginWeb/createSdk'
import { PluginPageMount } from '@/features/pluginWeb/mount'
import { usePluginSessionRefresh } from '@/features/pluginWeb/usePluginSessionRefresh'
import { pluginAssetUrl, pluginPagePath, pluginPageSession } from '@/features/pluginWeb/model'
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
const applicationPages = computed(() => pluginInteractionPages(pluginId.value, catalog.pages))
const isTelegram = computed(
  () =>
    pluginId.value === TELEGRAM_PLUGIN_ID &&
    pageId.value === TELEGRAM_PAGE_ID &&
    catalog.plugins.some((plugin) => plugin.plugin_id === TELEGRAM_PLUGIN_ID),
)
const telegramStatus = computed(() => catalog.plugins.find((plugin) => plugin.plugin_id === TELEGRAM_PLUGIN_ID))
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
  if (!pluginId.value) return 'unavailable'
  if (!catalog.loaded) return catalog.error ? 'error' : 'loading'
  if (isTelegram.value) return telegramStatus.value?.state === 'running' ? 'telegram' : 'unavailable'
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

async function openPage(page: PluginInteractionPage) {
  if (page.plugin_id !== pluginId.value) return
  const target = pluginInteractionLocation(
    page,
    location.value.query.settingsTab,
    sessionId.value === null ? undefined : String(sessionId.value),
  )
  await navigate(target.path, target.query)
}
async function selectPage(id: string) {
  const page = applicationPages.value.find((candidate) => candidate.page_id === id)
  if (page) await openPage(page)
}
async function managePlugins() {
  await navigation.openMainTab('settings', pluginSettingsLocation(pluginId.value, location.value.query.settingsTab))
}
async function updateTelegramQuery(patch: Record<string, string>) {
  const query: Record<string, string> = {}
  for (const [key, value] of Object.entries(location.value.query))
    if (typeof value === 'string' && !['windowId', 'ocEmbed', 'source'].includes(key)) query[key] = value
  for (const [key, value] of Object.entries(patch)) {
    if (value) query[key] = value
    else delete query[key]
  }
  await navigate(pluginPagePath(TELEGRAM_PLUGIN_ID, TELEGRAM_PAGE_ID), query)
}
async function openTelegramSession(id: number) {
  try {
    const session = await apiJson<{ owner_bot_id?: string | null }>('/api/v1/sessions/' + id)
    if (session.owner_bot_id) {
      await navigation.openWorkspaceLocation('bot', {
        path: '/bot',
        query: { botId: session.owner_bot_id, sessionId: String(id) },
        matchKeys: ['botId', 'sessionId'],
      })
    } else {
      await navigation.openWorkspaceLocation('chat', {
        path: '/chat',
        query: { sessionId: String(id) },
        matchKeys: ['sessionId'],
      })
    }
  } catch (error) {
    toasts.push('error', error instanceof Error ? error.message : String(error))
  }
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
            await navigate(pluginPagePath(page.plugin_id, target.page_id), {
              settingsTab: pluginReturnTab(location.value.query.settingsTab),
              ...(target.scope === 'session' && ownerSession !== null ? { sessionId: String(ownerSession) } : {}),
            })
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
  await navigate(location.value.path, {
    settingsTab: pluginReturnTab(location.value.query.settingsTab),
    ...(value ? { sessionId: value } : {}),
  })
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
    <header class="flex shrink-0 flex-wrap items-center gap-2 border-b border-border/60 px-4 py-3">
      <Button
        variant="ghost"
        size="sm"
        :title="t('pluginWeb.back')"
        :aria-label="t('pluginWeb.back')"
        data-plugin-return-settings
        @click="managePlugins"
      >
        <RiArrowLeftLine class="h-4 w-4" aria-hidden="true" /><span class="hidden sm:inline">{{
          t('pluginWeb.back')
        }}</span>
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
        :disabled="mounting || catalog.loading"
        :title="t('pluginWeb.refresh')"
        :aria-label="t('pluginWeb.refresh')"
        @click="retry"
        ><RiRefreshLine class="h-4 w-4" :class="mounting || catalog.loading ? 'animate-spin' : ''" aria-hidden="true"
      /></Button>
    </header>
    <nav
      v-if="applicationPages.length > 1"
      class="flex shrink-0 gap-1 overflow-x-auto border-b border-border/50 px-4 py-2"
      :aria-label="t('pluginWeb.interactivePages')"
    >
      <Button
        v-for="page in applicationPages"
        :key="page.page_id"
        size="sm"
        :variant="page.page_id === pageId ? 'secondary' : 'ghost'"
        :aria-pressed="page.page_id === pageId"
        :data-plugin-sibling-page="page.page_id"
        @click="selectPage(page.page_id)"
        >{{ page.title }}</Button
      >
    </nav>
    <div v-if="state === 'telegram'" class="min-h-0 flex-1 overflow-y-auto">
      <TelegramManager
        :status="telegramStatus"
        :bot-id="typeof location.query.bot === 'string' ? location.query.bot : undefined"
        :view="typeof location.query.view === 'string' ? location.query.view : undefined"
        @select-bot="updateTelegramQuery({ bot: $event })"
        @select-view="updateTelegramQuery({ view: $event })"
        @open-session="openTelegramSession"
        @refresh="catalog.refresh()"
        @manage="managePlugins"
      />
    </div>
    <div v-show="state !== 'telegram'" class="relative min-h-0 flex-1">
      <div
        ref="container"
        data-plugin-web-mount
        class="h-full min-h-0 overflow-auto"
        :class="{ invisible: state !== 'ready' }"
        :aria-hidden="state !== 'ready'"
        :inert="state !== 'ready' ? true : undefined"
      />
      <div
        v-if="state !== 'ready'"
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
