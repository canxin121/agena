<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiRefreshLine, RiTranslate2, RiPaletteLine } from '@remixicon/vue'

import SettingsCard from './SettingsCard.vue'
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
const fields = ref<InstanceType<typeof ServerSettingField>[]>([])
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

const groups = computed(() => [
  {
    title: t('settings.appearance.language.label'),
    icon: RiTranslate2,
    description: st(
      'This locale is consumed by server-backed TUI clients. The Web interface language is configured separately on the Web appearance page.',
    ),
    fields: [
      {
        path: 'ui.locale',
        label: t('settings.tui.fields.locale'),
        description: t('settings.tui.fields.localeDescription'),
        options: localeOptions.value,
        defaultValue: '',
        includeEmpty: true,
        emptyLabel: st('Use runtime/default locale'),
      },
    ],
  },
  {
    title: st('Terminal appearance'),
    icon: RiPaletteLine,
    fields: [
      {
        path: 'ui.tui.color_scheme',
        label: t('settings.tui.fields.colorScheme'),
        description: t('settings.tui.fields.colorSchemeDescription'),
        options: colorSchemeOptions.value,
        defaultValue: 'auto',
      },
      {
        path: 'ui.tui.theme',
        label: t('settings.tui.fields.theme'),
        description: t('settings.tui.fields.themeDescription'),
        options: themeOptions.value,
        includeEmpty: true,
        emptyLabel: st('Default TUI theme'),
      },
      {
        path: 'ui.tui.graphics',
        label: t('settings.tui.fields.graphics'),
        description: t('settings.tui.fields.graphicsDescription'),
        options: graphicsOptions.value,
        defaultValue: 'auto',
      },
    ],
  },
])
async function refresh() {
  await Promise.all([loadCatalog(), ...fields.value.map((field) => field.refresh())])
}
defineExpose({ refresh })
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
  <div class="space-y-4">
    <SettingsCard
      v-for="(group, index) in groups"
      :key="index"
      :title="group.title"
      :description="group.description"
      :icon="group.icon"
    >
      <template v-if="index === 1" #actions>
        <Button variant="outline" size="sm" :disabled="loadingCatalog" @click="loadCatalog">
          <RiRefreshLine class="mr-1.5 h-4 w-4 shrink-0" :class="loadingCatalog ? 'animate-spin' : ''" />{{
            t('settings.refresh')
          }}
        </Button>
      </template>
      <p
        v-if="index === 1 && catalogError"
        role="alert"
        class="mb-3 break-words rounded-lg border border-destructive/30 bg-destructive/10 p-3 text-xs text-destructive"
      >
        {{ catalogError }}
      </p>
      <ServerSettingField
        v-for="field in group.fields"
        :key="field.path"
        ref="fields"
        v-bind="field"
        kind="select"
        compact
      />
    </SettingsCard>
  </div>
</template>
