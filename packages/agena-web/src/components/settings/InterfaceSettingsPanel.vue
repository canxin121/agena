<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiRefreshLine } from '@remixicon/vue'

import Button from '@/components/ui/Button.vue'
import ServerSettingField from '@/components/settings/ServerSettingField.vue'
import { apiJson } from '@/lib/api'
import { jsonPathForKey } from '@/lib/runtimeSettings'
import { normalizeChatActivityKindCatalog, type ChatActivityKindCatalogItem } from '@/lib/chatActivity'
import { setAppLocale } from '@/i18n'
import { normalizeAppLocale, SUPPORTED_LOCALES, type AppLocale } from '@/i18n/locale'

type ToolCatalogResponse = {
  catalog?: {
    tui?: {
      themes?: Array<{ id?: string; display_name?: string; plugin_id?: string }>
    }
  }
  permission_tools?: Array<{ name?: string; summary?: string; tags?: string[] }>
  activity_kinds?: unknown
}

const BUILTIN_ACTIVITY_TOOLS = [
  'tools_list',
  'tools_search',
  'tools_help',
  'tools_tags',
  'tools_call',
  'plugins_list',
  'plugins_search',
  'plugins_tags',
]

const { t } = useI18n()
const loadingCatalog = ref(false)
const catalogError = ref('')
const activityKinds = ref<ChatActivityKindCatalogItem[]>([])
const toolNames = ref<string[]>([])
const themeOptions = ref<Array<{ value: string; label: string; description?: string }>>([])

const localeOptions = computed(() =>
  SUPPORTED_LOCALES.map((value) => ({
    value,
    label: value,
    description: value === 'zh-CN' ? '简体中文' : value === 'en-US' ? 'English' : value,
  })),
)

const colorSchemeOptions = [
  { value: 'auto', label: 'Auto', description: 'Follow the terminal environment.' },
  { value: 'dark', label: 'Dark', description: 'Use the dark TUI palette.' },
  { value: 'light', label: 'Light', description: 'Use the light TUI palette.' },
]

const graphicsOptions = [
  { value: 'auto', label: 'Auto', description: 'Use native terminal graphics when available.' },
  { value: 'native', label: 'Native', description: 'Prefer native terminal graphics.' },
  { value: 'unicode', label: 'Unicode', description: 'Use portable Unicode rendering.' },
]

async function loadCatalog() {
  if (loadingCatalog.value) return
  loadingCatalog.value = true
  catalogError.value = ''
  try {
    const response = await apiJson<ToolCatalogResponse>('/api/v1/plugins/ui')
    const kinds = normalizeChatActivityKindCatalog(response?.activity_kinds)
    activityKinds.value = kinds
    themeOptions.value = (Array.isArray(response?.catalog?.tui?.themes) ? response.catalog.tui.themes : [])
      .map((theme) => {
        const id = String(theme?.id || '').trim()
        const label = String(theme?.display_name || id).trim()
        const pluginId = String(theme?.plugin_id || '').trim()
        return id ? { value: id, label: label || id, description: pluginId ? `Plugin: ${pluginId}` : undefined } : null
      })
      .filter((theme): theme is { value: string; label: string; description?: string } => Boolean(theme))
    toolNames.value = [...new Set([
      ...BUILTIN_ACTIVITY_TOOLS,
      ...(Array.isArray(response?.permission_tools) ? response.permission_tools : [])
        .map((item) => String(item?.name || '').trim())
        .filter(Boolean),
    ])].sort((a, b) => a.localeCompare(b))
  } catch (reason) {
    catalogError.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    loadingCatalog.value = false
  }
}

function settingSaved(path: string, value: unknown) {
  if (path !== 'ui.locale') return
  const locale = normalizeAppLocale(value) as AppLocale | null
  if (locale) setAppLocale(locale)
}

function activityLabel(item: ChatActivityKindCatalogItem): string {
  const key = `settings.appearance.chat.activityKinds.${item.id}.label`
  return t(key) !== key ? String(t(key)) : item.label
}

function activityPath(id: string): string {
  return jsonPathForKey('ui.tui.transcript.activity_kinds', id)
}

function toolPath(name: string): string {
  return jsonPathForKey('ui.tui.transcript.activity_kinds', `tool:${name}`)
}

