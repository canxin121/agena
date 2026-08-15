<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useRoute } from 'vue-router'
import { useI18n } from 'vue-i18n'

import { useSettingsStore, type Settings } from '../stores/settings'
import { useUiStore } from '@/stores/ui'
import { i18n, setAppLocale } from '@/i18n'
import type { AppLocale } from '@/i18n/locale'

import OptionPicker from '@/components/ui/OptionPicker.vue'
import SettingsSidebar from '@/components/settings/sidebar/SettingsSidebar.vue'
import ProvidersPanel from '@/components/settings/ProvidersPanel.vue'
import PermissionsPanel from '@/components/settings/PermissionsPanel.vue'
import ActivitiesPanel from '@/components/settings/ActivitiesPanel.vue'
import MemoriesPanel from '@/components/settings/MemoriesPanel.vue'
import UsagePanel from '@/components/settings/UsagePanel.vue'
import {
  buildSettingsSidebarTabs,
  isSettingsTab,
  type SettingsTab,
} from '@/components/settings/sidebar/settingsSidebarNavigation'
import { useDesktopSidebarResize } from '@/composables/useDesktopSidebarResize'
import { localStorageKeys } from '@/lib/persistence/storageKeys'
import { buildLocalePickerOptions } from '@/pages/loginLocaleOptions'
import {
  CHAT_ACTIVITY_EXPAND_KEYS,
  DEFAULT_CHAT_ACTIVITY_EXPAND_KEYS,
  DEFAULT_CHAT_TOOL_ACTIVITY_FILTERS,
  DEFAULT_CHAT_ACTIVITY_SUMMARY_FILTERS,
  DEFAULT_CHAT_TOOL_ACTIVITY_SUMMARY_FILTERS,
  DEFAULT_CHAT_ACTIVITY_EXPANDED_TOOL_FILTERS,
  normalizeChatActivityFilters,
  normalizeChatActivityDefaultExpanded,
  normalizeChatToolActivityFilters,
  ACTIVITY_DEFAULT_EXPANDED_OPTIONS as activityDefaultExpandedOptions,
  TOOL_ACTIVITY_OPTIONS as toolActivityOptions,
  type ChatActivityType,
  type ChatActivityExpandKey,
  type ChatToolActivityType,
} from '@/lib/chatActivity'

const settings = useSettingsStore()
const ui = useUiStore()
const route = useRoute()
const { startDesktopSidebarResize } = useDesktopSidebarResize()
const { t } = useI18n()

const SETTINGS_LAST_SECTION_KEY = localStorageKeys.settings.lastRoute

const DEFAULT_SECTION: SettingsTab = 'general'

const TAB_LABELS: Record<SettingsTab, string> = {
  general: 'General',
  providers: 'Providers',
  permissions: 'Permissions',
  activities: 'Activities',
  memories: 'Memories',
  usage: 'Usage',
}

function readInitialSection(): SettingsTab {
  let raw = ''
  try {
    raw = localStorage.getItem(SETTINGS_LAST_SECTION_KEY) || ''
  } catch {
    // ignore
  }
  const normalized = String(raw || '')
    .trim()
    .toLowerCase()
  return isSettingsTab(normalized) ? normalized : DEFAULT_SECTION
}

const activeSection = ref<SettingsTab>(readInitialSection())

function goToSection(id: SettingsTab) {
  activeSection.value = id
  try {
    localStorage.setItem(SETTINGS_LAST_SECTION_KEY, id)
  } catch {
    // ignore
  }
}

const settingsSidebarClass = computed(() =>
  ui.isCompactLayout
    ? 'relative h-full w-full shrink-0 border-r border-border bg-sidebar'
    : 'relative h-full w-full shrink-0 bg-sidebar',
)

const showSidebar = computed(() => {
  if (ui.isCompactLayout) return ui.isSessionSwitcherOpen
  return true
})

const tabs = computed(() => buildSettingsSidebarTabs((id) => TAB_LABELS[id]))

