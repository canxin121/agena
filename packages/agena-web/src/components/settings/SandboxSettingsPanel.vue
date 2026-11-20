<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { RiRefreshLine, RiShieldCheckLine } from '@remixicon/vue'
import SettingsCard from './SettingsCard.vue'
import SettingsToggle from './SettingsToggle.vue'
import SettingsSourceControl from './SettingsSourceControl.vue'
import IconButton from '@/components/ui/IconButton.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import { usePaneVisibility } from '@/composables/usePaneVisibility'
import { settingsText as st } from '@/i18n/settingsText'
import {
  deleteRuntimeSetting,
  getRuntimeSettingLayer,
  subscribeRuntimeSettingsChanges,
  readRuntimeSettingSources,
  setRuntimeSetting,
  type RuntimeSettingsLayer,
} from '@/lib/runtimeSettings'
import {
  sandboxExecutionState,
  sandboxEffectiveSettings,
  sandboxInheritedKnown,
  sandboxInheritedValue,
  sandboxLayerOverride,
  sandboxLayerSettings,
  type SandboxSettingKey,
  type SandboxSettingSources,
} from '@/lib/sandboxSettings'

const props = defineProps<{ scope: string; workspaceId?: string; activeWorkspace?: boolean; disabled?: boolean }>()
const emit = defineEmits<{ saved: [] }>()
const layer = computed<RuntimeSettingsLayer>(() => (props.scope === 'workspace' ? 'workspace' : 'global'))
const workspaceId = computed(() => layer.value === 'workspace' && props.activeWorkspace !== true ? props.workspaceId : undefined)
const editable = computed(() => props.scope === 'global' || props.scope === 'workspace')
const sources = ref<SandboxSettingSources | null>(null)
const activeSources = ref<SandboxSettingSources | null>(null)
const loading = ref(false)
const saving = ref(false)
const error = ref('')
const visible = usePaneVisibility()
const busy = computed(() => props.disabled || loading.value || saving.value || !sources.value || !editable.value)
const localDrafts = ref(new Set<string>())
const pendingEdit = ref<{
  context: string
  key: SandboxSettingKey
  value: boolean | undefined
  source: 'inherit' | 'local'
} | null>(null)
const context = computed(() => JSON.stringify([props.scope, workspaceId.value]))
const configured = computed(() => {
  if (!editable.value) return sandboxEffectiveSettings(sources.value)
  const value = sandboxLayerSettings(sources.value, layer.value)
  for (const key of ['sandbox', 'resource_guard'] as const) {
    if (localDrafts.value.has(`${layer.value}:${key}`)) value[key] = undefined
    else if (layer.value !== 'global' && sourceMode(key) === 'inherit')
      value[key] = sandboxInheritedValue(sources.value, layer.value, key)
  }
  if (pendingEdit.value?.context === context.value) value[pendingEdit.value.key] = pendingEdit.value.value
  return value
})
const execution = computed(() => sandboxExecutionState(activeSources.value))
const executionLabel = computed(() =>
  execution.value.sandbox === undefined
    ? st('Unavailable')
    : execution.value.sandbox
      ? st('MXC sandbox enabled')
      : st('Direct execution'),
)
let readController: AbortController | undefined
let readSequence = 0
let disposed = false