onMounted(() => void loadCatalog())
</script>

<template>
  <div class="space-y-6">
    <div class="flex flex-wrap items-start justify-between gap-3">
      <div>
        <div class="text-lg font-medium">{{ t('settings.tabs.interface') }}</div>
        <div class="mt-1 max-w-3xl text-sm text-muted-foreground">
          {{ t('settings.tui.interfaceDescription') }}
        </div>
      </div>
      <Button variant="outline" size="sm" :disabled="loadingCatalog" @click="loadCatalog">
        <RiRefreshLine class="mr-2 h-4 w-4" :class="loadingCatalog ? 'animate-spin' : ''" />
        {{ t('settings.refresh') }}
      </Button>
    </div>

    <section class="grid gap-3">
      <div class="text-xs font-semibold uppercase tracking-[0.14em] text-muted-foreground">TUI</div>
      <ServerSettingField
        path="ui.locale"
        :label="t('settings.tui.fields.locale')"
        :description="t('settings.tui.fields.localeDescription')"
        kind="select"
        :options="localeOptions"
        default-value=""
        :include-empty="true"
        empty-label="Use runtime/default locale"
        @saved="(value) => settingSaved('ui.locale', value)"
      />
      <ServerSettingField
        path="ui.tui.color_scheme"
        :label="t('settings.tui.fields.colorScheme')"
        :description="t('settings.tui.fields.colorSchemeDescription')"
        kind="select"
        :options="colorSchemeOptions"
        default-value="auto"
      />
      <ServerSettingField
        path="ui.tui.graphics"
        :label="t('settings.tui.fields.graphics')"
        :description="t('settings.tui.fields.graphicsDescription')"
        kind="select"
        :options="graphicsOptions"
        default-value="auto"
      />
      <ServerSettingField
        path="ui.tui.theme"
        :label="t('settings.tui.fields.theme')"
        :description="t('settings.tui.fields.themeDescription')"
        kind="select"
        :options="themeOptions"
        :include-empty="true"
        empty-label="Default TUI theme"
        monospace
      />
      <ServerSettingField
        path="ui.tui.transcript.activity_default_expanded"
        :label="t('settings.tui.fields.activityDefaultExpanded')"
        :description="t('settings.tui.fields.activityDefaultExpandedDescription')"
        kind="boolean"
        :default-value="false"
      />
    </section>

    <section class="grid gap-3 border-t border-border/60 pt-5">
      <div>
        <h2 class="text-sm font-medium">{{ t('settings.tui.activityKindsTitle') }}</h2>
        <p class="mt-1 text-xs text-muted-foreground">{{ t('settings.tui.activityKindsDescription') }}</p>
      </div>
      <div v-if="loadingCatalog && activityKinds.length === 0" class="text-sm text-muted-foreground">Loading activity catalog…</div>
      <div v-else-if="catalogError" class="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive">
        {{ catalogError }}
      </div>
      <div v-else class="grid gap-2">
        <ServerSettingField
          v-for="item in activityKinds"
          :key="activityPath(item.id)"
          :path="activityPath(item.id)"
          :label="activityLabel(item)"
          :description="`${item.id} · ${item.category}`"
          kind="boolean"
          default-value=""
          :include-empty="true"
          empty-label="Inherit TUI default"
          compact
        />
      </div>
    </section>

    <section class="grid gap-3 border-t border-border/60 pt-5">
      <div>
        <h2 class="text-sm font-medium">{{ t('settings.tui.toolOverridesTitle') }}</h2>
        <p class="mt-1 text-xs text-muted-foreground">{{ t('settings.tui.toolOverridesDescription') }}</p>
      </div>
      <div v-if="toolNames.length === 0" class="text-sm text-muted-foreground">{{ t('settings.tui.noTools') }}</div>
      <div v-else class="grid gap-2">
        <ServerSettingField
          v-for="name in toolNames"
          :key="toolPath(name)"
          :path="toolPath(name)"
          :label="name"
          :description="toolPath(name)"
          kind="boolean"
          default-value=""
          :include-empty="true"
          empty-label="Inherit activity default"
          monospace
          compact
        />
      </div>
    </section>
  </div>
</template>
