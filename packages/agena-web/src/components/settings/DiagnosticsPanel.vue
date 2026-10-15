<script setup lang="ts">
import { computed, onMounted, onScopeDispose, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import {
  RiRefreshLine,
  RiRestartLine,
  RiPulseLine,
  RiFileList2Line,
  RiTerminalBoxLine,
  RiCheckboxCircleLine,
} from '@remixicon/vue'

import SettingsCard from './SettingsCard.vue'
import Button from '@/components/ui/Button.vue'
import ServerSettingField from '@/components/settings/ServerSettingField.vue'
import { apiJson } from '@/lib/api'
import { reloadAgenaRuntime } from '@/lib/reload'
import { validateRuntimeSettings } from '@/lib/runtimeSettings'
import { useToastsStore } from '@/stores/toasts'
import type { JsonValue } from '@/types/json'
import { settingsText as st } from '@/i18n/settingsText'

type RuntimeStatus = {
  generation?: number
  loaded_at?: string
  workspace_root?: string
  config_path?: string
  config_found?: boolean
  provider_ids?: string[]
  plugin_count?: number
  session_runtime_available?: boolean
  watch_paths?: string[]
  background_tasks?: Array<{ id?: string; kind?: string; status?: string; title?: string; message?: string | null }>
}

type ResolvedDocument = {
  config?: JsonValue
  meta?: {
    config_path?: string
    config_found?: boolean
    project_config_path?: string
    project_config_found?: boolean
    applied_layers?: Array<{ source?: string; description?: string }>
  }
}

const { t } = useI18n()
const toasts = useToastsStore()
const fields = ref<InstanceType<typeof ServerSettingField>[]>([])
const loading = ref(false)
const actionBusy = ref(false)
const error = ref('')
const runtime = ref<RuntimeStatus | null>(null)
const resolved = ref<ResolvedDocument | null>(null)
const validation = ref<JsonValue>(null)
const reloadInfo = ref<JsonValue>(null)
const reloadController = new AbortController()
onScopeDispose(() => reloadController.abort())

// Keep the tracing editors aligned with the TUI's SelectOnly choices. The
// runtime still validates these values server-side, but offering the same
// finite catalog here prevents the web workbench from drifting into a
// free-form editor for a setting that the TUI deliberately constrains.
const tracingLevelOptions = [
  { value: 'off', label: 'off' },
  { value: 'error', label: 'error' },
  { value: 'warn', label: 'warn' },
  { value: 'info', label: 'info' },
  { value: 'debug', label: 'debug' },
  { value: 'trace', label: 'trace' },
]

const appliedLayers = computed(() => resolved.value?.meta?.applied_layers || [])
const resolvedJson = computed(() => JSON.stringify(resolved.value?.config ?? {}, null, 2))

async function refresh() {
  if (loading.value) return
  loading.value = true
  error.value = ''
  try {
    const [runtimeResponse, resolvedResponse] = await Promise.all([
      apiJson<RuntimeStatus>('/api/v1/runtime'),
      apiJson<ResolvedDocument>('/api/v1/config/resolved'),
    ])
    runtime.value = runtimeResponse
    resolved.value = resolvedResponse
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    loading.value = false
  }
}

async function validate() {
  actionBusy.value = true
  error.value = ''
  try {
    validation.value = await validateRuntimeSettings()
    toasts.push('success', st('Runtime settings are valid'))
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    actionBusy.value = false
  }
}

async function reload() {
  actionBusy.value = true
  error.value = ''
  try {
    reloadInfo.value = await reloadAgenaRuntime(reloadController.signal)
    await refresh()
    toasts.push('success', st('Runtime reloaded'))
  } catch (reason) {
    if (reloadController.signal.aborted) return
    error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    actionBusy.value = false
  }
}

const tracingFields = computed(() => [
  {
    path: 'tracing.filter',
    label: t('settings.tui.fields.tracingFilter'),
    description: t('settings.tui.fields.tracingFilterDescription'),
    defaultValue: 'info',
  },
  {
    path: 'tracing.database',
    label: t('settings.tui.fields.tracingDatabase'),
    description: t('settings.tui.fields.tracingDatabaseDescription'),
    defaultValue: 'error',
  },
  {
    path: 'tracing.adapter',
    label: t('settings.tui.fields.tracingAdapter'),
    description: t('settings.tui.fields.tracingAdapterDescription'),
    defaultValue: 'off',
  },
])
defineExpose({ refresh: () => Promise.all([refresh(), ...fields.value.map((field) => field.refresh())]) })
onMounted(() => void refresh())
</script>

<template>
  <div class="min-w-0 space-y-4">
    <p
      v-if="error"
      role="alert"
      class="break-words rounded-lg border border-destructive/30 bg-destructive/10 p-3 text-sm text-destructive"
    >
      {{ error }}
    </p>
    <SettingsCard :title="$st('Runtime snapshot')" :icon="RiPulseLine">
      <template #actions>
        <div class="flex flex-wrap gap-2">
          <Button variant="outline" size="sm" :disabled="loading || actionBusy" @click="refresh"
            ><RiRefreshLine class="mr-1.5 h-4 w-4 shrink-0" :class="loading ? 'animate-spin' : ''" />{{
              $st('Refresh')
            }}</Button
          >
          <Button variant="outline" size="sm" :disabled="loading || actionBusy" @click="validate">{{
            $st('Validate')
          }}</Button>
          <Button size="sm" :disabled="loading || actionBusy" @click="reload"
            ><RiRestartLine class="mr-1.5 h-4 w-4 shrink-0" />{{ $st('Reload runtime') }}</Button
          >
        </div>
      </template>
      <p v-if="loading && !runtime" class="py-4 text-center text-sm text-muted-foreground">{{ $st('Loading...') }}</p>
      <template v-if="runtime">
        <dl class="grid grid-cols-2 gap-3 xl:grid-cols-4">
          <div class="min-w-0 rounded-lg border border-border/50 bg-muted/20 p-3">
            <dt class="text-xs text-muted-foreground">{{ $st('Generation') }}</dt>
            <dd class="mt-1 font-mono text-lg font-semibold">{{ runtime.generation ?? '—' }}</dd>
          </div>
          <div class="min-w-0 rounded-lg border border-border/50 bg-muted/20 p-3">
            <dt class="text-xs text-muted-foreground">{{ $st('Providers') }}</dt>
            <dd class="mt-1 font-mono text-lg font-semibold">{{ runtime.provider_ids?.length ?? 0 }}</dd>
          </div>
          <div class="min-w-0 rounded-lg border border-border/50 bg-muted/20 p-3">
            <dt class="text-xs text-muted-foreground">{{ $st('Plugins') }}</dt>
            <dd class="mt-1 font-mono text-lg font-semibold">{{ runtime.plugin_count ?? 0 }}</dd>
          </div>
          <div class="min-w-0 rounded-lg border border-border/50 bg-muted/20 p-3">
            <dt class="text-xs text-muted-foreground">{{ $st('Loaded at') }}</dt>
            <dd class="mt-2 break-words font-mono text-xs">{{ runtime.loaded_at || '—' }}</dd>
          </div>
        </dl>
        <dl class="mt-4 divide-y divide-border/50 text-xs">
          <div class="grid gap-1.5 py-2.5 sm:grid-cols-[9rem_minmax(0,1fr)]">
            <dt class="text-muted-foreground">{{ $st('Workspace:') }}</dt>
            <dd class="break-all font-mono">{{ runtime.workspace_root || '—' }}</dd>
          </div>
          <div class="grid gap-1.5 py-2.5 sm:grid-cols-[9rem_minmax(0,1fr)]">
            <dt class="text-muted-foreground">{{ $st('Config:') }}</dt>
            <dd class="break-all font-mono">
              {{ runtime.config_path || '—' }}
              <span class="ml-2 font-sans text-muted-foreground">{{
                runtime.config_found ? $st('found') : $st('not found')
              }}</span>
            </dd>
          </div>
          <div class="grid gap-1.5 pt-2.5 sm:grid-cols-[9rem_minmax(0,1fr)]">
            <dt class="text-muted-foreground">{{ $st('Session runtime:') }}</dt>
            <dd class="flex items-center gap-2">
              <span
                class="h-1.5 w-1.5 rounded-full"
                :class="runtime.session_runtime_available ? 'bg-emerald-500' : 'bg-amber-500'"
              />{{ runtime.session_runtime_available ? $st('available') : $st('unavailable') }}
            </dd>
          </div>
        </dl>
      </template>
    </SettingsCard>
    <SettingsCard :title="$st('Tracing')" :icon="RiTerminalBoxLine">
      <ServerSettingField
        v-for="field in tracingFields"
        :key="field.path"
        ref="fields"
        v-bind="field"
        kind="select"
        :options="tracingLevelOptions"
        monospace
        compact
      />
    </SettingsCard>
    <SettingsCard v-if="resolved" :title="$st('Configuration sources')" :icon="RiFileList2Line">
      <dl class="grid gap-3 sm:grid-cols-2">
        <div class="min-w-0 rounded-lg border border-border/50 bg-muted/20 p-3">
          <dt class="text-xs font-medium">
            {{ $st('Global file:') }}
            <span class="text-muted-foreground">{{
              resolved.meta?.config_found ? $st('found') : $st('not found')
            }}</span>
          </dt>
          <dd class="mt-2 break-all font-mono text-xs text-muted-foreground">
            {{ resolved.meta?.config_path || runtime?.config_path || '—' }}
          </dd>
        </div>
        <div class="min-w-0 rounded-lg border border-border/50 bg-muted/20 p-3">
          <dt class="text-xs font-medium">
            {{ $st('Workspace file:') }}
            <span class="text-muted-foreground">{{
              resolved.meta?.project_config_found ? $st('found') : $st('not found')
            }}</span>
          </dt>
          <dd class="mt-2 break-all font-mono text-xs text-muted-foreground">
            {{ resolved.meta?.project_config_path || '—' }}
          </dd>
        </div>
      </dl>
      <div v-if="appliedLayers.length" class="mt-4">
        <h3 class="mb-2 text-xs font-medium text-muted-foreground">{{ $st('Active layers') }}</h3>
        <ol class="divide-y divide-border/50">
          <li
            v-for="(layer, index) in appliedLayers"
            :key="`${layer.source}-${index}`"
            class="flex min-w-0 items-start gap-3 py-2.5 text-xs"
          >
            <span
              class="flex h-5 w-5 shrink-0 items-center justify-center rounded-md bg-muted font-mono text-[11px] text-muted-foreground"
              >{{ index + 1 }}</span
            >
            <div class="min-w-0">
              <div class="break-all font-mono font-medium">{{ layer.source }}</div>
              <p class="mt-1 break-words text-muted-foreground">{{ layer.description }}</p>
            </div>
          </li>
        </ol>
      </div>
      <details class="mt-3 min-w-0 rounded-lg border border-border/60 bg-background/30">
        <summary class="cursor-pointer px-3 py-2.5 text-sm font-medium">
          {{ $st('Resolved configuration document') }}
        </summary>
        <pre dir="ltr" class="max-h-96 overflow-auto border-t border-border/60 p-3 font-mono text-xs leading-5">{{
          resolvedJson
        }}</pre>
      </details>
    </SettingsCard>
    <SettingsCard v-if="validation || reloadInfo" :title="$st('Last action')" :icon="RiCheckboxCircleLine">
      <details v-if="validation" class="min-w-0 rounded-lg border border-border/50">
        <summary class="cursor-pointer px-3 py-2.5 text-sm">{{ $st('Runtime settings are valid') }}</summary>
        <pre dir="ltr" class="max-h-64 overflow-auto border-t border-border/50 p-3 font-mono text-xs">{{
          JSON.stringify(validation, null, 2)
        }}</pre>
      </details>
      <details v-if="reloadInfo" class="mt-2 min-w-0 rounded-lg border border-border/50">
        <summary class="cursor-pointer px-3 py-2.5 text-sm">{{ $st('Runtime reloaded') }}</summary>
        <pre dir="ltr" class="max-h-64 overflow-auto border-t border-border/50 p-3 font-mono text-xs">{{
          JSON.stringify(reloadInfo, null, 2)
        }}</pre>
      </details>
    </SettingsCard>
  </div>
</template>