async function refreshSettingsSidebar() {
  // The settings store is client-side UI prefs only; server-backed panels have
  // their own refresh buttons.
  await settings.refresh()
}

onMounted(() => {
  if (!settings.data && !settings.loading) {
    void settings.refresh()
  }
})

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
const showTextJustificationActivity = makeSetting('showTextJustificationActivity', true)

const chatActivityAutoCollapseOnIdle = makeSetting('chatActivityAutoCollapseOnIdle', true)

const chatActivityDefaultExpanded = computed<ChatActivityExpandKey[]>({
  get() {
    const s = settings.data
    if (s && Object.prototype.hasOwnProperty.call(s, 'chatActivityDefaultExpanded')) {
      return normalizeChatActivityDefaultExpanded(s.chatActivityDefaultExpanded)
    }
    return DEFAULT_CHAT_ACTIVITY_EXPAND_KEYS.slice()
  },
  set(value) {
    void settings.save({ chatActivityDefaultExpanded: value })
  },
})

function activityDefaultExpandedEnabled(id: ChatActivityExpandKey): boolean {
  return chatActivityDefaultExpanded.value.includes(id)
}

function toggleActivityDefaultExpanded(id: ChatActivityExpandKey) {
  const next = new Set(chatActivityDefaultExpanded.value)
  if (next.has(id)) next.delete(id)
  else next.add(id)
  const ordered = CHAT_ACTIVITY_EXPAND_KEYS.filter((t) => next.has(t))
  chatActivityDefaultExpanded.value = ordered
}

function activitySummaryEnabled(id: ChatActivityExpandKey): boolean {
  if (id === 'thinking') return showReasoningTraces.value
  if (id === 'justification') return showTextJustificationActivity.value
  return chatActivitySummaryFilters.value.includes(id as ChatActivityType)
}

function setActivitySummaryEnabled(id: ChatActivityExpandKey, enabled: boolean) {
  if (id === 'thinking') {
    showReasoningTraces.value = enabled
  } else if (id === 'justification') {
    showTextJustificationActivity.value = enabled
  } else {
    const next = new Set(chatActivitySummaryFilters.value)
    if (enabled) next.add(id as ChatActivityType)
    else next.delete(id as ChatActivityType)
    const ordered = DEFAULT_CHAT_ACTIVITY_SUMMARY_FILTERS.filter((t) => next.has(t))
    chatActivitySummaryFilters.value = ordered
  }

  if (!enabled) {
    const nextExpanded = new Set(chatActivityDefaultExpanded.value)
    if (nextExpanded.delete(id)) {
      chatActivityDefaultExpanded.value = CHAT_ACTIVITY_EXPAND_KEYS.filter((t) => nextExpanded.has(t))
    }
  }
}

