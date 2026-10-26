<script setup lang="ts">
import { computed, defineAsyncComponent, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import {
  RiRefreshLine,
  RiResetLeftLine,
  RiTranslate2,
  RiPaletteLine,
  RiFontSize,
  RiLayoutLine,
  RiChat3Line,
  RiListCheck2,
  RiToolsLine,
} from '@remixicon/vue'

import { useSettingsStore, type Settings } from '../stores/settings'
import { useTranscriptPreferencesStore } from '@/stores/transcriptPreferences'
import { useUiStore } from '@/stores/ui'
import { i18n, setAppLocale } from '@/i18n'
import { normalizeAppLocale } from '@/i18n/locale'
import type { AppLocale } from '@/i18n/locale'

import SettingsCard from '@/components/settings/SettingsCard.vue'
import SettingsRow from '@/components/settings/SettingsRow.vue'
import SettingsToggle from '@/components/settings/SettingsToggle.vue'
import BrowserNotificationsPanel from '@/components/settings/BrowserNotificationsPanel.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import IconButton from '@/components/ui/IconButton.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import SettingsSidebar from '@/components/settings/sidebar/SettingsSidebar.vue'
import { settingsQueryForDestination } from '@/components/settings/settingsDestination'
import { normalizePluginSettingsView, pluginTabForSettingsView } from '@/components/settings/pluginSettingsNavigation'
import SettingsSectionWorkbench from '@/components/settings/workbench/SettingsSectionWorkbench.vue'
import {
  resolveSettingsSubpage,
  settingsSubpageStorageKey,
} from '@/components/settings/workbench/settingsSectionNavigation'
import {
  SETTINGS_DEFAULT_SUBPAGE,
  buildSettingsSidebarNodes,
  buildSettingsSubpages,
} from '@/components/settings/settingsNavigationCatalog'
import {
  buildSettingsScopes,
  findSettingsScope,
  resolveSettingsScope,
  settingsScopeStorageKey,
} from '@/components/settings/settingsScopeCatalog'
import {
  buildSettingsSidebarTabs,
  normalizeRememberedSettingsRoute,
  settingsPathForTab,
  settingsTabFromRouteValue,
  type SettingsSidebarDestination,
  type SettingsTab,
} from '@/components/settings/sidebar/settingsSidebarNavigation'
import { useDesktopSidebarResize } from '@/composables/useDesktopSidebarResize'
import { localStorageKeys } from '@/lib/persistence/storageKeys'
import { apiJson } from '@/lib/api'
import { cn } from '@/lib/utils'
import { buildLocalePickerOptions } from '@/pages/loginLocaleOptions'
import { useWorkspacePaneContext } from '@/app/workspace/workspacePaneContext'
import { usePaneVisibility } from '@/composables/usePaneVisibility'
import { WORKSPACE_SIDEBAR_PANEL_HOST_SELECTOR } from '@/layout/workspaceSidebarHost'
import {
  BUILTIN_CHAT_ACTIVITY_KINDS,
  normalizeChatActivityKindCatalog,
  normalizeChatToolActivityId,
  normalizeChatToolPreferenceId,
  type ChatActivityKindCatalogItem,
} from '@/lib/chatActivity'
import { settingsText as st } from '@/i18n/settingsText'
import { resolveTranscriptVimEnabled } from '@/pages/chat/transcriptVimPreference'

const ModelsProvidersPanel = defineAsyncComponent(() => import('@/components/settings/ModelsProvidersPanel.vue'))
const PermissionsWorkbenchPanel = defineAsyncComponent(
  () => import('@/components/settings/PermissionsWorkbenchPanel.vue'),
)
const PluginsToolsPanel = defineAsyncComponent(() => import('@/components/settings/PluginsToolsPanel.vue'))
const McpServerControlPanel = defineAsyncComponent(() => import('@/components/settings/McpServerControlPanel.vue'))
const RuntimeSessionPanel = defineAsyncComponent(() => import('@/components/settings/RuntimeSessionPanel.vue'))
const InterfaceSettingsPanel = defineAsyncComponent(() => import('@/components/settings/InterfaceSettingsPanel.vue'))
const DiagnosticsWorkbenchPanel = defineAsyncComponent(
  () => import('@/components/settings/DiagnosticsWorkbenchPanel.vue'),
)

const settings = useSettingsStore()
const transcriptPreferences = useTranscriptPreferencesStore()
const ui = useUiStore()
const workspacePane = useWorkspacePaneContext()
const visible = usePaneVisibility()
const route = useRoute()
const router = useRouter()
const { startDesktopSidebarResize, resizeDesktopSidebarFromKeyboard } = useDesktopSidebarResize()
const { t, te } = useI18n()

const SETTINGS_LAST_SECTION_KEY = localStorageKeys.settings.lastRoute

const DEFAULT_SECTION: SettingsTab = 'interface'

const interfacePages = computed(() => buildSettingsSubpages('interface'))

function readInitialSection(): SettingsTab {
  const routeSection = settingsTabFromRouteValue(route.params.section)
  if (routeSection) return routeSection
  try {
    const remembered = settingsTabFromRouteValue(localStorage.getItem(SETTINGS_LAST_SECTION_KEY))
    if (remembered) return remembered
  } catch {
    // ignore
  }
  return DEFAULT_SECTION
}

const activeSection = computed<SettingsTab>(
  () => settingsTabFromRouteValue(route.params.section) || readInitialSection(),
)
const activePages = computed(() => buildSettingsSubpages(activeSection.value))
const activeView = computed(() => {
  let remembered = ''
  try {
    remembered = localStorage.getItem(settingsSubpageStorageKey(activeSection.value)) || ''
  } catch {}
  const requested =
    activeSection.value === 'plugins-tools'
      ? normalizePluginSettingsView(route.query.view || (route.query.pluginTab ? 'plugin-workbench' : remembered))
      : route.query.view
  return resolveSettingsSubpage(requested, remembered, activePages.value, SETTINGS_DEFAULT_SUBPAGE[activeSection.value])
})

// Every scoped page resolves the same configuration layers; its workbench owns
// the visible selector while the route keeps the choice shareable.
const activeScopes = computed(() => buildSettingsScopes(activeSection.value, activeView.value))
const activeScope = computed(() => {
  let remembered = ''
  try {
    remembered = localStorage.getItem(settingsScopeStorageKey(activeSection.value)) || ''
  } catch {}
  return resolveSettingsScope(route.query.scope, remembered, activeScopes.value)
})

// The active subpage declares whether it owns the whole content area (dense
// master/detail pages such as the Model Catalog do) or scrolls inside it like
// every other settings page.
const activeSubpageDefinition = computed(() => activePages.value.find((page) => page.id === activeView.value) || null)
const contentFillsViewport = computed(() => activeSubpageDefinition.value?.fillsViewport === true)

watch(
  () =>
    [
      route.params.section,
      route.query.view,
      route.query.scope,
      route.query.pluginTab,
      activeView.value,
      activeScope.value,
    ] as const,
  () => {
    if (!settingsTabFromRouteValue(route.params.section)) return
    if (route.params.section === 'plugins-tools' && route.query.view === 'mcp-server') {
      void router.replace({ path: settingsPathForTab('mcp-server'), query: { view: 'connection' }, hash: route.hash })
      return
    }
    try {
      localStorage.setItem(settingsSubpageStorageKey(activeSection.value), activeView.value)
      localStorage.setItem(settingsScopeStorageKey(activeSection.value), activeScope.value)
    } catch {}
    const { pluginTab: _legacyPluginTab, scope: _scope, ...query } = route.query
    if (activeScope.value) query.scope = activeScope.value
    const isPluginView = activeSection.value === 'plugins-tools' && pluginTabForSettingsView(activeView.value)
    if (isPluginView) {
      const tab = pluginTabForSettingsView(route.query.view || activeView.value, route.query.pluginTab)
      if (tab && tab !== 'overview') query.pluginTab = tab
    } else delete query.plugin
    if (
      query.view === activeView.value &&
      query.scope === (activeScope.value || undefined) &&
      route.query.pluginTab === query.pluginTab &&
      (isPluginView || route.query.plugin === undefined)
    )
      return
    void router.replace({
      path: settingsPathForTab(activeSection.value),
      query: { ...query, view: activeView.value },
      hash: route.hash,
    })
  },
  { immediate: true },
)
const runtimeSessionPanel = ref<{ refresh: () => Promise<void> } | null>(null)
const diagnosticsPanel = ref<{ refresh: () => Promise<void> } | null>(null)
const interfacePanel = ref<{ refresh: () => Promise<void> } | null>(null)
const pluginsToolsPanel = ref<{ refresh: () => Promise<void> } | null>(null)
const mcpServerPanel = ref<{ refresh: () => Promise<void> } | null>(null)
const modelsProvidersPanel = ref<{ refresh: () => Promise<void> } | null>(null)
const permissionsPanel = ref<{ refresh: () => Promise<void> } | null>(null)

function goToSettingsDestination(destination: SettingsSidebarDestination) {
  const path = settingsPathForTab(destination.section)
  const query = settingsQueryForDestination(destination, activeSection.value, route.query)
  void router.push({ path, query, hash: route.hash })
  if (ui.isCompactLayout) ui.setSessionSwitcherOpen(false)
}

const isFocusedWorkspacePane = computed(() => !workspacePane || workspacePane.isFocused.value)
const useDesktopSidebarHost = computed(
  () => Boolean(workspacePane) && !ui.isCompactLayout && isFocusedWorkspacePane.value,
)
const settingsSidebarClass = computed(() =>
  ui.isCompactLayout || useDesktopSidebarHost.value
    ? 'relative h-full w-full shrink-0 border-r border-border bg-sidebar'
    : 'relative h-full w-full shrink-0 bg-sidebar',
)

const showSidebar = computed(() => {
  if (ui.isCompactLayout) return ui.isSessionSwitcherOpen
  if (workspacePane && !isFocusedWorkspacePane.value) return false
  return true
})

const tabs = computed(() =>
  buildSettingsSidebarTabs(
    (_id, labelKey) => String(t(labelKey)),
    (section) => buildSettingsSidebarNodes(section),
  ),
)

async function refreshSettingsSidebar() {
  // Configuration editors refresh in place so unsaved drafts survive.
  if (activeSection.value === 'plugins-tools') {
    await pluginsToolsPanel.value?.refresh()
  } else if (activeSection.value === 'mcp-server') {
    await mcpServerPanel.value?.refresh()
  } else if (activeSection.value === 'models-providers') {
    await modelsProvidersPanel.value?.refresh()
  } else if (activeSection.value === 'permissions') {
    await permissionsPanel.value?.refresh()
  } else if (activeSection.value === 'runtime-session') {
    await runtimeSessionPanel.value?.refresh()
  } else if (activeSection.value === 'diagnostics') {
    await diagnosticsPanel.value?.refresh()
  } else if (activeView.value === 'tui') {
    await interfacePanel.value?.refresh()
  } else if (needsChatToolCatalog.value) await loadChatToolCatalog()
  await settings.refresh()
  await transcriptPreferences.refresh()
}

onMounted(async () => {
  if (!settings.data && !settings.loading) {
    await settings.refresh()
  }
  await transcriptPreferences.refresh()
})

watch(
  () => route.params.section,
  (value) => {
    const section = settingsTabFromRouteValue(value)
    if (section) {
      try {
        localStorage.setItem(SETTINGS_LAST_SECTION_KEY, settingsPathForTab(activeSection.value))
      } catch {
        // ignore storage failures
      }
      return
    }

    let target = settingsPathForTab(activeSection.value)
    try {
      target = normalizeRememberedSettingsRoute(localStorage.getItem(SETTINGS_LAST_SECTION_KEY), activeSection.value)
    } catch {
      // ignore storage failures
    }
    void router.replace({ path: target, query: route.query, hash: route.hash })
  },
  { immediate: true },
)

watch(
  () => route.fullPath,
  (path) => {
    const fullPath = String(path || '')
    if (!fullPath.startsWith('/settings')) return
    // Keep the previous behavior of opening the settings sidebar when arriving
    // on the page in a non-compact layout.
    if (ui.isCompactLayout) return
    ui.setSidebarOpen(true, { preserveWidth: true })
  },
  { immediate: true },
)

function joinWindowTitle(parts: Array<string | null | undefined>) {
  return parts
    .map((part) => String(part || '').trim())
    .filter(Boolean)
    .join(' · ')
}

const settingsWindowTitle = computed(() => {
  const base = String(t('settings.title'))
  const activeTabLabel = tabs.value.find((tab) => tab.id === activeSection.value)?.label || base
  const pageLabel = activePages.value.find((page) => page.id === activeView.value)?.label
  const scopeLabel = findSettingsScope(activeSection.value, activeView.value, activeScope.value)?.label
  return joinWindowTitle([base, activeTabLabel, pageLabel, scopeLabel])
})

watch(
  () => [route.path, route.query, settingsWindowTitle.value] as const,
  ([path]) => {
    if (!String(path || '').startsWith('/settings')) return
    ui.setWorkspaceWindowTitleFromRoute(route.query, settingsWindowTitle.value)
  },
  { immediate: true, deep: true },
)

function makeSetting<K extends keyof Settings>(key: K, fallback: NonNullable<Settings[K]>) {
  return computed<NonNullable<Settings[K]>>({
    get() {
      const v = settings.data?.[key]
      // IMPORTANT: Only return the value without mutation side effects.
      return (v ?? fallback) as NonNullable<Settings[K]>
    },
    set(value: NonNullable<Settings[K]>) {
      // Direct store update without forcing a recursive re-compute of this same getter.
      void settings.save({ [key]: value } as Pick<Settings, K>)
    },
  })
}

const useSystemTheme = makeSetting('useSystemTheme', true)
const themeVariant = makeSetting('themeVariant', 'dark')
const uiFont = makeSetting('uiFont', 'ibm-plex-sans')
const monoFont = makeSetting('monoFont', 'ibm-plex-mono')
const fontSize = makeSetting('fontSize', 90)
const padding = makeSetting('padding', 100)
const cornerRadius = makeSetting('cornerRadius', 10)
const inputBarOffset = makeSetting('inputBarOffset', 0)

const uiLocale = computed<AppLocale>({
  get() {
    return i18n.global.locale.value as AppLocale
  },
  set(value) {
    setAppLocale(value)
  },
})

const localePickerOptions = computed(() => buildLocalePickerOptions((key) => String(t(key))))

const themeVariantPickerOptions = computed(() => [
  { value: 'light', label: String(t('settings.appearance.theme.options.light')) },
  { value: 'dark', label: String(t('settings.appearance.theme.options.dark')) },
])

const uiFontPickerOptions = computed(() => [
  { value: 'system', label: String(t('settings.appearance.fonts.options.system')) },
  { value: 'ibm-plex-sans', label: String(t('settings.appearance.fonts.options.ibmPlexSans')) },
  { value: 'atkinson', label: String(t('settings.appearance.fonts.options.atkinson')) },
  { value: 'serif', label: String(t('settings.appearance.fonts.options.serif')) },
])

const monoFontPickerOptions = computed(() => [
  { value: 'system', label: String(t('settings.appearance.fonts.options.system')) },
  { value: 'ibm-plex-mono', label: String(t('settings.appearance.fonts.options.ibmPlexMono')) },
  { value: 'jetbrains-mono', label: String(t('settings.appearance.fonts.options.jetbrainsMono')) },
])

const showChatTimestamps = makeSetting('showChatTimestamps', true)
const showReasoningTraces = makeSetting('showReasoningTraces', true)

// The stored value wins, but before the user ever touches the switch the
// effective default is device-based (desktop on, mobile off), so the checkbox
// shows the mode the transcript is actually running.
const chatTranscriptVim = computed<boolean>({
  get() {
    return resolveTranscriptVimEnabled(settings.data?.chatTranscriptVim, ui.isMobileDevice)
  },
  set(value: boolean) {
    void settings.save({ chatTranscriptVim: value })
  },
})

const chatActivityAutoCollapseOnIdle = makeSetting('chatActivityAutoCollapseOnIdle', true)

const transcriptActivityDefaultExpanded = computed({
  get: () => transcriptPreferences.activityDefaultExpanded,
  set: (value: boolean) => transcriptPreferences.setActivityDefaultExpanded(value),
})

function activityKindDefaultExpandedEnabled(id: string): boolean {
  return transcriptPreferences.kindExpanded(id)
}

function toggleActivityKindDefaultExpanded(id: string) {
  const normalizedId = String(id || '').trim()
  if (!normalizedId) return
  transcriptPreferences.setActivityKindExpanded(normalizedId, !activityKindDefaultExpandedEnabled(normalizedId))
}

type ToolCatalogItem = {
  name?: string
  summary?: string
  summary_translations?: Record<string, string>
  tags?: string[]
}

type ToolCatalogResponse = {
  permission_tools?: ToolCatalogItem[]
  activity_kinds?: unknown
}

type ToolExpansionOption = {
  id: string
  label: string
  description: string
}

const toolApiFunctions = computed<ToolExpansionOption[]>(() => [
  { id: 'tools_list', label: 'tools_list', description: st('Enumerate execution tools.') },
  { id: 'tools_help', label: 'tools_help', description: st('Inspect execution-tool contracts.') },
  { id: 'tools_call', label: 'tools_call', description: st('Invoke an execution tool.') },
  { id: 'plugins_list', label: 'plugins_list', description: st('Enumerate tool plugins.') },
])

const toolCatalogItems = ref<ToolCatalogItem[]>([])
const activityKindCatalogItems = ref<ChatActivityKindCatalogItem[]>(BUILTIN_CHAT_ACTIVITY_KINDS.slice())
const toolCatalogLoading = ref(false)
const toolCatalogError = ref('')
const toolCatalogQuery = ref('')
let toolCatalogLoaded = false
let toolCatalogController: AbortController | undefined
const needsChatToolCatalog = computed(() => activeSection.value === 'interface' && activeView.value === 'conversation')

async function loadChatToolCatalog() {
  if (!visible.value || !needsChatToolCatalog.value || toolCatalogLoading.value) return
  const controller = new AbortController()
  toolCatalogController = controller
  toolCatalogLoading.value = true
  toolCatalogError.value = ''
  try {
    const response = await apiJson<ToolCatalogResponse>('/api/v1/plugins/surface', {
      signal: AbortSignal.any([controller.signal, AbortSignal.timeout(15_000)]),
    })
    if (controller.signal.aborted || toolCatalogController !== controller) return
    toolCatalogItems.value = Array.isArray(response?.permission_tools) ? response.permission_tools : []
    const activityKinds = normalizeChatActivityKindCatalog(response?.activity_kinds)
    activityKindCatalogItems.value = activityKinds.length ? activityKinds : BUILTIN_CHAT_ACTIVITY_KINDS.slice()
    toolCatalogLoaded = true
  } catch (error) {
    if (controller.signal.aborted || toolCatalogController !== controller) return
    toolCatalogError.value = error instanceof Error ? error.message : String(error)
  } finally {
    if (toolCatalogController === controller) {
      toolCatalogController = undefined
      toolCatalogLoading.value = false
    }
  }
}
watch(
  [visible, needsChatToolCatalog],
  ([shown, needed]) => {
    if (shown && needed) {
      if (!toolCatalogLoaded) void loadChatToolCatalog()
    } else {
      toolCatalogController?.abort()
      toolCatalogController = undefined
      toolCatalogLoading.value = false
    }
  },
  { immediate: true, flush: 'sync' },
)
onBeforeUnmount(() => toolCatalogController?.abort())

const activityKindOptions = computed(() => activityKindCatalogItems.value)

function activityKindLabel(item: ChatActivityKindCatalogItem): string {
  const key = `settings.appearance.chat.activityKinds.${item.id}.label`
  return te(key) ? String(t(key)) : item.label
}

function activityKindDescription(item: ChatActivityKindCatalogItem): string {
  const key = `settings.appearance.chat.activityKinds.${item.id}.description`
  if (te(key)) return String(t(key))
  if (item.category === 'plugin') return String(t('settings.appearance.chat.pluginActivityKind'))
  return item.id
}

/// Runtime tool summaries arrive with UI-only manifest translations; the
/// permission editor shows the client language and falls back to English.
function localizedToolSummary(item: ToolCatalogItem): string {
  const locale = normalizeAppLocale(i18n.global.locale.value)
  const translations = item.summary_translations || {}
  const localized = locale ? translations[locale] : undefined
  return String(localized || item.summary || '').trim()
}

const toolActivityOptions = computed<ToolExpansionOption[]>(() => {
  const byId = new Map<string, ToolExpansionOption>()
  for (const option of toolApiFunctions.value) byId.set(option.id, option)
  for (const item of toolCatalogItems.value) {
    const label = String(item.name || '').trim()
    const id = normalizeChatToolPreferenceId(label)
    if (!id) continue
    const summary = localizedToolSummary(item)
    const tags = Array.isArray(item.tags) ? item.tags.map((tag) => String(tag).trim()).filter(Boolean) : []
    byId.set(id, {
      id,
      label,
      description: summary || tags.join(' · '),
    })
  }
  return [...byId.values()].sort((a, b) => a.label.localeCompare(b.label))
})

const filteredToolActivityOptions = computed(() => {
  const query = toolCatalogQuery.value.trim().toLowerCase()
  if (!query) return toolActivityOptions.value
  return toolActivityOptions.value.filter((option) => {
    return `${option.label}\n${option.description}`.toLowerCase().includes(query)
  })
})

function toolDefaultExpandedEnabled(toolId: string): boolean {
  const exact = transcriptPreferences.toolExpanded(toolId)
  if (exact !== undefined) return exact
  const category = normalizeChatToolActivityId(toolId)
  if (Object.prototype.hasOwnProperty.call(transcriptPreferences.toolCategoryOverrides, category)) {
    return transcriptPreferences.toolCategoryOverrides[category] === true
  }
  if (!category && Object.prototype.hasOwnProperty.call(transcriptPreferences.toolCategoryOverrides, 'unknown')) {
    return transcriptPreferences.toolCategoryOverrides.unknown === true
  }
  return transcriptPreferences.kindExpanded('operation')
}

function toolDefaultExpandedCustomized(toolId: string): boolean {
  const id = normalizeChatToolPreferenceId(toolId)
  return Boolean(id && Object.prototype.hasOwnProperty.call(transcriptPreferences.toolOverrides, id))
}

function toggleToolDefaultExpanded(toolId: string) {
  const id = normalizeChatToolPreferenceId(toolId)
  if (!id) return
  transcriptPreferences.setToolExpanded(id, !toolDefaultExpandedEnabled(id))
}

function resetToolDefaultExpanded(toolId: string) {
  const id = normalizeChatToolPreferenceId(toolId)
  if (!id || !toolDefaultExpandedCustomized(id)) return
  transcriptPreferences.clearToolOverride(id)
}

const dirtyHint = computed(
  () => settings.error || (needsChatToolCatalog.value ? transcriptPreferences.error : '') || null,
)
</script>

<template>
  <div class="settings-page flex h-full flex-col overflow-hidden bg-background text-foreground">
    <div class="flex flex-1 overflow-hidden">
      <Teleport :to="WORKSPACE_SIDEBAR_PANEL_HOST_SELECTOR" :disabled="!useDesktopSidebarHost">
        <aside
          v-if="showSidebar"
          :class="settingsSidebarClass"
          :style="ui.isCompactLayout || useDesktopSidebarHost ? undefined : { width: `${ui.sidebarWidth}px` }"
        >
          <div
            v-if="!ui.isCompactLayout && !useDesktopSidebarHost"
            class="absolute right-0 top-0 z-10 h-full w-1 cursor-col-resize hover:bg-primary/40"
            @pointerdown="startDesktopSidebarResize"
            role="separator"
            aria-orientation="vertical"
            :aria-label="String(t('header.windowTabs.resizingSidebar'))"
            :aria-valuenow="Math.round(ui.sidebarWidth)"
            :aria-valuemin="280"
            :aria-valuemax="520"
            tabindex="0"
            @keydown="resizeDesktopSidebarFromKeyboard"
          />
          <SettingsSidebar
            :tabs="tabs"
            :active-tab="activeSection"
            :active-view="activeView"
            :active-scope="activeScope"
            :loading="settings.loading"
            :is-touch-pointer="ui.isTouchPointer"
            @refresh="refreshSettingsSidebar"
            @navigate="goToSettingsDestination"
          />
        </aside>
      </Teleport>

      <!-- Content -->
      <main
        class="flex-1 min-w-0 overflow-y-auto bg-background"
        v-show="!ui.isCompactLayout || !ui.isSessionSwitcherOpen"
      >
        <div
          :class="
            cn(
              'w-full',
              contentFillsViewport
                ? 'mx-auto flex max-w-6xl flex-col gap-4 p-4 lg:mx-0 lg:h-full lg:max-w-none lg:min-h-0 lg:gap-6 lg:p-6'
                : 'space-y-6 p-4 lg:p-6',
            )
          "
        >
          <div
            v-if="dirtyHint"
            class="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-destructive"
          >
            {{ dirtyHint }}
          </div>

          <!-- Interface: TUI, Web appearance, and conversation display are independent subpages. -->
          <SettingsSectionWorkbench
            v-if="activeSection === 'interface'"
            section="interface"
            :pages="interfacePages"
            :active-page="activeView"
            :default-page="SETTINGS_DEFAULT_SUBPAGE.interface"
            v-slot="{ activePage }"
          >
            <InterfaceSettingsPanel v-if="activePage === 'tui'" ref="interfacePanel" />
            <div v-else-if="activePage === 'web-appearance'" class="space-y-4">
              <BrowserNotificationsPanel />
              <div class="grid gap-4 xl:grid-cols-2">
                <SettingsCard
                  :title="t('settings.appearance.language.label')"
                  :description="t('settings.appearance.language.help')"
                  :icon="RiTranslate2"
                >
                  <OptionPicker
                    v-model="uiLocale"
                    :options="localePickerOptions"
                    :title="t('settings.appearance.language.label')"
                    :search-placeholder="t('settings.appearance.language.label')"
                    :include-empty="false"
                  />
                </SettingsCard>
                <SettingsCard :title="t('settings.appearance.theme.label')" :icon="RiPaletteLine">
                  <SettingsRow :title="t('settings.appearance.theme.useSystem')" toggle>
                    <SettingsToggle v-model="useSystemTheme" :label="t('settings.appearance.theme.useSystem')" />
                  </SettingsRow>
                  <OptionPicker
                    v-model="themeVariant"
                    class="mt-3"
                    :disabled="useSystemTheme"
                    :options="themeVariantPickerOptions"
                    :title="t('settings.appearance.theme.pickerTitle')"
                    :search-placeholder="t('settings.appearance.theme.pickerSearch')"
                    :include-empty="false"
                  />
                </SettingsCard>
              </div>
              <SettingsCard :title="t('settings.appearance.fonts.label')" :icon="RiFontSize">
                <div class="grid gap-4 sm:grid-cols-2">
                  <div class="min-w-0 space-y-2">
                    <div class="text-xs font-medium text-muted-foreground">{{ t('settings.appearance.fonts.ui') }}</div>
                    <OptionPicker
                      v-model="uiFont"
                      :options="uiFontPickerOptions"
                      :title="t('settings.appearance.fonts.ui')"
                      :search-placeholder="t('settings.appearance.fonts.search')"
                      :include-empty="false"
                    />
                  </div>
                  <div class="min-w-0 space-y-2">
                    <div class="text-xs font-medium text-muted-foreground">
                      {{ t('settings.appearance.fonts.mono') }}
                    </div>
                    <OptionPicker
                      v-model="monoFont"
                      :options="monoFontPickerOptions"
                      :title="t('settings.appearance.fonts.mono')"
                      :search-placeholder="t('settings.appearance.fonts.search')"
                      :include-empty="false"
                    />
                  </div>
                </div>
              </SettingsCard>
              <SettingsCard :title="t('settings.appearance.sizing.label')" :icon="RiLayoutLine">
                <div class="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
                  <label class="grid min-w-0 gap-2 text-xs font-medium text-muted-foreground">
                    {{ t('settings.appearance.sizing.fontSize') }}
                    <input v-model.number="fontSize" type="number" min="70" max="140" class="settings-number-input" />
                  </label>
                  <label class="grid min-w-0 gap-2 text-xs font-medium text-muted-foreground">
                    {{ t('settings.appearance.sizing.padding') }}
                    <input v-model.number="padding" type="number" min="70" max="140" class="settings-number-input" />
                  </label>
                  <label class="grid min-w-0 gap-2 text-xs font-medium text-muted-foreground">
                    {{ t('settings.appearance.sizing.cornerRadius') }}
                    <input v-model.number="cornerRadius" type="number" min="0" max="28" class="settings-number-input" />
                  </label>
                  <label class="grid min-w-0 gap-2 text-xs font-medium text-muted-foreground">
                    {{ t('settings.appearance.sizing.inputBarOffset') }}
                    <input
                      v-model.number="inputBarOffset"
                      type="number"
                      min="-40"
                      max="80"
                      class="settings-number-input"
                    />
                  </label>
                </div>
              </SettingsCard>
              <p v-if="!dirtyHint" class="flex items-center gap-1.5 text-xs text-muted-foreground">
                {{ $st('Saved automatically') }}
              </p>
            </div>
            <div v-else class="space-y-4">
              <SettingsCard :title="t('settings.appearance.chat.label')" :icon="RiChat3Line">
                <SettingsRow :title="t('settings.appearance.chat.showTimestamps')" toggle>
                  <SettingsToggle v-model="showChatTimestamps" :label="t('settings.appearance.chat.showTimestamps')" />
                </SettingsRow>
                <SettingsRow :title="t('settings.appearance.chat.showReasoning')" toggle>
                  <SettingsToggle v-model="showReasoningTraces" :label="t('settings.appearance.chat.showReasoning')" />
                </SettingsRow>
                <SettingsRow :title="t('settings.appearance.chat.autoCollapseActivity')" toggle>
                  <SettingsToggle
                    v-model="chatActivityAutoCollapseOnIdle"
                    :label="t('settings.appearance.chat.autoCollapseActivity')"
                  />
                </SettingsRow>
                <SettingsRow
                  :title="t('settings.appearance.chat.transcriptVim')"
                  :description="t('settings.appearance.chat.transcriptVimHint')"
                  toggle
                >
                  <SettingsToggle v-model="chatTranscriptVim" :label="t('settings.appearance.chat.transcriptVim')" />
                </SettingsRow>
              </SettingsCard>
              <SettingsCard :title="t('settings.appearance.chat.activityDetails')" :icon="RiListCheck2">
                <SettingsRow :title="$st('Expand activity parts by default unless a type has its own setting')" toggle>
                  <SettingsToggle
                    v-model="transcriptActivityDefaultExpanded"
                    :label="$st('Expand activity parts by default unless a type has its own setting')"
                  />
                </SettingsRow>
                <div class="mt-3 grid gap-2 xl:grid-cols-2">
                  <div
                    v-for="opt in activityKindOptions"
                    :key="opt.id"
                    class="flex min-w-0 items-center gap-3 rounded-lg border border-border/50 bg-background/30 p-3"
                  >
                    <div class="min-w-0 flex-1">
                      <div class="text-sm font-medium">{{ activityKindLabel(opt) }}</div>
                      <p class="mt-1 text-xs leading-relaxed text-muted-foreground">
                        {{ activityKindDescription(opt) }}
                      </p>
                    </div>
                    <SettingsToggle
                      :model-value="activityKindDefaultExpandedEnabled(opt.id)"
                      :label="activityKindLabel(opt)"
                      @update:model-value="toggleActivityKindDefaultExpanded(opt.id)"
                    />
                  </div>
                </div>
              </SettingsCard>
              <SettingsCard
                :title="t('settings.appearance.chat.toolDetails')"
                :icon="RiToolsLine"
                :count="toolActivityOptions.length"
              >
                <template #actions>
                  <IconButton
                    variant="outline"
                    size="sm"
                    :disabled="toolCatalogLoading"
                    :aria-label="t('settings.appearance.chat.refreshTools')"
                    :tooltip="t('settings.appearance.chat.refreshTools')"
                    @click="loadChatToolCatalog"
                  >
                    <RiRefreshLine class="h-4 w-4" :class="toolCatalogLoading ? 'animate-spin' : ''" />
                  </IconButton>
                </template>
                <SearchInput
                  v-model="toolCatalogQuery"
                  class="mb-3"
                  :placeholder="t('settings.appearance.chat.searchTools')"
                  :input-aria-label="t('settings.appearance.chat.searchTools')"
                  :show-search-button="false"
                />
                <p
                  v-if="toolCatalogError"
                  role="alert"
                  class="mb-3 break-words rounded-lg border border-destructive/30 bg-destructive/10 p-3 text-xs text-destructive"
                >
                  {{ toolCatalogError }}
                </p>
                <p
                  v-if="toolCatalogLoading && !toolCatalogItems.length"
                  class="py-4 text-center text-sm text-muted-foreground"
                >
                  {{ t('settings.appearance.chat.loadingTools') }}
                </p>
                <div v-else-if="filteredToolActivityOptions.length" class="divide-y divide-border/50">
                  <div
                    v-for="opt in filteredToolActivityOptions"
                    :key="opt.id"
                    class="flex min-w-0 items-center gap-3 py-3 first:pt-0"
                  >
                    <div class="min-w-0 flex-1">
                      <div class="break-all font-mono text-xs font-medium">{{ opt.label }}</div>
                      <p v-if="opt.description" class="mt-1 break-words text-xs leading-relaxed text-muted-foreground">
                        {{ opt.description }}
                      </p>
                    </div>
                    <div class="flex shrink-0 items-center gap-2">
                      <IconButton
                        v-if="toolDefaultExpandedCustomized(opt.id)"
                        variant="ghost"
                        size="sm"
                        :tooltip="t('settings.appearance.chat.resetToolDefault')"
                        :aria-label="t('settings.appearance.chat.resetToolDefault')"
                        @click="resetToolDefaultExpanded(opt.id)"
                        ><RiResetLeftLine class="h-4 w-4"
                      /></IconButton>
                      <SettingsToggle
                        :model-value="toolDefaultExpandedEnabled(opt.id)"
                        :label="opt.label"
                        @update:model-value="toggleToolDefaultExpanded(opt.id)"
                      />
                    </div>
                  </div>
                </div>
                <p v-else class="rounded-lg bg-muted/20 p-6 text-center text-sm text-muted-foreground">
                  {{ t('settings.appearance.chat.noTools') }}
                </p>
                <p class="mt-3 border-t border-border/50 pt-3 text-xs leading-relaxed text-muted-foreground">
                  {{ t('settings.appearance.chat.toolDetailsHint') }}
                </p>
              </SettingsCard>
            </div>
          </SettingsSectionWorkbench>

          <!-- Models & Providers -->
          <ModelsProvidersPanel
            ref="modelsProvidersPanel"
            :active-page="activeView"
            :scope="activeScope"
            v-else-if="activeSection === 'models-providers'"
          />

          <!-- Permissions -->
          <PermissionsWorkbenchPanel
            ref="permissionsPanel"
            :active-page="activeView"
            :scope="activeScope"
            v-else-if="activeSection === 'permissions'"
          />

          <!-- Plugins & Tools -->
          <PluginsToolsPanel
            ref="pluginsToolsPanel"
            :active-page="activeView"
            v-else-if="activeSection === 'plugins-tools'"
          />

          <!-- MCP Server -->
          <SettingsSectionWorkbench
            v-else-if="activeSection === 'mcp-server'"
            section="mcp-server"
            :pages="activePages"
            :active-page="activeView"
            :default-page="SETTINGS_DEFAULT_SUBPAGE['mcp-server']"
          >
            <McpServerControlPanel ref="mcpServerPanel" />
          </SettingsSectionWorkbench>

          <!-- Runtime & Session -->
          <RuntimeSessionPanel
            ref="runtimeSessionPanel"
            :active-page="activeView"
            v-else-if="activeSection === 'runtime-session'"
          />

          <DiagnosticsWorkbenchPanel
            ref="diagnosticsPanel"
            :active-page="activeView"
            :scope="activeScope"
            v-else-if="activeSection === 'diagnostics'"
          />

          <div v-else class="flex flex-col items-center justify-center h-64 text-muted-foreground">
            <p>{{ t('settings.unknownTab') }}</p>
          </div>
        </div>
      </main>
    </div>
  </div>
</template>

<style scoped>
@reference '../style.css';
.settings-number-input {
  @apply h-9 w-full min-w-0 rounded-md border border-input bg-background px-3 text-sm font-normal text-foreground outline-none focus:border-ring;
}

.settings-page :deep(input[type='checkbox']),
.settings-page :deep(input[type='radio']) {
  accent-color: oklch(var(--muted-foreground));
}
</style>
