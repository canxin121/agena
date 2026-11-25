<script setup lang="ts">
import { computed, defineAsyncComponent, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import {
  RiRefreshLine,
  RiResetLeftLine,
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
import {
  AGENT_PROFILE_DEFAULTS_HASH,
  PROMPT_LIBRARY_HASH,
  isAgentProfileDefaultsHash,
  movedSettingsPage,
  settingsQueryForDestination,
} from '@/components/settings/settingsDestination'
import { pluginTabForSettingsView } from '@/components/settings/pluginSettingsNavigation'
import { legacyPermissionAccessScope, permissionAccessHash } from '@/components/settings/permissions/permissionNavigation'
import { settingsSessionId, useSettingsSessionTarget } from '@/components/settings/settingsSessionScope'
import SettingsSectionWorkbench from '@/components/settings/workbench/SettingsSectionWorkbench.vue'
import {
  normalizeSettingsSubpageId,
  resolveSettingsSubpage,
  settingsSubpageStorageKey,
} from '@/components/settings/workbench/settingsSectionNavigation'
import { SETTINGS_DEFAULT_SUBPAGE, buildSettingsSubpages } from '@/components/settings/settingsNavigationCatalog'
import { buildSettingsNavigationCategories } from '@/components/settings/settingsInformationArchitecture'
import {
  buildSettingsScopes,
  findSettingsScope,
  resolveSettingsScope,
  settingsScopeStorageKey,
} from '@/components/settings/settingsScopeCatalog'
import {
  normalizeRememberedSettingsRoute,
  settingsPathForTab,
  settingsTabFromRouteValue,
  type SettingsSidebarDestination,
  type SettingsTab,
} from '@/components/settings/sidebar/settingsSidebarNavigation'
import WorkspacePageSidebar from '@/layout/WorkspacePageSidebar.vue'
import { localStorageKeys } from '@/lib/persistence/storageKeys'
import { apiJson } from '@/lib/api'
import { buildLocalePickerOptions } from '@/pages/loginLocaleOptions'
import { useWorkspacePaneContext } from '@/app/workspace/workspacePaneContext'
import { usePaneVisibility } from '@/composables/usePaneVisibility'
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
const SkillsPanel = defineAsyncComponent(() => import('@/components/settings/SkillsPanel.vue'))
const McpConnectionsPanel = defineAsyncComponent(() => import('@/components/settings/McpConnectionsPanel.vue'))
const McpServerControlPanel = defineAsyncComponent(() => import('@/components/settings/McpServerControlPanel.vue'))
const RuntimeSessionPanel = defineAsyncComponent(() => import('@/components/settings/RuntimeSessionPanel.vue'))
const SessionStoragePanel = defineAsyncComponent(() => import('@/components/settings/SessionStoragePanel.vue'))
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

const requestedSection = computed(readInitialSection)
const requestedView = computed(() => normalizeSettingsSubpageId(route.query.view))
const movedPage = computed(() => movedSettingsPage(requestedSection.value, requestedView.value, route.hash))
const activeSection = computed(() => movedPage.value?.section ?? requestedSection.value)
const activePages = computed(() => buildSettingsSubpages(activeSection.value))
const activeView = computed(() => {
  if (movedPage.value) return movedPage.value.view
  let remembered = ''
  try {
    remembered = localStorage.getItem(settingsSubpageStorageKey(activeSection.value)) || ''
  } catch {}
  return resolveSettingsSubpage(
    route.query.view,
    remembered,
    activePages.value,
    SETTINGS_DEFAULT_SUBPAGE[activeSection.value],
  )
})

// Every scoped page resolves the same configuration layers; its workbench owns
// the visible selector while the route keeps the choice shareable.
const activeScopes = computed(() => buildSettingsScopes(activeSection.value, activeView.value))
// Runtime settings pages with distinct scope choices remember those choices independently.
const scopeStorageKey = computed(() =>
  settingsScopeStorageKey(
    ['session-behavior', 'prompt-library'].includes(activeView.value)
      ? `${activeSection.value}.${activeView.value}`
      : activeSection.value,
  ),
)
const activeScope = computed(() => {
  if (activeSection.value === 'permissions') {
    const legacyScope = legacyPermissionAccessScope(requestedView.value, route.query.scope)
    if (legacyScope) return legacyScope
  }
  if (
    activeView.value === 'session-behavior' &&
    (requestedView.value === 'compaction' ||
      (requestedView.value === 'agent-profiles' && isAgentProfileDefaultsHash(route.hash)))
  ) {
    return route.query.scope === 'workspace' ? 'workspace' : 'global'
  }
  let remembered = ''
  try {
    remembered = localStorage.getItem(scopeStorageKey.value) || ''
  } catch {}
  return resolveSettingsScope(route.query.scope, remembered, activeScopes.value)
})

// Session-scoped pages share this settings window's explicit target, falling
// back to the last used conversation until the user chooses one.
const sessionTarget = useSettingsSessionTarget()

// The active subpage declares whether it owns the whole content area (dense
// master/detail pages such as the Model Catalog do) or scrolls inside it like
// every other settings page.
const activeSubpageDefinition = computed(() => activePages.value.find((page) => page.id === activeView.value) || null)
const contentFillsViewport = computed(() => activeSubpageDefinition.value?.fillsViewport === true)
const settingsPane = ref<HTMLElement | null>(null)

watch([activeSection, activeView], () => {
  if (!route.hash) settingsPane.value?.scrollTo({ top: 0 })
}, { flush: 'post' })

function legacyViewAnchor(): string {
  if (activeSection.value === 'permissions') return permissionAccessHash(requestedView.value)
  if (activeSection.value === 'interface' && ['conversation', 'notifications'].includes(requestedView.value))
    return '#settings-' + requestedView.value
  if (activeView.value === 'session-behavior') {
    if (requestedView.value === 'compaction') return '#settings-context-management'
    if (requestedView.value === 'approval-model' && activeScope.value !== 'session') return '#settings-approval-model'
  }
  return ''
}

watch(
  () =>
    [
      route.params.section,
      route.query.view,
      route.query.scope,
      route.query.pluginTab,
      route.hash,
      activeView.value,
      activeScope.value,
    ] as const,
  () => {
    if (!settingsTabFromRouteValue(route.params.section)) return
    try {
      localStorage.setItem(settingsSubpageStorageKey(activeSection.value), activeView.value)
      localStorage.setItem(scopeStorageKey.value, activeScope.value)
    } catch {}
    const { pluginTab: _pluginTab, scope: _scope, sessionId: _sessionId, ...query } = route.query
    if (activeScope.value) query.scope = activeScope.value
    const isPluginView = activeSection.value === 'plugins-tools' && pluginTabForSettingsView(activeView.value)
    if (isPluginView) {
      const tab = pluginTabForSettingsView(route.query.view || activeView.value, route.query.pluginTab)
      if (tab && tab !== 'overview') query.pluginTab = tab
    } else delete query.plugin
    const hash =
      activeView.value === 'prompt-library' && route.hash === PROMPT_LIBRARY_HASH
        ? ''
        : activeView.value === 'session-behavior' && isAgentProfileDefaultsHash(route.hash)
          ? AGENT_PROFILE_DEFAULTS_HASH
          : route.hash || legacyViewAnchor()
    if (
      requestedSection.value === activeSection.value &&
      query.view === activeView.value &&
      route.query.scope === query.scope &&
      route.query.pluginTab === query.pluginTab &&
      route.query.sessionId === undefined &&
      hash === route.hash &&
      (isPluginView || route.query.plugin === undefined)
    )
      return
    void router.replace({
      path: settingsPathForTab(activeSection.value),
      query: { ...query, view: activeView.value },
      hash,
    })
  },
  { immediate: true },
)
const runtimeSessionPanel = ref<{ refresh: () => Promise<void> } | null>(null)
const diagnosticsPanel = ref<{ refresh: () => Promise<void> } | null>(null)
const interfacePanel = ref<{ refresh: () => Promise<void> } | null>(null)
const pluginsToolsPanel = ref<{ refresh: () => Promise<void> } | null>(null)
const skillsPanel = ref<{ refresh: () => Promise<void> } | null>(null)
const mcpConnectionsPanel = ref<{ refresh: () => Promise<void> } | null>(null)
const mcpServerPanel = ref<{ refresh: () => Promise<void> } | null>(null)
const modelsProvidersPanel = ref<{ refresh: () => Promise<void> } | null>(null)
const permissionsPanel = ref<{ refresh: () => Promise<void> } | null>(null)

function goToSettingsDestination(destination: SettingsSidebarDestination) {
  const path = settingsPathForTab(destination.section)
  const query = settingsQueryForDestination(destination, activeSection.value, route.query)
  void router.push({ path, query })
  if (ui.isCompactLayout) ui.setSessionSwitcherOpen(false)
}

const tabs = computed(() => buildSettingsNavigationCategories())

async function refreshSettingsSidebar() {
  // Configuration editors refresh in place so unsaved drafts survive.
  if (activeSection.value === 'plugins-tools') {
    await pluginsToolsPanel.value?.refresh()
  } else if (activeSection.value === 'skills') {
    await skillsPanel.value?.refresh()
  } else if (activeSection.value === 'mcp-connections') {
    await mcpConnectionsPanel.value?.refresh()
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
    .filter((part, index, parts) => index === 0 || part !== parts[index - 1])
    .join(' · ')
}

const settingsWindowTitle = computed(() => {
  const base = String(t('settings.title'))
  const activeTabLabel =
    tabs.value.find((tab) =>
      tab.children?.some(
        (task) =>
          task.section === activeSection.value &&
          (task.view === activeView.value || task.relatedViews?.includes(activeView.value)),
      ),
    )?.label || base
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
const needsChatToolCatalog = computed(() => activeSection.value === 'interface' && activeView.value === 'web-appearance')

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
      <WorkspacePageSidebar panel="settings" :mode="workspacePane ? 'page' : 'local'" local-resizable>
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
      </WorkspacePageSidebar>

      <!-- Content -->
      <main
        ref="settingsPane"
        class="settings-pane flex-1 min-w-0 overflow-y-auto bg-background"
        v-show="!ui.isCompactLayout || !ui.isSessionSwitcherOpen"
      >
        <div
          class="w-full space-y-3 p-3 sm:p-4"
          :class="{ 'settings-pane-fill': contentFillsViewport }"
        >
          <div
            v-if="dirtyHint"
            class="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-destructive"
          >
            {{ dirtyHint }}
          </div>

          <!-- Browser preferences and server-backed TUI preferences each have their own scope. -->
          <SettingsSectionWorkbench
            v-if="activeSection === 'interface'"
            section="interface"
            :pages="interfacePages"
            :active-page="activeView"
            :default-page="SETTINGS_DEFAULT_SUBPAGE.interface"
            v-slot="{ activePage }"
          >
            <InterfaceSettingsPanel v-if="activePage === 'tui'" ref="interfacePanel" />
            <div v-else class="settings-columns">
              <div class="settings-stack">
                <SettingsCard :title="t('settings.appearance.theme.label')" :icon="RiPaletteLine">
                  <SettingsRow
                    :title="t('settings.appearance.language.label')"
                    :description="t('settings.appearance.language.help')"
                  >
                    <OptionPicker
                      v-model="uiLocale"
                      :options="localePickerOptions"
                      :title="t('settings.appearance.language.label')"
                      :search-placeholder="t('settings.appearance.language.label')"
                      :include-empty="false"
                      size="sm"
                    />
                  </SettingsRow>
                  <SettingsRow :title="t('settings.appearance.theme.useSystem')" toggle>
                    <SettingsToggle v-model="useSystemTheme" :label="t('settings.appearance.theme.useSystem')" />
                  </SettingsRow>
                  <SettingsRow :title="t('settings.appearance.theme.pickerTitle')">
                    <OptionPicker
                      v-model="themeVariant"
                      :disabled="useSystemTheme"
                      :options="themeVariantPickerOptions"
                      :title="t('settings.appearance.theme.pickerTitle')"
                      :search-placeholder="t('settings.appearance.theme.pickerSearch')"
                      :include-empty="false"
                      size="sm"
                    />
                  </SettingsRow>
                </SettingsCard>
                <SettingsCard :title="t('settings.appearance.fonts.label')" :icon="RiFontSize">
                  <div class="settings-control-grid">
                    <div class="min-w-0 space-y-1">
                      <div class="text-xs font-medium text-muted-foreground">{{ t('settings.appearance.fonts.ui') }}</div>
                      <OptionPicker
                        v-model="uiFont"
                        :options="uiFontPickerOptions"
                        :title="t('settings.appearance.fonts.ui')"
                        :search-placeholder="t('settings.appearance.fonts.search')"
                        :include-empty="false"
                      />
                    </div>
                    <div class="min-w-0 space-y-1">
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
                  <div class="settings-control-grid">
                    <label class="grid min-w-0 gap-1 text-xs font-medium text-muted-foreground">
                      {{ t('settings.appearance.sizing.fontSize') }}
                      <input v-model.number="fontSize" type="number" min="70" max="140" class="settings-number-input" />
                    </label>
                    <label class="grid min-w-0 gap-1 text-xs font-medium text-muted-foreground">
                      {{ t('settings.appearance.sizing.padding') }}
                      <input v-model.number="padding" type="number" min="70" max="140" class="settings-number-input" />
                    </label>
                    <label class="grid min-w-0 gap-1 text-xs font-medium text-muted-foreground">
                      {{ t('settings.appearance.sizing.cornerRadius') }}
                      <input v-model.number="cornerRadius" type="number" min="0" max="28" class="settings-number-input" />
                    </label>
                    <label class="grid min-w-0 gap-1 text-xs font-medium text-muted-foreground">
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
              </div>
              <div class="settings-stack">
                <SettingsCard id="settings-conversation" :title="t('settings.appearance.chat.label')" :icon="RiChat3Line">
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
                <BrowserNotificationsPanel id="settings-notifications" />
                <SettingsCard :title="t('settings.appearance.chat.activityDetails')" :icon="RiListCheck2">
                  <SettingsRow :title="$st('Expand activity parts by default unless a type has its own setting')" toggle>
                    <SettingsToggle
                      v-model="transcriptActivityDefaultExpanded"
                      :label="$st('Expand activity parts by default unless a type has its own setting')"
                    />
                  </SettingsRow>
                  <div class="mt-2 divide-y divide-border/50">
                    <div
                      v-for="opt in activityKindOptions"
                      :key="opt.id"
                      class="flex min-w-0 items-center gap-2 py-1.5"
                    >
                      <div class="min-w-0 flex-1">
                        <div class="text-sm font-medium">{{ activityKindLabel(opt) }}</div>
                        <p class="mt-0.5 text-xs leading-snug text-muted-foreground">
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
              </div>
              <SettingsCard
                :title="t('settings.appearance.chat.toolDetails')"
                class="settings-columns-wide"
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
                <div v-else-if="filteredToolActivityOptions.length" class="max-h-96 divide-y divide-border/50 overflow-y-auto">
                  <div
                    v-for="opt in filteredToolActivityOptions"
                    :key="opt.id"
                    class="flex min-w-0 items-center gap-2 py-1.5 first:pt-0"
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
            v-else-if="activeSection === 'models-providers'"
          />

          <!-- Permissions -->
          <PermissionsWorkbenchPanel
            ref="permissionsPanel"
            :active-page="activeView"
            :scope="activeScope"
            :session-id="sessionTarget.sessionId.value"
            v-else-if="activeSection === 'permissions'"
          />

          <!-- Plugins & Tools -->
          <PluginsToolsPanel
            ref="pluginsToolsPanel"
            :active-page="activeView"
            v-else-if="activeSection === 'plugins-tools'"
          />

          <!-- Skills -->
          <SkillsPanel v-else-if="activeSection === 'skills'" ref="skillsPanel" :active-page="activeView" />
          <McpConnectionsPanel
            v-else-if="activeSection === 'mcp-connections'"
            ref="mcpConnectionsPanel"
            :active-page="activeView"
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
          <SessionStoragePanel
            v-else-if="activeSection === 'runtime-session' && activeView === 'storage'"
            :target-session-id="settingsSessionId(route.query.targetSessionId)"
            :target-bot-id="typeof route.query.storageBot === 'string' ? route.query.storageBot : undefined"
            @select="sessionTarget.selectSession"
          />
          <RuntimeSessionPanel
            ref="runtimeSessionPanel"
            :active-page="activeView"
            :scope="activeScope"
            :session-id="sessionTarget.sessionId.value"
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
.settings-pane {
  container: settings-pane / inline-size;
}

@container settings-pane (min-width: 44rem) {
  .settings-pane-fill {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
}

.settings-number-input {
  @apply h-9 w-full min-w-0 rounded-md border border-input bg-background px-3 text-sm font-normal text-foreground outline-none focus:border-ring;
}

.settings-page :deep(input[type='checkbox']),
.settings-page :deep(input[type='radio']) {
  accent-color: oklch(var(--muted-foreground));
}
</style>
