<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { RiRefreshLine, RiResetLeftLine } from '@remixicon/vue'

import { useSettingsStore, type Settings } from '../stores/settings'
import { useUiStore } from '@/stores/ui'
import { i18n, setAppLocale } from '@/i18n'
import type { AppLocale } from '@/i18n/locale'

import OptionPicker from '@/components/ui/OptionPicker.vue'
import SettingsSidebar from '@/components/settings/sidebar/SettingsSidebar.vue'
import ModelsProvidersPanel from '@/components/settings/ModelsProvidersPanel.vue'
import PermissionsWorkbenchPanel from '@/components/settings/PermissionsWorkbenchPanel.vue'
import PluginsToolsPanel from '@/components/settings/PluginsToolsPanel.vue'
import RuntimeSessionPanel from '@/components/settings/RuntimeSessionPanel.vue'
import InterfaceSettingsPanel from '@/components/settings/InterfaceSettingsPanel.vue'
import DiagnosticsWorkbenchPanel from '@/components/settings/DiagnosticsWorkbenchPanel.vue'
import SettingsSectionWorkbench from '@/components/settings/workbench/SettingsSectionWorkbench.vue'
import { SETTINGS_DEFAULT_SUBPAGE, buildSettingsSubpages } from '@/components/settings/settingsNavigationCatalog'
import {
  buildSettingsSidebarTabs,
  normalizeRememberedSettingsRoute,
  settingsPathForTab,
  settingsTabFromRouteValue,
  canonicalSettingsTab,
  type SettingsSidebarDestination,
  type SettingsTab,
} from '@/components/settings/sidebar/settingsSidebarNavigation'
import { useDesktopSidebarResize } from '@/composables/useDesktopSidebarResize'
import { localStorageKeys } from '@/lib/persistence/storageKeys'
import { apiJson } from '@/lib/api'
import { buildLocalePickerOptions } from '@/pages/loginLocaleOptions'
import { useWorkspacePaneContext } from '@/app/workspace/workspacePaneContext'
import { WORKSPACE_SIDEBAR_PANEL_HOST_SELECTOR } from '@/layout/workspaceSidebarHost'
import {
  BUILTIN_CHAT_ACTIVITY_KINDS,
  DEFAULT_CHAT_ACTIVITY_EXPANDED_TOOL_FILTERS,
  normalizeChatActivityKindCatalog,
  normalizeChatActivityKindDefaultExpanded,
  normalizeChatToolActivityFilters,
  normalizeChatToolExpansionOverrides,
  normalizeChatToolPreferenceId,
  resolveChatActivityKindDefaultExpanded,
  resolveChatToolDefaultExpanded,
  type ChatActivityKindCatalogItem,
  type ChatToolExpansionOverrides,
  type ChatToolActivityType,
} from '@/lib/chatActivity'
import { settingsText as st } from '@/i18n/settingsText'

const settings = useSettingsStore()
const ui = useUiStore()
const workspacePane = useWorkspacePaneContext()
const route = useRoute()
const router = useRouter()
const { startDesktopSidebarResize } = useDesktopSidebarResize()
const { t, te } = useI18n()

const SETTINGS_LAST_SECTION_KEY = localStorageKeys.settings.lastRoute

const DEFAULT_SECTION: SettingsTab = 'interface'

const interfacePages = computed(() => buildSettingsSubpages('interface'))

function readInitialSection(): SettingsTab {
  const routeSection = settingsTabFromRouteValue(route.params.section)
  if (routeSection) return canonicalSettingsTab(routeSection)
  try {
    const remembered = settingsTabFromRouteValue(localStorage.getItem(SETTINGS_LAST_SECTION_KEY))
    if (remembered) return canonicalSettingsTab(remembered)
  } catch {
    // ignore
  }
  return DEFAULT_SECTION
}

const activeSection = ref<SettingsTab>(readInitialSection())
const settingsRefreshNonce = ref(0)

watch(
  () => i18n.global.locale.value,
  () => {
    // Most dense Settings panels build option catalogs during setup. Remount
    // the active workbench so every script-side label changes immediately
    // when the browser locale changes.
    settingsRefreshNonce.value += 1
  },
)

