<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiRefreshLine } from '@remixicon/vue'

import Button from '@/components/ui/Button.vue'
import ServerSettingField from '@/components/settings/ServerSettingField.vue'
import { apiJson } from '@/lib/api'
import { TUI_LOCALE_OPTIONS } from '@/i18n/tuiLocale'
import { settingsText as st } from '@/i18n/settingsText'
import { usePaneVisibility } from '@/composables/usePaneVisibility'

type ToolCatalogResponse = {
  catalog?: {
    // The plugin surface catalog nests its presentation data under
    // `terminal`, matching the runtime resource field name.
    terminal?: {
      display?: unknown[]
      themes?: Array<{ id?: string; display_name?: string; plugin_id?: string }>
    }
    operations?: unknown[]
  }
}

const { t } = useI18n()
const loadingCatalog = ref(false)
const catalogError = ref('')
const themes = ref<Array<{ value: string; label: string; pluginId: string }>>([])
const themeOptions = computed(() =>
  themes.value.map(({ value, label, pluginId }) => ({
    value,
    label,
    description: pluginId ? st('Plugin: {pluginId}', { pluginId }) : undefined,
  })),
)
const visible = usePaneVisibility()
let catalogController: AbortController | undefined
let pendingCatalog = true

const localeOptions = computed(() =>
  TUI_LOCALE_OPTIONS.map((option) => ({
    value: option.value,
    label: option.label,
    description: option.value,
  })),
)

const colorSchemeOptions = computed(() => [
  { value: 'auto', label: st('Auto'), description: st('Follow the terminal environment.') },
  { value: 'dark', label: st('Dark'), description: st('Use the dark TUI palette.') },
  { value: 'light', label: st('Light'), description: st('Use the light TUI palette.') },
])

const graphicsOptions = computed(() => [
  { value: 'auto', label: st('Auto'), description: st('Use native terminal graphics when available.') },
  { value: 'native', label: st('Native'), description: st('Prefer native terminal graphics.') },
  { value: 'unicode', label: st('Unicode'), description: st('Use portable Unicode rendering.') },
])

async function loadCatalog() {
  if (!visible.value) {
    pendingCatalog = true
    return
  }
  if (loadingCatalog.value) return
  pendingCatalog = false
  const controller = new AbortController()
  catalogController = controller
  loadingCatalog.value = true
  catalogError.value = ''
  try {
    const response = await apiJson<ToolCatalogResponse>('/api/v1/plugins/surface', {
      signal: AbortSignal.any([controller.signal, AbortSignal.timeout(15_000)]),
    })
    if (controller.signal.aborted || catalogController !== controller) return
    themes.value = (Array.isArray(response?.catalog?.terminal?.themes) ? response.catalog.terminal.themes : []).flatMap(
      (theme) => {
        const id = String(theme?.id || '').trim()
        const label = String(theme?.display_name || id).trim()
        const pluginId = String(theme?.plugin_id || '').trim()
        if (!id) return []
        return [
          {
            value: id,
            label: label || id,
            pluginId,
          },
        ]
      },
    )
  } catch (reason) {
    if (controller.signal.aborted || catalogController !== controller) return
    catalogError.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    if (catalogController === controller) {
      catalogController = undefined
      loadingCatalog.value = false
    }
  }
}

onMounted(() => void loadCatalog())
watch(
  visible,
  (shown) => {
    if (shown) {
      if (pendingCatalog) void loadCatalog()
    } else if (catalogController) {
      pendingCatalog = true
      catalogController.abort()
      catalogController = undefined
      loadingCatalog.value = false
    }
  },
  { flush: 'sync' },
)
onBeforeUnmount(() => catalogController?.abort())
</script>

<template>
  <div class="space-y-6">
    <div class="flex flex-wrap items-center justify-between gap-3">
      <div class="max-w-3xl text-sm text-muted-foreground">
        {{ t('settings.tui.interfaceDescription') }}
      </div>
      <Button variant="outline" size="sm" :disabled="loadingCatalog" @click="loadCatalog">
        <RiRefreshLine class="mr-2 h-4 w-4" :class="loadingCatalog ? 'animate-spin' : ''" />
        {{ t('settings.refresh') }}
      </Button>
    </div>

    <section class="grid gap-3">
      <div class="text-xs font-semibold uppercase tracking-[0.14em] text-muted-foreground">TUI</div>
      <div class="rounded-md border border-border/60 bg-muted/20 px-3 py-2 text-xs leading-5 text-muted-foreground">
        {{
          $st(
            'This locale is consumed by server-backed TUI clients. The Web interface language is configured separately on the Web appearance page.',
          )
        }}
      </div>
      <ServerSettingField
        path="ui.locale"
        :label="t('settings.tui.fields.locale')"
        :description="t('settings.tui.fields.localeDescription')"
        kind="select"
        :options="localeOptions"
        default-value=""
        :include-empty="true"
        :empty-label="$st('Use runtime/default locale')"
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
        :empty-label="$st('Default TUI theme')"
        monospace
      />
    </section>
  </div>
</template>