const fields = computed(() => [
  {
    key: 'sandbox' as const,
    label: st('Native sandbox'),
    description: st('Run agent tools with MXC filesystem and network isolation.'),
    active: execution.value.sandbox,
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
  const target = workspaceId.value
  loading.value = true
  error.value = ''
  try {
    const keys = ['sandbox', 'resource_guard'] as const
    const [active, selected] = await Promise.all([
      Promise.all(keys.map((key) => readRuntimeSettingSources(key, controller.signal))),
      target
        ? Promise.all(keys.map((key) => getRuntimeSettingLayer('workspace', key, {
            workspaceId: target,
            signal: controller.signal,
          })))
        : undefined,
    ])
    if (controller.signal.aborted || sequence !== readSequence) return
    activeSources.value = { sandbox: active[0], resource_guard: active[1] }
    if (selected) {
      if (selected.some((field) => field.workspace_id !== Number(target)))
        throw new Error(st('The server did not return the selected workspace.'))
      sources.value = {
        sandbox: { ...active[0], workspace: selected[0] },
        resource_guard: { ...active[1], workspace: selected[1] },
      }
    } else sources.value = activeSources.value
  } catch (reason) {
    if (!controller.signal.aborted && sequence === readSequence) {
      if (target) sources.value = null
      error.value = reason instanceof Error ? reason.message : String(reason)
    }
  } finally {
    if (sequence === readSequence) {
      loading.value = false
      readController = undefined
    }
  }
}

async function persist(key: SandboxSettingKey, value: boolean | undefined) {
  if (busy.value) return
  const target = layer.value
  const editContext = context.value
  const options = {
    reload: true,
    workspace_id: workspaceId.value,
    expected_revision: sources.value?.[key][target].revision,
  }
  saving.value = true
  error.value = ''
  pendingEdit.value = {
    context: editContext,
    key,
    value: value ?? sandboxInheritedValue(sources.value, target, key),
    source: value === undefined ? 'inherit' : 'local',
  }
  try {
    if (value === undefined) await deleteRuntimeSetting(key, options, target)
    else await setRuntimeSetting(key, value, options, target)
    if (editContext === context.value) emit('saved')
  } catch (reason) {
    if (editContext === context.value) error.value = reason instanceof Error ? reason.message : String(reason)
    pendingEdit.value = null
    if (editContext !== context.value) {
      saving.value = false
      await refresh()
    }
    return
  } finally {
    saving.value = false
  }
  await refresh()
  if (editContext === context.value) localDrafts.value.delete(`${target}:${key}`)
  pendingEdit.value = null
}

function sourceMode(key: SandboxSettingKey): 'inherit' | 'local' | 'effective' {
  if (!editable.value) return 'effective'
  if (pendingEdit.value?.context === context.value && pendingEdit.value.key === key) return pendingEdit.value.source
  if (localDrafts.value.has(`${layer.value}:${key}`)) return 'local'
  return sandboxLayerOverride(sources.value, layer.value, key) === undefined ? 'inherit' : 'local'
}
function valueDisabled(key: SandboxSettingKey) {
  return busy.value || (layer.value !== 'global' && sourceMode(key) === 'inherit')
}
function save(key: SandboxSettingKey, value: boolean | undefined) {
  if (valueDisabled(key)) return
  return persist(key, value)
}
function chooseSource(key: SandboxSettingKey, mode: 'inherit' | 'local') {
  if (layer.value === 'global' && mode === 'local') return
  if (busy.value || sourceMode(key) === mode) return
  const inherited = sandboxInheritedValue(sources.value, layer.value, key)
  if (mode === 'local' && inherited === undefined) {
    localDrafts.value.add(`${layer.value}:${key}`)
    return
  }
  if (
    mode === 'inherit' &&
    localDrafts.value.delete(`${layer.value}:${key}`) &&
    sandboxLayerOverride(sources.value, layer.value, key) === undefined
  )
    return
  return persist(key, mode === 'local' ? inherited : undefined)
}
function inheritedLabel(key: SandboxSettingKey) {
  if (!sandboxInheritedKnown(sources.value, layer.value, key)) return st('Unavailable')
  const value = sandboxInheritedValue(sources.value, layer.value, key)
  return value === undefined ? st('No value') : value ? st('Enabled') : st('Disabled')
}
function configuredLabel(key: SandboxSettingKey) {
  const value = configured.value[key]
  if (value !== undefined) return value ? st('Enabled') : st('Disabled')
  if (localDrafts.value.has(`${layer.value}:${key}`)) return st('Select value')
  const resolved = sources.value?.[key]?.[layer.value]?.layer_value
  if (!editable.value && !sources.value) return st('Unavailable')
  if (resolved !== undefined) return resolved === null ? st('No value') : st('Invalid configuration')
  return sourceMode(key) === 'inherit' ? inheritedLabel(key) : st('Unavailable')
}

watch(context, () => {
  readController?.abort()
  readSequence++
  sources.value = null
  localDrafts.value.clear()
  loading.value = false
  error.value = ''
  void refresh()
}, { flush: 'sync' })
const unsubscribeSettings = subscribeRuntimeSettingsChanges(() => void refresh())
watch(
  visible,
  (value) => {
    if (value) void refresh()
  },
  { immediate: true },
)
onBeforeUnmount(() => {
  disposed = true
  unsubscribeSettings()
  readSequence++
  readController?.abort()
})
defineExpose({ refresh, saving })
</script>

<template>
  <SettingsCard
    :title="st('Sandbox & resource preflight')"
    :icon="RiShieldCheckLine"
    data-sandbox-settings
    :aria-busy="loading || saving"
  >
    <template #actions>
      <span v-if="saving || loading" class="text-xs text-muted-foreground" role="status">
        {{ saving ? st('Saving…') : st('Loading…') }}
      </span>
      <span v-else-if="activeSources && activeWorkspace !== false" class="text-xs text-muted-foreground" role="status">
        {{ executionLabel }}
      </span>
      <IconButton
        variant="ghost"
        size="sm"
        :disabled="loading || saving"
        :aria-label="st('Refresh')"
        :tooltip="st('Refresh')"
        @click="refresh"
      ><RiRefreshLine class="h-4 w-4" /></IconButton>
    </template>
    <div
      v-for="field in fields"
      :key="field.key"
      class="sandbox-setting-row"
      :data-sandbox-field="field.key"
    >
      <div class="min-w-0">
        <h3 class="text-[13px] font-medium">{{ field.label }}</h3>
        <p class="mt-0.5 text-xs leading-snug text-muted-foreground">{{ field.description }}</p>
        <p v-if="activeSources && activeWorkspace !== false && field.status" class="mt-0.5 text-xs text-muted-foreground">
          {{ field.status }}
        </p>
      </div>
      <div class="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
        <SettingsSourceControl
          v-if="editable"
          :model-value="sourceMode(field.key)"
          :scope="layer"
          :label="field.label"
          :inherited-value="inheritedLabel(field.key)"
          :inherited-source="layer === 'workspace' ? st('Global layer') : st('Built-in default')"
          :show-inherited-value="false"
          :disabled="busy"
          @update:model-value="chooseSource(field.key, $event)"
        />
        <template v-if="configured[field.key] !== undefined">
          <span class="text-[13px]" :data-active="activeWorkspace === false ? undefined : field.active">
            {{ configuredLabel(field.key) }}
          </span>
          <SettingsToggle
            v-if="editable"
            :model-value="configured[field.key] === true"
            :label="field.label"
            :disabled="valueDisabled(field.key)"
            @update:model-value="save(field.key, $event)"
          />
        </template>
        <OptionPicker
          v-else-if="editable && sources"
          model-value=""
          :options="[
            { value: 'true', label: st('Enabled') },
            { value: 'false', label: st('Disabled') },
          ]"
          :display-label="configuredLabel(field.key)"
          :title="field.label"
          :include-empty="false"
          :disabled="valueDisabled(field.key)"
          size="sm"
          @update:model-value="save(field.key, $event === 'true')"
        />
        <span v-else class="text-xs text-muted-foreground">{{ st('Unavailable') }}</span>
      </div>
    </div>
    <p v-if="execution.environmentDisabled && activeWorkspace !== false" class="mt-2 text-xs text-amber-700 dark:text-amber-300" role="status">
      {{ st('The runtime environment has disabled the sandbox. Remove that override to enable MXC.') }}
    </p>
    <p v-if="error" class="mt-2 break-words text-xs text-destructive" role="alert">{{ error }}</p>
  </SettingsCard>
</template>

<style scoped>
@reference '../../style.css';
.sandbox-setting-row {
  @apply border-border/50 border-b py-1.5 first:pt-0 last:border-b-0 last:pb-0;
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  min-width: 0;
  gap: 0.375rem 0.75rem;
  align-items: start;
}
@container settings-card (min-width: 30rem) {
  .sandbox-setting-row {
    grid-template-columns: minmax(0, 1fr) auto;
  }
}
</style>