function toggleActivitySummary(id: ChatActivityExpandKey) {
  setActivitySummaryEnabled(id, !activitySummaryEnabled(id))
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

function activityDefaultExpandedToolEnabled(id: ChatToolActivityType): boolean {
  return chatActivityDefaultExpandedToolFilters.value.includes(id)
}

function toggleActivityDefaultExpandedTool(id: ChatToolActivityType) {
  const next = new Set(chatActivityDefaultExpandedToolFilters.value)
  if (next.has(id)) next.delete(id)
  else next.add(id)
  const ordered = DEFAULT_CHAT_TOOL_ACTIVITY_FILTERS.filter((t) => next.has(t))
  chatActivityDefaultExpandedToolFilters.value = ordered
}

function toggleToolDetailSummary(id: ChatToolActivityType) {
  const next = new Set(chatToolActivitySummaryFilters.value)
  if (next.has(id)) {
    next.delete(id)
  } else {
    next.add(id)
  }
  const ordered = DEFAULT_CHAT_TOOL_ACTIVITY_SUMMARY_FILTERS.filter((t) => next.has(t))
  chatToolActivitySummaryFilters.value = ordered

  if (!next.has(id)) {
    const expanded = new Set(chatActivityDefaultExpandedToolFilters.value)
    if (expanded.delete(id)) {
      chatActivityDefaultExpandedToolFilters.value = DEFAULT_CHAT_TOOL_ACTIVITY_FILTERS.filter((t) => expanded.has(t))
    }
  }
}

const chatActivitySummaryFilters = computed<ChatActivityType[]>({
  get() {
    const s = settings.data
    if (s && Object.prototype.hasOwnProperty.call(s, 'chatActivitySummaryFilters')) {
      return normalizeChatActivityFilters(s.chatActivitySummaryFilters)
    }
    if (s && Object.prototype.hasOwnProperty.call(s, 'chatActivityFilters')) {
      return normalizeChatActivityFilters(s.chatActivityFilters)
    }
    return DEFAULT_CHAT_ACTIVITY_SUMMARY_FILTERS.slice()
  },
  set(value) {
    void settings.save({ chatActivitySummaryFilters: value })
  },
})

const chatToolActivitySummaryFilters = computed<ChatToolActivityType[]>({
  get() {
    const s = settings.data
    if (s && Object.prototype.hasOwnProperty.call(s, 'chatToolActivitySummaryFilters')) {
      return normalizeChatToolActivityFilters(s.chatToolActivitySummaryFilters)
    }
    if (s && Object.prototype.hasOwnProperty.call(s, 'chatActivityToolFilters')) {
      return normalizeChatToolActivityFilters(s.chatActivityToolFilters)
    }
    return DEFAULT_CHAT_TOOL_ACTIVITY_SUMMARY_FILTERS.slice()
  },
  set(value) {
    void settings.save({ chatToolActivitySummaryFilters: value })
  },
})

function toolActivityEnabled(id: ChatToolActivityType): boolean {
  return chatToolActivitySummaryFilters.value.includes(id)
}

const dirtyHint = computed(() => (settings.error ? settings.error : null))
</script>

<template>
  <div class="settings-page flex h-full flex-col overflow-hidden bg-background text-foreground">
    <div class="flex flex-1 overflow-hidden">
      <aside
        v-if="showSidebar"
        :class="settingsSidebarClass"
        :style="ui.isCompactLayout ? undefined : { width: `${ui.sidebarWidth}px` }"
      >
        <div
          v-if="!ui.isCompactLayout"
          class="absolute right-0 top-0 z-10 h-full w-1 cursor-col-resize hover:bg-primary/40"
          @pointerdown="startDesktopSidebarResize"
        />
        <SettingsSidebar
          :tabs="tabs"
          :active-tab="activeSection"
          :loading="settings.loading"
          :is-touch-pointer="ui.isTouchPointer"
          @refresh="refreshSettingsSidebar"
          @navigate-tab="goToSection"
        />
      </aside>

      <!-- Content -->
      <main
        class="flex-1 min-w-0 overflow-y-auto bg-background"
        v-show="!ui.isCompactLayout || !ui.isSessionSwitcherOpen"
      >
        <div :class="['mx-auto w-full p-4 lg:p-8 space-y-8', activeSection === 'general' ? 'max-w-3xl' : 'max-w-5xl']">
          <div
            v-if="dirtyHint"
            class="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-destructive"
          >
            {{ dirtyHint }}
          </div>

          <!-- General Tab: appearance + chat UX -->
          <div v-if="activeSection === 'general'" class="space-y-6">
            <div class="text-lg font-medium">{{ t('settings.appearance.intro') }}</div>

            <div class="grid gap-6">
              <div class="grid gap-2">
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

              <div class="grid gap-2">
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

              <div class="grid gap-2">
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

              <div class="grid gap-2">
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

              <div class="grid gap-2">
                <label class="text-sm font-medium leading-none">{{ t('settings.appearance.chat.label') }}</label>
                <div class="grid gap-3">
                  <label class="inline-flex items-center gap-2 text-sm">
                    <input type="checkbox" v-model="showChatTimestamps" />
                    {{ t('settings.appearance.chat.showTimestamps') }}
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
                              {{ t('settings.appearance.chat.activityTable.summary') }}
                            </th>
                            <th class="px-3 py-2 text-center font-medium">
                              {{ t('settings.appearance.chat.activityTable.expand') }}
                            </th>
                          </tr>
                        </thead>
                        <tbody>
                          <tr
                            v-for="opt in activityDefaultExpandedOptions"
                            :key="`activity-matrix-${opt.id}`"
                            class="border-t border-border/50"
                          >
                            <td class="px-3 py-2 align-top">
                              <div>{{ opt.label }}</div>
                              <div class="text-[11px] text-muted-foreground">{{ opt.description }}</div>
                            </td>
                            <td class="px-3 py-2 text-center align-middle">
                              <input
                                type="checkbox"
                                :checked="activitySummaryEnabled(opt.id)"
                                @change="toggleActivitySummary(opt.id)"
                              />
                            </td>
                            <td class="px-3 py-2 text-center align-middle">
                              <input
                                type="checkbox"
                                :checked="activityDefaultExpandedEnabled(opt.id)"
                                :disabled="!activitySummaryEnabled(opt.id)"
                                @change="toggleActivityDefaultExpanded(opt.id)"
                              />
                            </td>
                          </tr>
                        </tbody>
                      </table>
                    </div>
                  </div>

                  <div class="mt-1">
                    <div class="text-xs font-medium text-muted-foreground">
                      {{ t('settings.appearance.chat.toolDetails') }}
                    </div>
                    <div class="mt-2 overflow-x-auto rounded-md border border-border/60">
                      <table class="min-w-full text-sm">
                        <thead class="bg-muted/30 text-xs text-muted-foreground">
                          <tr>
                            <th class="px-3 py-2 text-left font-medium">
                              {{ t('settings.appearance.chat.toolDetailsTable.tool') }}
                            </th>
                            <th class="px-3 py-2 text-center font-medium">
                              {{ t('settings.appearance.chat.toolDetailsTable.summary') }}
                            </th>
                            <th class="px-3 py-2 text-center font-medium">
                              {{ t('settings.appearance.chat.toolDetailsTable.expand') }}
                            </th>
                          </tr>
                        </thead>
                        <tbody>
                          <tr
                            v-for="opt in toolActivityOptions"
                            :key="`tool-matrix-${opt.id}`"
                            class="border-t border-border/50"
                          >
                            <td class="px-3 py-2 align-top">
                              <div>{{ opt.label }}</div>
                              <div class="text-[11px] text-muted-foreground">{{ opt.description }}</div>
                            </td>
                            <td class="px-3 py-2 text-center align-middle">
                              <input
                                type="checkbox"
                                :checked="toolActivityEnabled(opt.id)"
                                @change="toggleToolDetailSummary(opt.id)"
                              />
                            </td>
                            <td class="px-3 py-2 text-center align-middle">
                              <input
                                type="checkbox"
                                :checked="activityDefaultExpandedToolEnabled(opt.id)"
                                :disabled="!toolActivityEnabled(opt.id)"
                                @change="toggleActivityDefaultExpandedTool(opt.id)"
                              />
                            </td>
                          </tr>
                        </tbody>
                      </table>
                    </div>
                    <div class="mt-2 text-[11px] text-muted-foreground">
                      {{ t('settings.appearance.chat.toolDetailsHint') }}
                    </div>
                  </div>

                  <div class="text-xs text-muted-foreground">
                    {{ t('settings.appearance.chat.activitySummaryHelp') }}
                  </div>
                </div>
              </div>
            </div>
          </div>

          <!-- Providers Tab -->
          <ProvidersPanel v-else-if="activeSection === 'providers'" />

          <!-- Permissions Tab -->
          <PermissionsPanel v-else-if="activeSection === 'permissions'" />

          <!-- Activities Tab -->
          <ActivitiesPanel v-else-if="activeSection === 'activities'" />

          <!-- Memories Tab -->
          <MemoriesPanel v-else-if="activeSection === 'memories'" />

          <!-- Usage Tab -->
          <UsagePanel v-else-if="activeSection === 'usage'" />

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
