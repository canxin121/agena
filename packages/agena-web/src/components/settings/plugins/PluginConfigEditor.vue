<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiDeleteBinLine, RiRefreshLine, RiSave3Line, RiShieldCheckLine } from '@remixicon/vue'

import JsonSchemaField from '@/components/settings/plugins/JsonSchemaField.vue'
import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import { jsonPathForKey, setRuntimeSetting } from '@/lib/runtimeSettings'
import { useToastsStore } from '@/stores/toasts'
import type { JsonObject, JsonValue } from '@/types/json'
import {
  cloneJson,
  deriveConfigOverride,
  isJsonRecord,
  localizedPluginSchema,
  materializeConfigValue,
  stableJson,
} from './pluginConfigSchema'
import { settingsText as st } from '@/i18n/settingsText'

type ConfiguredPlugin = {
  enabled?: boolean
  package?: JsonValue
  config?: JsonValue
  timeouts?: JsonValue
  [key: string]: JsonValue
}

type PluginManifest = {
  config_schema?: JsonValue
  config_schema_i18n?: Record<string, JsonValue>
}

const props = defineProps<{
  pluginId: string
  manifest?: PluginManifest | null
  configuredPlugin?: ConfiguredPlugin | null
  disabled?: boolean
}>()
const emit = defineEmits<{ (event: 'saved'): void }>()

const { locale } = useI18n()
const toasts = useToastsStore()
const draft = ref<JsonValue>(null)
const enabled = ref(true)
const rawText = ref('null')
const rawError = ref('')
const actionError = ref('')
const validating = ref(false)
const saving = ref(false)

const schema = computed(() => {
  const base = props.manifest?.config_schema
  if (!base || !isJsonRecord(base)) return null
  return localizedPluginSchema(base, props.manifest?.config_schema_i18n, String(locale.value || ''))
})
const defaultConfig = computed(() => materializeConfigValue(schema.value, null))
const savedConfig = computed(() => materializeConfigValue(schema.value, props.configuredPlugin?.config ?? null))
const savedOverride = computed(() => deriveConfigOverride(defaultConfig.value, savedConfig.value) ?? null)
const draftOverride = computed(() => deriveConfigOverride(defaultConfig.value, draft.value) ?? null)
const dirty = computed(
  () =>
    stableJson(draft.value) !== stableJson(savedConfig.value) ||
    enabled.value !== (props.configuredPlugin?.enabled !== false),
)
const settingsPath = computed(() => jsonPathForKey('plugins.list', props.pluginId))
const busy = computed(() => Boolean(props.disabled || validating.value || saving.value))

function baseRecord(): ConfiguredPlugin {
  const existing = props.configuredPlugin
  if (existing && isJsonRecord(existing)) return cloneJson(existing) as ConfiguredPlugin
  return {
    enabled: true,
    package: { kind: 'static' },
    config: null,
  }
}

function persistedRecord(): JsonObject {
  const record = baseRecord()
  const override = deriveConfigOverride(defaultConfig.value, draft.value)
  return {
    ...record,
    enabled: enabled.value,
    config: override ?? null,
  }
}

function sync() {
  draft.value = cloneJson(savedConfig.value)
  enabled.value = props.configuredPlugin?.enabled !== false
  rawText.value = JSON.stringify(draft.value, null, 2)
  rawError.value = ''
  actionError.value = ''
}

watch(() => [props.pluginId, props.manifest, props.configuredPlugin, locale.value] as const, sync, {
  immediate: true,
  deep: true,
})
watch(
  draft,
  (value) => {
    rawText.value = JSON.stringify(value, null, 2)
  },
  { deep: true },
)

function applyRaw() {
  try {
    draft.value = JSON.parse(rawText.value) as JsonValue
    rawError.value = ''
  } catch (reason) {
    rawError.value = reason instanceof Error ? reason.message : String(reason)
  }
}

function resetDefaults() {
  draft.value = cloneJson(defaultConfig.value)
  actionError.value = ''
}

function revert() {
  sync()
}

async function validate() {
  if (busy.value) return
  validating.value = true
  actionError.value = ''
  try {
    await setRuntimeSetting(
      settingsPath.value,
      persistedRecord(),
      { dry_run: true, validate: true, reload: false },
      'global',
    )
    toasts.push('success', st('{pluginId} configuration is valid', { pluginId: props.pluginId }))
  } catch (reason) {
    actionError.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    validating.value = false
  }
}

