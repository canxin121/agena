<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { RiRefreshLine, RiShieldCheckLine } from '@remixicon/vue'
import SettingsCard from './SettingsCard.vue'
import SettingsToggle from './SettingsToggle.vue'
import Button from '@/components/ui/Button.vue'
import { usePaneVisibility } from '@/composables/usePaneVisibility'
import { settingsText as st } from '@/i18n/settingsText'
import {
  deleteRuntimeSetting,
  readRuntimeSettingSources,
  setRuntimeSetting,
  type RuntimeSettingsLayer,
} from '@/lib/runtimeSettings'
import {
  SANDBOX_SETTING_PATHS,
  sandboxExecutionState,
  sandboxLayerOverride,
  sandboxLayerSettings,
  type SandboxSettingKey,
  type SandboxSettingSources,
} from '@/lib/sandboxSettings'

const props = defineProps<{ scope: string }>()
const layer = computed<RuntimeSettingsLayer>(() => (props.scope === 'workspace' ? 'workspace' : 'global'))
const sources = ref<SandboxSettingSources | null>(null)
const loading = ref(false)
const saving = ref(false)
const error = ref('')
const visible = usePaneVisibility()
const busy = computed(() => loading.value || saving.value || !sources.value)
const pendingEdit = ref<{ layer: RuntimeSettingsLayer; key: SandboxSettingKey; value: boolean } | null>(null)
const configured = computed(() => {
  const value = sandboxLayerSettings(sources.value, layer.value)
  if (pendingEdit.value?.layer === layer.value) value[pendingEdit.value.key] = pendingEdit.value.value
  return value
})
const execution = computed(() => sandboxExecutionState(sources.value))
let readController: AbortController | undefined
let readSequence = 0
let disposed = false

const fields = computed(() => [
  {
    key: 'enabled' as const,
    label: st('Native sandbox'),
    description: st('Run agent tools with MXC filesystem and network isolation.'),
    active: execution.value.enabled,
    status: '',
  },
  {
    key: 'resource_guard' as const,
    label: st('Resource preflight'),
    description: st(
      'When MXC is not used, validate declared paths and network targets against permissions and ask the AI to add missing command resources.',
    ),
    active: execution.value.resourceGuard,
    status: execution.value.resourceGuardStandby ? st('On standby while the MXC sandbox is enabled.') : '',
  },
])

async function refresh() {
  if (!visible.value || saving.value || disposed) return
  readController?.abort()
  const controller = new AbortController()
  readController = controller
  const sequence = ++readSequence
  loading.value = true
  error.value = ''
  try {
    const [enabled, resourceGuard] = await Promise.all([
      readRuntimeSettingSources(SANDBOX_SETTING_PATHS.enabled, controller.signal),
      readRuntimeSettingSources(SANDBOX_SETTING_PATHS.resource_guard, controller.signal),
    ])
    if (!controller.signal.aborted && sequence === readSequence) {
      sources.value = { enabled, resource_guard: resourceGuard }
    }
  } catch (reason) {
    if (!controller.signal.aborted && sequence === readSequence) {
      error.value = reason instanceof Error ? reason.message : String(reason)
    }
  } finally {
    if (sequence === readSequence) {
      loading.value = false
      readController = undefined
    }
  }
}

async function save(key: SandboxSettingKey, value: boolean | undefined) {
  if (busy.value) return
  const target = layer.value
  saving.value = true
  error.value = ''
  pendingEdit.value = {
    layer: target,
    key,
    value: value ?? (target === 'workspace' ? sandboxLayerSettings(sources.value, 'global')[key] : true),
  }
  try {
    const path = SANDBOX_SETTING_PATHS[key]
    if (value === undefined) await deleteRuntimeSetting(path, { reload: true }, target)
    else await setRuntimeSetting(path, value, { reload: true }, target)
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
    pendingEdit.value = null
    return
  } finally {
    saving.value = false
  }
  await refresh()
  pendingEdit.value = null
}

watch(
  visible,
  (value) => {
    if (value) void refresh()
  },
  { immediate: true },
)
onBeforeUnmount(() => {
  disposed = true
  readSequence++
  readController?.abort()
})
defineExpose({ refresh })
</script>

<template>
  <div class="space-y-4" data-sandbox-settings>
    <SettingsCard
      :title="st('Current execution mode')"
      :description="st('Applies to agent tool execution.')"
      :icon="RiShieldCheckLine"
    >
      <template #actions>
        <Button variant="ghost" size="sm" :disabled="loading || saving" @click="refresh">
          <RiRefreshLine class="mr-1.5 h-4 w-4" />{{ st('Refresh') }}
        </Button>
      </template>
      <p v-if="!sources" class="text-sm text-muted-foreground">
        {{ loading ? st('Loading...') : st('Settings could not be loaded.') }}
      </p>
      <template v-else>
        <p class="text-sm font-medium" aria-live="polite">
          {{ execution.enabled ? st('MXC sandbox enabled') : st('Direct execution') }}
        </p>
        <p class="mt-1 text-xs leading-relaxed text-muted-foreground">
          {{ st('Saved preferences remain available when execution modes change.') }}
        </p>
        <p v-if="execution.environmentDisabled" class="mt-3 text-sm text-amber-700 dark:text-amber-300" role="status">
          {{ st('The runtime environment has disabled the sandbox. Remove that override to enable MXC.') }}
        </p>
      </template>
    </SettingsCard>

    <SettingsCard
      :title="st('Sandbox & resource preflight')"
      :description="st('Changes save automatically to the selected layer.')"
    >
      <div
        v-for="field in fields"
        :key="field.key"
        class="border-b border-border/50 py-4 first:pt-0 last:border-b-0 last:pb-0"
        :data-sandbox-field="field.key"
      >
        <div class="flex items-start justify-between gap-4">
          <div class="min-w-0">
            <h3 class="text-sm font-medium">{{ field.label }}</h3>
            <p class="mt-1 text-xs leading-relaxed text-muted-foreground">{{ field.description }}</p>
          </div>
          <SettingsToggle
            :model-value="configured[field.key]"
            :label="field.label"
            :disabled="busy"
            @update:model-value="save(field.key, $event)"
          />
        </div>
        <div
          v-if="sources"
          class="mt-3 flex flex-wrap items-center justify-between gap-2 text-xs text-muted-foreground"
        >
          <div class="flex flex-wrap items-center gap-2">
            <span>{{
              sandboxLayerOverride(sources, layer, field.key) === undefined
                ? st('Inherited value')
                : st('Saved in this layer')
            }}</span>
            <span class="rounded-md bg-muted px-2 py-0.5" :data-active="field.active">{{
              field.active ? st('Active now') : st('Inactive now')
            }}</span>
          </div>
          <Button
            v-if="sandboxLayerOverride(sources, layer, field.key) !== undefined"
            variant="ghost"
            size="sm"
            :disabled="busy"
            :aria-label="`${st('Use inherited value')}: ${field.label}`"
            @click="save(field.key, undefined)"
          >
            {{ st('Use inherited value') }}
          </Button>
        </div>
        <p v-if="sources && field.status" class="mt-2 text-xs text-muted-foreground">{{ field.status }}</p>
      </div>
    </SettingsCard>
    <p v-if="saving" class="text-xs text-muted-foreground" role="status">{{ st('Saving…') }}</p>
    <p
      v-if="error"
      class="rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
      role="alert"
    >
      {{ error }}
    </p>
  </div>
</template>