function goToSettingsDestination(destination: SettingsSidebarDestination) {
  const path = settingsPathForTab(destination.section)
  // A plugin detail deep link remains attached only while staying on Plugin
  // Workbench. Other destinations receive the shared workspace/window scope
  // query without stale Settings routing state.
  const { view: _view, plugin, pluginTab, ...scopeQuery } = route.query
  const query = { ...scopeQuery, view: destination.view }
  if (
    destination.section === 'plugins-tools' &&
    destination.view === 'plugin-workbench' &&
    activeSection.value === 'plugins-tools'
  ) {
    if (plugin !== undefined) query.plugin = plugin
    if (pluginTab !== undefined) query.pluginTab = pluginTab
  }
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
    (section) => buildSettingsSubpages(section),
  ),
)

async function refreshSettingsSidebar() {
  // Remount the active server-backed workbench as well as refreshing the
  // browser-only preference store. This gives the sidebar refresh control the
  // same practical meaning as opening Settings again in the TUI.
  settingsRefreshNonce.value += 1
  await settings.refresh()
}

onMounted(() => {
  if (!settings.data && !settings.loading) {
    void settings.refresh()
  }
  void loadChatToolCatalog()
})

watch(
  () => route.params.section,
  (value) => {
    const section = settingsTabFromRouteValue(value)
    if (section) {
      activeSection.value = canonicalSettingsTab(section)
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
  return joinWindowTitle([base, activeTabLabel])
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

const chatActivityAutoCollapseOnIdle = makeSetting('chatActivityAutoCollapseOnIdle', true)

const chatActivityKindDefaultExpanded = computed<string[]>({
  get() {
    return resolveChatActivityKindDefaultExpanded(settings.data)
  },
  set(value) {
    void settings.save({
      chatActivityKindDefaultExpanded: normalizeChatActivityKindDefaultExpanded(value),
    })
  },
})

function activityKindDefaultExpandedEnabled(id: string): boolean {
  return chatActivityKindDefaultExpanded.value.includes(id)
}

function toggleActivityKindDefaultExpanded(id: string) {
  const normalizedId = String(id || '').trim()
  if (!normalizedId) return
  const next = new Set(chatActivityKindDefaultExpanded.value)
  if (next.has(normalizedId)) next.delete(normalizedId)
  else next.add(normalizedId)
  const catalogOrder = activityKindOptions.value.map((item) => item.id)
  const ordered = catalogOrder.filter((item) => next.has(item))
  const remaining = [...next].filter((item) => !catalogOrder.includes(item)).sort()
  chatActivityKindDefaultExpanded.value = [...ordered, ...remaining]
}

const chatActivityDefaultExpandedToolFilters = computed<ChatToolActivityType[]>({
  get() {
    const s = settings.data
    if (s && Object.prototype.hasOwnProperty.call(s, 'chatActivityDefaultExpandedToolFilters')) {
      return normalizeChatToolActivityFilters(s.chatActivityDefaultExpandedToolFilters)
    }
    return DEFAULT_CHAT_ACTIVITY_EXPANDED_TOOL_FILTERS.slice()
  },
  set(value) {
    void settings.save({ chatActivityDefaultExpandedToolFilters: value })
  },
})

type ToolCatalogItem = {
  name?: string
  summary?: string
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
  { id: 'tools_search', label: 'tools_search', description: st('Search execution tools.') },
  { id: 'tools_help', label: 'tools_help', description: st('Inspect execution-tool contracts.') },
  { id: 'tools_tags', label: 'tools_tags', description: st('List execution-tool tags.') },
  { id: 'tools_call', label: 'tools_call', description: st('Invoke an execution tool.') },
  { id: 'plugins_list', label: 'plugins_list', description: st('Enumerate tool plugins.') },
  { id: 'plugins_search', label: 'plugins_search', description: st('Search tool plugins.') },
  { id: 'plugins_tags', label: 'plugins_tags', description: st('List tool-plugin tags.') },
])

const toolCatalogItems = ref<ToolCatalogItem[]>([])
const activityKindCatalogItems = ref<ChatActivityKindCatalogItem[]>(BUILTIN_CHAT_ACTIVITY_KINDS.slice())
const toolCatalogLoading = ref(false)
const toolCatalogError = ref('')
const toolCatalogQuery = ref('')

async function loadChatToolCatalog() {
  if (toolCatalogLoading.value) return
  toolCatalogLoading.value = true
  toolCatalogError.value = ''
  try {
    const response = await apiJson<ToolCatalogResponse>('/api/v1/plugins/surface')
    toolCatalogItems.value = Array.isArray(response?.permission_tools) ? response.permission_tools : []
    const activityKinds = normalizeChatActivityKindCatalog(response?.activity_kinds)
    activityKindCatalogItems.value = activityKinds.length ? activityKinds : BUILTIN_CHAT_ACTIVITY_KINDS.slice()
  } catch (error) {
    toolCatalogError.value = error instanceof Error ? error.message : String(error)
  } finally {
    toolCatalogLoading.value = false
  }
}

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

const toolActivityOptions = computed<ToolExpansionOption[]>(() => {
  const byId = new Map<string, ToolExpansionOption>()
  for (const option of toolApiFunctions.value) byId.set(option.id, option)
  for (const item of toolCatalogItems.value) {
    const label = String(item.name || '').trim()
    const id = normalizeChatToolPreferenceId(label)
    if (!id) continue
    const summary = String(item.summary || '').trim()
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

const chatToolActivityDefaultExpandedOverrides = computed<ChatToolExpansionOverrides>({
  get() {
    return normalizeChatToolExpansionOverrides(settings.data?.chatToolActivityDefaultExpandedOverrides)
  },
  set(value) {
    void settings.save({ chatToolActivityDefaultExpandedOverrides: value })
  },
})

const legacyExpandedToolCategories = computed(() => new Set<string>(chatActivityDefaultExpandedToolFilters.value))

function toolDefaultExpandedEnabled(toolId: string): boolean {
  return resolveChatToolDefaultExpanded(
    toolId,
    chatToolActivityDefaultExpandedOverrides.value,
    legacyExpandedToolCategories.value,
    activityKindDefaultExpandedEnabled('operation'),
  )
}

function toolDefaultExpandedCustomized(toolId: string): boolean {
  const id = normalizeChatToolPreferenceId(toolId)
  return Boolean(id && Object.prototype.hasOwnProperty.call(chatToolActivityDefaultExpandedOverrides.value, id))
}

function toggleToolDefaultExpanded(toolId: string) {
  const id = normalizeChatToolPreferenceId(toolId)
  if (!id) return
  chatToolActivityDefaultExpandedOverrides.value = {
    ...chatToolActivityDefaultExpandedOverrides.value,
    [id]: !toolDefaultExpandedEnabled(id),
  }
}

function resetToolDefaultExpanded(toolId: string) {
  const id = normalizeChatToolPreferenceId(toolId)
  if (!id || !toolDefaultExpandedCustomized(id)) return
  const next = { ...chatToolActivityDefaultExpandedOverrides.value }
  delete next[id]
  chatToolActivityDefaultExpandedOverrides.value = next
}

const dirtyHint = computed(() => (settings.error ? settings.error : null))
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
          />
          <SettingsSidebar
            :tabs="tabs"
            :active-tab="activeSection"
            :active-view="String(route.query.view || '')"
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
        <div class="mx-auto w-full max-w-6xl space-y-8 p-4 lg:p-8">
          <div
            v-if="dirtyHint"
            class="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-destructive"
          >
            {{ dirtyHint }}
          </div>

          <!-- Interface: TUI, Web appearance, and conversation display are independent subpages. -->
          <SettingsSectionWorkbench
            v-if="activeSection === 'interface'"
            :key="`interface-${settingsRefreshNonce}`"
            section="interface"
            :title="String(t('settings.tabs.interface'))"
            :description="
              $st(
                'Configure server-backed TUI behavior and browser-only Web appearance without mixing unrelated settings into one page.',
              )
            "
            :pages="interfacePages"
            :default-page="SETTINGS_DEFAULT_SUBPAGE.interface"
            v-slot="{ activePage }"
          >
            <InterfaceSettingsPanel v-if="activePage === 'tui'" />
            <div v-else class="space-y-6">
              <div class="text-lg font-medium">
                {{
                  activePage === 'web-appearance' ? t('settings.appearance.intro') : t('settings.appearance.chat.label')
                }}
              </div>

              <div class="grid gap-6">
                <div v-if="activePage === 'web-appearance'" class="grid gap-2">
                  <label class="text-sm font-medium leading-none">{{ t('settings.appearance.language.label') }}</label>
                  <div class="text-xs text-muted-foreground">{{ t('settings.appearance.language.help') }}</div>
                  <div class="w-56 max-w-full">
                    <OptionPicker
                      v-model="uiLocale"
                      :options="localePickerOptions"
                      :title="String(t('settings.appearance.language.label'))"
                      :search-placeholder="String(t('settings.appearance.language.label'))"
                      :include-empty="false"
                    />
                  </div>
                </div>

                <div v-if="activePage === 'web-appearance'" class="grid gap-2">
                  <label class="text-sm font-medium leading-none">{{ t('settings.appearance.theme.label') }}</label>
                  <div class="flex items-center gap-3 flex-wrap">
                    <label class="inline-flex items-center gap-2 text-sm">
                      <input type="checkbox" v-model="useSystemTheme" />
                      {{ t('settings.appearance.theme.useSystem') }}
                    </label>
                    <div class="w-28 min-w-[7rem]">
                      <OptionPicker
                        v-model="themeVariant"
                        :options="themeVariantPickerOptions"
                        :title="String(t('settings.appearance.theme.pickerTitle'))"
                        :search-placeholder="String(t('settings.appearance.theme.pickerSearch'))"
                        :include-empty="false"
                        :disabled="useSystemTheme"
                      />
                    </div>
                  </div>
                </div>

                <div v-if="activePage === 'web-appearance'" class="grid gap-2">
                  <label class="text-sm font-medium leading-none">{{ t('settings.appearance.fonts.label') }}</label>
                  <div class="grid grid-cols-1 lg:grid-cols-2 gap-3">
                    <div class="grid gap-1">
                      <div class="text-xs text-muted-foreground">{{ t('settings.appearance.fonts.ui') }}</div>
                      <OptionPicker
                        v-model="uiFont"
                        :options="uiFontPickerOptions"
                        :title="String(t('settings.appearance.fonts.ui'))"
                        :search-placeholder="String(t('settings.appearance.fonts.search'))"
                        :include-empty="false"
                      />
                    </div>
                    <div class="grid gap-1">
                      <div class="text-xs text-muted-foreground">{{ t('settings.appearance.fonts.mono') }}</div>
                      <OptionPicker
                        v-model="monoFont"
                        :options="monoFontPickerOptions"
                        :title="String(t('settings.appearance.fonts.mono'))"
                        :search-placeholder="String(t('settings.appearance.fonts.search'))"
                        :include-empty="false"
                      />
                    </div>
                  </div>
                </div>

                <div v-if="activePage === 'web-appearance'" class="grid gap-2">
                  <label class="text-sm font-medium leading-none">{{ t('settings.appearance.sizing.label') }}</label>
                  <div class="grid grid-cols-1 lg:grid-cols-2 gap-3">
                    <label class="grid gap-1">
                      <span class="text-xs text-muted-foreground">{{ t('settings.appearance.sizing.fontSize') }}</span>
                      <input
                        type="number"
                        min="70"
                        max="140"
                        v-model.number="fontSize"
                        class="h-9 rounded-md border border-input bg-transparent px-3 text-sm"
                      />
                    </label>
                    <label class="grid gap-1">
                      <span class="text-xs text-muted-foreground">{{ t('settings.appearance.sizing.padding') }}</span>
                      <input
                        type="number"
                        min="70"
                        max="140"
                        v-model.number="padding"
                        class="h-9 rounded-md border border-input bg-transparent px-3 text-sm"
                      />
                    </label>
                    <label class="grid gap-1">
                      <span class="text-xs text-muted-foreground">{{
                        t('settings.appearance.sizing.cornerRadius')
                      }}</span>
                      <input
                        type="number"
                        min="0"
                        max="28"
                        v-model.number="cornerRadius"
                        class="h-9 rounded-md border border-input bg-transparent px-3 text-sm"
                      />
                    </label>
                    <label class="grid gap-1">
                      <span class="text-xs text-muted-foreground">{{
                        t('settings.appearance.sizing.inputBarOffset')
                      }}</span>
                      <input
                        type="number"
                        min="-40"
                        max="80"
                        v-model.number="inputBarOffset"
                        class="h-9 rounded-md border border-input bg-transparent px-3 text-sm"
                      />
                    </label>
                  </div>
                </div>

                <div v-if="activePage === 'conversation'" class="grid gap-2">
                  <label class="text-sm font-medium leading-none">{{ t('settings.appearance.chat.label') }}</label>
                  <div class="grid gap-3">
                    <label class="inline-flex items-center gap-2 text-sm">
                      <input type="checkbox" v-model="showChatTimestamps" />
                      {{ t('settings.appearance.chat.showTimestamps') }}
                    </label>
                    <label class="inline-flex items-center gap-2 text-sm">
                      <input type="checkbox" v-model="showReasoningTraces" />
                      {{ t('settings.appearance.chat.showReasoning') }}
                    </label>
                    <label class="inline-flex items-center gap-2 text-sm">
                      <input type="checkbox" v-model="chatActivityAutoCollapseOnIdle" />
                      {{ t('settings.appearance.chat.autoCollapseActivity') }}
                    </label>
                    <div class="mt-1">
                      <div class="text-xs font-medium text-muted-foreground">
                        {{ t('settings.appearance.chat.activityDetails') }}
                      </div>
                      <div class="mt-2 overflow-x-auto rounded-md border border-border/60">
                        <table class="min-w-full text-sm">
                          <thead class="bg-muted/30 text-xs text-muted-foreground">
                            <tr>
                              <th class="px-3 py-2 text-left font-medium">
                                {{ t('settings.appearance.chat.activityTable.type') }}
                              </th>
                              <th class="px-3 py-2 text-center font-medium">
                                {{ t('settings.appearance.chat.activityTable.expand') }}
                              </th>
                            </tr>
                          </thead>
                          <tbody>
                            <tr
                              v-for="opt in activityKindOptions"
                              :key="`activity-matrix-${opt.id}`"
                              class="border-t border-border/50"
                            >
                              <td class="px-3 py-2 align-top">
                                <div class="flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-0.5">
                                  <span>{{ activityKindLabel(opt) }}</span>
                                  <code class="font-mono text-[10px] text-muted-foreground">{{ opt.id }}</code>
                                </div>
                                <div class="text-[11px] text-muted-foreground">{{ activityKindDescription(opt) }}</div>
                              </td>
                              <td class="px-3 py-2 text-center align-middle">
                                <input
                                  type="checkbox"
                                  :checked="activityKindDefaultExpandedEnabled(opt.id)"
                                  @change="toggleActivityKindDefaultExpanded(opt.id)"
                                />
                              </td>
                            </tr>
                          </tbody>
                        </table>
                      </div>
                    </div>

                    <div class="mt-1">
                      <div class="flex flex-wrap items-center justify-between gap-2">
                        <div>
                          <div class="text-xs font-medium text-muted-foreground">
                            {{ t('settings.appearance.chat.toolDetails') }}
                          </div>
                          <div class="mt-0.5 text-[11px] text-muted-foreground">
                            {{
                              t('settings.appearance.chat.toolDetailsCount', {
                                shown: filteredToolActivityOptions.length,
                                total: toolActivityOptions.length,
                              })
                            }}
                          </div>
                        </div>
                        <div class="flex min-w-0 items-center gap-2">
                          <input
                            v-model="toolCatalogQuery"
                            type="search"
                            class="h-8 min-w-0 w-56 max-w-[55vw] rounded-md border border-input bg-transparent px-2.5 text-xs outline-none focus:border-ring"
                            :placeholder="t('settings.appearance.chat.searchTools')"
                          />
                          <button
                            type="button"
                            class="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-md border border-input text-muted-foreground hover:bg-muted/50 hover:text-foreground disabled:opacity-50"
                            :title="t('settings.appearance.chat.refreshTools')"
                            :aria-label="t('settings.appearance.chat.refreshTools')"
                            :disabled="toolCatalogLoading"
                            @click="loadChatToolCatalog"
                          >
                            <RiRefreshLine class="h-4 w-4" :class="toolCatalogLoading ? 'animate-spin' : ''" />
                          </button>
                        </div>
                      </div>
                      <div class="mt-2 overflow-x-auto rounded-md border border-border/60">
                        <table class="min-w-full text-sm">
                          <thead class="bg-muted/30 text-xs text-muted-foreground">
                            <tr>
                              <th class="px-3 py-2 text-left font-medium">
                                {{ t('settings.appearance.chat.toolDetailsTable.tool') }}
                              </th>
                              <th class="px-3 py-2 text-center font-medium">
                                {{ t('settings.appearance.chat.toolDetailsTable.expand') }}
                              </th>
                              <th class="w-10 px-2 py-2" :aria-label="$st('Reset')"></th>
                            </tr>
                          </thead>
                          <tbody>
                            <tr v-if="toolCatalogLoading && toolActivityOptions.length === toolApiFunctions.length">
                              <td
                                colspan="3"
                                class="border-t border-border/50 px-3 py-6 text-center text-xs text-muted-foreground"
                              >
                                {{ t('settings.appearance.chat.loadingTools') }}
                              </td>
                            </tr>
                            <tr v-else-if="toolCatalogError && toolActivityOptions.length === toolApiFunctions.length">
                              <td
                                colspan="3"
                                class="border-t border-border/50 px-3 py-4 text-xs text-rose-700 dark:text-rose-300"
                              >
                                {{ toolCatalogError }}
                              </td>
                            </tr>
                            <tr v-else-if="filteredToolActivityOptions.length === 0">
                              <td
                                colspan="3"
                                class="border-t border-border/50 px-3 py-6 text-center text-xs text-muted-foreground"
                              >
                                {{ t('settings.appearance.chat.noTools') }}
                              </td>
                            </tr>
                            <tr
                              v-for="opt in filteredToolActivityOptions"
                              :key="`tool-matrix-${opt.id}`"
                              class="border-t border-border/50"
                            >
                              <td class="px-3 py-2 align-top">
                                <div class="font-mono text-xs">{{ opt.label }}</div>
                                <div class="text-[11px] text-muted-foreground">{{ opt.description }}</div>
                              </td>
                              <td class="px-3 py-2 text-center align-middle">
                                <input
                                  type="checkbox"
                                  :checked="toolDefaultExpandedEnabled(opt.id)"
                                  @change="toggleToolDefaultExpanded(opt.id)"
                                />
                              </td>
                              <td class="px-2 py-2 text-center align-middle">
                                <button
                                  type="button"
                                  class="inline-flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground hover:bg-muted/50 hover:text-foreground disabled:invisible"
                                  :title="t('settings.appearance.chat.resetToolDefault')"
                                  :aria-label="t('settings.appearance.chat.resetToolDefault')"
                                  :disabled="!toolDefaultExpandedCustomized(opt.id)"
                                  @click="resetToolDefaultExpanded(opt.id)"
                                >
                                  <RiResetLeftLine class="h-3.5 w-3.5" />
                                </button>
                              </td>
                            </tr>
                          </tbody>
                        </table>
                      </div>
                      <div class="mt-2 text-[11px] text-muted-foreground">
                        {{ t('settings.appearance.chat.toolDetailsHint') }}
                      </div>
                    </div>
                  </div>
                </div>
              </div>
            </div>
          </SettingsSectionWorkbench>

          <!-- Models & Providers -->
          <ModelsProvidersPanel
            v-else-if="activeSection === 'models-providers'"
            :key="`models-providers-${settingsRefreshNonce}`"
          />

          <!-- Permissions -->
          <PermissionsWorkbenchPanel
            v-else-if="activeSection === 'permissions'"
            :key="`permissions-${settingsRefreshNonce}`"
          />

          <!-- Plugins & Tools -->
          <PluginsToolsPanel
            v-else-if="activeSection === 'plugins-tools'"
            :key="`plugins-tools-${settingsRefreshNonce}`"
          />

          <!-- Runtime & Session -->
          <RuntimeSessionPanel
            v-else-if="activeSection === 'runtime-session'"
            :key="`runtime-session-${settingsRefreshNonce}`"
          />

          <DiagnosticsWorkbenchPanel
            v-else-if="activeSection === 'diagnostics'"
            :key="`diagnostics-${settingsRefreshNonce}`"
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
.settings-page :deep(input[type='checkbox']),
.settings-page :deep(input[type='radio']) {
  accent-color: oklch(var(--muted-foreground));
}
</style>
