<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiRefreshLine, RiRestartLine } from '@remixicon/vue'

import Button from '@/components/ui/Button.vue'
import ServerSettingField from '@/components/settings/ServerSettingField.vue'
import { apiJson } from '@/lib/api'
import { reloadAgenaRuntime } from '@/lib/reload'
import { validateRuntimeSettings } from '@/lib/runtimeSettings'
import { useToastsStore } from '@/stores/toasts'
import type { JsonValue } from '@/types/json'

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
const loading = ref(false)
const actionBusy = ref(false)
const error = ref('')
const runtime = ref<RuntimeStatus | null>(null)
const resolved = ref<ResolvedDocument | null>(null)
const validation = ref<JsonValue>(null)
const reloadInfo = ref<JsonValue>(null)

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
    toasts.push('success', 'Runtime settings are valid')
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
    reloadInfo.value = await reloadAgenaRuntime()
    await refresh()
    toasts.push('success', 'Runtime reloaded')
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    actionBusy.value = false
  }
}

onMounted(() => void refresh())
</script>

<template>
  <div class="space-y-6">
    <div class="flex flex-wrap items-start justify-between gap-3">
      <div>
        <div class="text-lg font-medium">{{ t('settings.tabs.diagnostics') }}</div>
        <div class="mt-1 max-w-3xl text-sm text-muted-foreground">{{ t('settings.tui.diagnosticsDescription') }}</div>
      </div>
      <div class="flex gap-2">
        <Button variant="outline" size="sm" :disabled="loading || actionBusy" @click="refresh"
          ><RiRefreshLine class="mr-2 h-4 w-4" :class="loading ? 'animate-spin' : ''" /> Refresh</Button
        >
        <Button variant="outline" size="sm" :disabled="actionBusy" @click="validate">Validate</Button>
        <Button size="sm" :disabled="actionBusy" @click="reload"
          ><RiRestartLine class="mr-2 h-4 w-4" /> Reload runtime</Button
        >
      </div>
    </div>
    <div
      v-if="error"
      class="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
    >
      {{ error }}
    </div>

    <section class="grid gap-3">
      <div class="text-xs font-semibold uppercase tracking-[0.14em] text-muted-foreground">Tracing</div>
      <ServerSettingField
        path="tracing.filter"
        :label="t('settings.tui.fields.tracingFilter')"
        :description="t('settings.tui.fields.tracingFilterDescription')"
        kind="select"
        :options="tracingLevelOptions"
        default-value="info"
        monospace
        compact
      />
      <ServerSettingField
        path="tracing.database"
        :label="t('settings.tui.fields.tracingDatabase')"
        :description="t('settings.tui.fields.tracingDatabaseDescription')"
        kind="select"
        :options="tracingLevelOptions"
        default-value="error"
        monospace
        compact
      />
      <ServerSettingField
        path="tracing.adapter"
        :label="t('settings.tui.fields.tracingAdapter')"
        :description="t('settings.tui.fields.tracingAdapterDescription')"
        kind="select"
        :options="tracingLevelOptions"
        default-value="off"
        monospace
        compact
      />
    </section>

    <section v-if="runtime" class="grid gap-3 border-t border-border/60 pt-5">
      <div class="text-xs font-semibold uppercase tracking-[0.14em] text-muted-foreground">Runtime snapshot</div>
      <dl class="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
        <div class="rounded-md border border-border/60 p-3">
          <dt class="text-[11px] text-muted-foreground">Generation</dt>
          <dd class="mt-1 font-mono text-sm">{{ runtime.generation ?? '—' }}</dd>
        </div>
        <div class="rounded-md border border-border/60 p-3">
          <dt class="text-[11px] text-muted-foreground">Loaded at</dt>
          <dd class="mt-1 break-all font-mono text-xs">{{ runtime.loaded_at || '—' }}</dd>
        </div>
        <div class="rounded-md border border-border/60 p-3">
          <dt class="text-[11px] text-muted-foreground">Providers</dt>
          <dd class="mt-1 font-mono text-sm">{{ runtime.provider_ids?.length ?? 0 }}</dd>
        </div>
        <div class="rounded-md border border-border/60 p-3">
          <dt class="text-[11px] text-muted-foreground">Plugins</dt>
          <dd class="mt-1 font-mono text-sm">{{ runtime.plugin_count ?? 0 }}</dd>
        </div>
      </dl>
      <div class="grid gap-1 text-xs text-muted-foreground">
        <div>
          <span class="font-medium text-foreground">Workspace:</span> <code>{{ runtime.workspace_root || '—' }}</code>
        </div>
        <div>
          <span class="font-medium text-foreground">Config:</span> <code>{{ runtime.config_path || '—' }}</code> ·
          {{ runtime.config_found ? 'found' : 'not found' }}
        </div>
        <div>
          <span class="font-medium text-foreground">Session runtime:</span>
          {{ runtime.session_runtime_available ? 'available' : 'unavailable' }}
        </div>
      </div>
    </section>

    <section v-if="resolved" class="grid gap-3 border-t border-border/60 pt-5">
      <div class="text-xs font-semibold uppercase tracking-[0.14em] text-muted-foreground">Configuration sources</div>
      <div class="grid gap-2 text-xs">
        <div>
          <span class="font-medium">Global file:</span>
          <code>{{ resolved.meta?.config_path || runtime?.config_path || '—' }}</code> ·
          {{ resolved.meta?.config_found ? 'found' : 'not found' }}
        </div>
        <div>
          <span class="font-medium">Workspace file:</span>
          <code>{{ resolved.meta?.project_config_path || '—' }}</code> ·
          {{ resolved.meta?.project_config_found ? 'found' : 'not found' }}
        </div>
      </div>
      <div class="grid gap-1">
        <div class="text-sm font-medium">Active layers</div>
        <div
          v-for="(layer, index) in appliedLayers"
          :key="`${layer.source}-${index}`"
          class="rounded-md border border-border/50 px-3 py-2 text-xs"
        >
          <span class="font-mono">{{ layer.source }}</span
          ><span class="ml-2 text-muted-foreground">{{ layer.description }}</span>
        </div>
      </div>
      <details class="rounded-md border border-border/60">
        <summary class="cursor-pointer px-3 py-2 text-sm font-medium">Resolved configuration document</summary>
        <pre class="max-h-[32rem] overflow-auto border-t border-border/60 p-3 font-mono text-[11px] leading-5">{{
          resolvedJson
        }}</pre>
      </details>
    </section>

    <section v-if="validation || reloadInfo" class="grid gap-2 border-t border-border/60 pt-5">
      <div class="text-xs font-semibold uppercase tracking-[0.14em] text-muted-foreground">Last action</div>
      <pre v-if="validation" class="overflow-auto rounded-md border border-border/60 p-3 font-mono text-xs">{{
        JSON.stringify(validation, null, 2)
      }}</pre>
      <pre v-if="reloadInfo" class="overflow-auto rounded-md border border-border/60 p-3 font-mono text-xs">{{
        JSON.stringify(reloadInfo, null, 2)
      }}</pre>
    </section>
  </div>
</template>