async function save() {
  if (busy.value) return
  saving.value = true
  actionError.value = ''
  try {
    await setRuntimeSetting(settingsPath.value, persistedRecord(), { validate: true, reload: true }, 'global')
    toasts.push('success', st('{pluginId} configuration saved and runtime reloaded', { pluginId: props.pluginId }))
    emit('saved')
  } catch (reason) {
    actionError.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <div class="grid min-w-0 gap-4">
    <div class="flex flex-wrap items-start justify-between gap-3 rounded-lg border border-border/60 bg-muted/10 p-4">
      <div>
        <h3 class="text-sm font-semibold">{{ $st('Plugin configuration') }}</h3>
        <p class="mt-1 max-w-3xl text-xs leading-5 text-muted-foreground">
          {{
            $st(
              'Edits are validated against the plugin’s JSON Schema and the complete composed Agena configuration before the global override is written.',
            )
          }}
        </p>
        <code class="mt-1 block break-all font-mono text-[10px] text-muted-foreground">{{ settingsPath }}</code>
      </div>
      <label class="inline-flex min-h-9 items-center gap-2 rounded-md border border-border/60 px-3 text-sm">
        <input v-model="enabled" type="checkbox" :disabled="busy" />
        {{ $st('Plugin enabled') }}
      </label>
    </div>

    <div
      v-if="!schema"
      class="rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-800 dark:text-amber-200"
    >
      {{
        $st(
          'This plugin does not publish a configuration schema. Raw JSON editing remains available and the server still validates the complete plugin record.',
        )
      }}
    </div>

    <JsonSchemaField
      v-if="schema"
      v-model="draft"
      :schema="schema"
      :root-schema="schema"
      :label="$st('Configuration')"
      :disabled="busy"
    />

    <details class="rounded-lg border border-border/60">
      <summary class="cursor-pointer px-4 py-3 text-sm font-medium">
        {{ $st('Configuration diff & persisted override') }}
      </summary>
      <div class="grid gap-3 border-t border-border/60 p-4 lg:grid-cols-2">
        <div class="grid min-w-0 gap-1.5">
          <div class="text-xs font-medium text-muted-foreground">{{ $st('Saved override') }}</div>
          <pre class="max-h-72 overflow-auto rounded-md border border-border/60 p-3 font-mono text-[11px] leading-5">{{
            JSON.stringify(savedOverride, null, 2)
          }}</pre>
        </div>
        <div class="grid min-w-0 gap-1.5">
          <div class="text-xs font-medium text-muted-foreground">{{ $st('Draft override') }}</div>
          <pre class="max-h-72 overflow-auto rounded-md border border-border/60 p-3 font-mono text-[11px] leading-5">{{
            JSON.stringify(draftOverride, null, 2)
          }}</pre>
        </div>
      </div>
    </details>

    <details class="rounded-lg border border-border/60">
      <summary class="cursor-pointer px-4 py-3 text-sm font-medium">{{ $st('Raw configuration JSON') }}</summary>
      <div class="grid gap-2 border-t border-border/60 p-4">
        <textarea
          v-model="rawText"
          rows="16"
          spellcheck="false"
          :disabled="busy"
          class="w-full rounded-md border border-input bg-transparent p-3 font-mono text-xs leading-5 outline-none focus:border-ring"
        />
        <div class="flex flex-wrap items-center justify-between gap-2">
          <span v-if="rawError" class="text-xs text-destructive">{{ rawError }}</span>
          <span v-else class="text-xs text-muted-foreground">{{
            $st('Use this editor for open-ended or schema-unsupported structures.')
          }}</span>
          <Button variant="outline" size="sm" :disabled="busy" @click="applyRaw">{{ $st('Apply JSON') }}</Button>
        </div>
      </div>
    </details>

    <div
      v-if="actionError"
      class="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive"
    >
      {{ actionError }}
    </div>

    <div class="flex flex-wrap items-center justify-between gap-3 border-t border-border/60 pt-4">
      <div class="flex flex-wrap gap-2">
        <Button variant="ghost" size="sm" :disabled="busy || !dirty" @click="revert">
          <RiRefreshLine class="mr-1.5 h-4 w-4" /> {{ $st('Revert') }}
        </Button>
        <Button variant="ghost" size="sm" :disabled="busy" @click="resetDefaults">
          <RiDeleteBinLine class="mr-1.5 h-4 w-4" /> {{ $st('Reset to defaults') }}
        </Button>
      </div>
      <div class="flex flex-wrap gap-2">
        <Button variant="outline" size="sm" :disabled="busy" @click="validate">
          <RiShieldCheckLine class="mr-1.5 h-4 w-4" /> {{ validating ? $st('Validating…') : $st('Validate') }}
        </Button>
        <Button size="sm" :disabled="busy || !dirty" @click="save">
          <RiSave3Line class="mr-1.5 h-4 w-4" /> {{ saving ? $st('Saving…') : $st('Save & reload') }}
        </Button>
      </div>
    </div>
  </div>
</template>
