<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, shallowRef, useId, watch } from 'vue'
import { RiRobot2Line } from '@remixicon/vue'
import SettingsCard from './SettingsCard.vue'
import Button from '@/components/ui/Button.vue'
import {
  DEFAULT_PRIMARY_AGENT_PROFILE,
  DEFAULT_SUBAGENT_PROFILE,
  agentProfileCatalog,
  agentProfileDisplayName,
  mergedAgentProfiles,
  resolveAgentProfileDefault,
  selectableAgentProfiles,
  withBuiltinAgentProfiles,
  type AgentProfileCatalog,
} from '@/lib/agentProfiles'
import {
  readRuntimeSettingSources,
  setRuntimeSetting,
  subscribeRuntimeSettingsChanges,
  type RuntimeSettingsReadBundle,
} from '@/lib/runtimeSettings'
import { useDirectoryStore } from '@/stores/directory'
import { settingsText as st } from '@/i18n/settingsText'

const props = defineProps<{ scope: 'global' | 'workspace' | 'effective' }>()
const directory = useDirectoryStore()
const id = useId()
const inherit = ':inherit'
const fields = computed(
  () =>
    [
      {
        kind: 'primary',
        path: 'default_profile',
        label: st('Default profile for new sessions'),
        fallback: DEFAULT_PRIMARY_AGENT_PROFILE,
      },
      {
        kind: 'subagent',
        path: 'default_subagent_profile',
        label: st('Default profile for subagents'),
        fallback: DEFAULT_SUBAGENT_PROFILE,
      },
    ] as const,
)
type Field = (typeof fields.value)[number]
type Kind = Field['kind']
type Sources = Record<Kind, RuntimeSettingsReadBundle>

const sources = shallowRef<Sources | null>(null)
const catalog = shallowRef<AgentProfileCatalog>({})
const drafts = reactive<Partial<Record<Kind, string>>>({})
const loading = ref(false)
const saving = ref<Kind | null>(null)
const error = ref('')
const context = computed(() => JSON.stringify([props.scope, directory.currentDirectory]))
const dirty = computed(() => Object.keys(drafts).length > 0)
const busy = computed(() => loading.value || saving.value !== null)
let generation = 0
let disposed = false

function savedValue(kind: Kind) {
  const value = props.scope === 'effective' ? undefined : sources.value?.[kind][props.scope].value
  return typeof value === 'string' ? value : inherit
}
const rows = computed(() =>
  fields.value.map((field) => {
    const { kind, fallback } = field
    const bundle = sources.value?.[kind]
    const layer = props.scope === 'global' ? 'global' : 'workspace'
    const resolved = resolveAgentProfileDefault(bundle, layer, fallback)
    const effective = bundle?.effective.value
    const name = props.scope === 'effective' && typeof effective === 'string' ? effective || fallback : resolved.name
    return {
      ...field,
      name,
      source:
        resolved.source === 'global'
          ? st('Global')
          : resolved.source === 'workspace'
            ? st('Workspace')
            : st('Built-in'),
      selected: drafts[kind] ?? savedValue(kind),
      inherited: layer === 'global' ? fallback : resolveAgentProfileDefault(bundle, 'global', fallback).name,
      options: selectableAgentProfiles(catalog.value, kind === 'subagent'),
      dirty: drafts[kind] !== undefined,
    }
  }),
)

function selectDefault(kind: Kind, value: string) {
  if (value === savedValue(kind)) delete drafts[kind]
  else drafts[kind] = value
  error.value = ''
}

async function load() {
  const revision = ++generation
  loading.value = true
  error.value = ''
  try {
    const [profiles, builtins, primary, subagent] = await Promise.all([
      readRuntimeSettingSources('profiles'),
      readRuntimeSettingSources('builtin_profiles'),
      readRuntimeSettingSources('default_profile'),
      readRuntimeSettingSources('default_subagent_profile'),
    ])
    if (revision !== generation) return
    const global = agentProfileCatalog(profiles.global.value)
    const overrides =
      props.scope === 'global' ? global : mergedAgentProfiles(global, agentProfileCatalog(profiles.workspace.value))
    catalog.value = withBuiltinAgentProfiles(agentProfileCatalog(builtins.effective.value), overrides)
    sources.value = { primary, subagent }
    for (const { kind } of fields.value) {
      if (drafts[kind] === savedValue(kind)) delete drafts[kind]
    }
  } catch (reason) {
    if (revision === generation) error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    if (revision === generation) loading.value = false
  }
}

async function saveDefault(kind: Kind) {
  const layer = props.scope
  const bundle = sources.value?.[kind]
  const selected = drafts[kind]
  if (busy.value || layer === 'effective' || !bundle || selected === undefined) return
  const row = rows.value.find((row) => row.kind === kind)!
  const target = context.value
  saving.value = kind
  error.value = ''
  try {
    if (selected && selected !== inherit && !Object.hasOwn(row.options, selected))
      throw new Error(
        kind === 'primary'
          ? st('Choose a primary profile for new sessions.')
          : st('Choose a subagent profile for delegation.'),
      )
    await setRuntimeSetting(
      row.path,
      selected === inherit ? null : selected,
      { expected_revision: bundle[layer].revision },
      layer,
    )
    if (!disposed && target === context.value) await load()
  } catch (reason) {
    if (!disposed && target === context.value) error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    saving.value = null
  }
}

async function refresh() {
  if (!saving.value) await load()
}
const unsubscribe = subscribeRuntimeSettingsChanges(() => void refresh())
watch(context, () => {
  for (const { kind } of fields.value) delete drafts[kind]
  sources.value = null
  void load()
})
onMounted(() => void load())
onBeforeUnmount(() => {
  disposed = true
  generation++
  unsubscribe()
})
defineExpose({ refresh, dirty, saving })
</script>

<template>
  <SettingsCard
    id="settings-default-agent-profiles"
    :title="$st('Default profiles')"
    :icon="RiRobot2Line"
    :aria-busy="busy"
    data-testid="agent-profile-defaults"
  >
    <p v-if="error" class="mb-2 break-words text-xs text-destructive" role="alert">{{ error }}</p>
    <p v-if="loading && !sources" class="text-xs text-muted-foreground" role="status">{{ $st('Loading…') }}</p>
    <div
      v-for="row in sources ? rows : []"
      :key="row.kind"
      class="settings-field-row border-b border-border/60 py-1.5 first:pt-0 last:border-b-0 last:pb-0"
    >
      <component
        :is="scope === 'effective' ? 'span' : 'label'"
        :for="scope === 'effective' ? undefined : id + '-' + row.kind"
        class="text-[13px] font-medium"
      >
        {{ row.label }}
      </component>
      <div class="grid min-w-0 gap-1">
        <select
          v-if="scope !== 'effective'"
          :id="id + '-' + row.kind"
          :value="row.selected"
          :data-testid="'agent-profile-' + row.kind + '-default'"
          :disabled="busy"
          class="h-8 w-full min-w-0 rounded-md border border-input bg-background px-2 text-[13px] outline-none focus:border-ring disabled:opacity-60"
          @change="selectDefault(row.kind, ($event.target as HTMLSelectElement).value)"
        >
          <option :value="inherit">{{ $st('Inherit') }} · {{ row.inherited }}</option>
          <option value="">{{ $st('Built-in default') }} · {{ row.fallback }}</option>
          <option
            v-if="row.selected && row.selected !== inherit && !Object.hasOwn(row.options, row.selected)"
            :value="row.selected"
          >
            {{ row.selected }}
          </option>
          <option v-for="(profile, name) in row.options" :key="name" :value="name">
            {{ agentProfileDisplayName(name, profile) }} · {{ name }}
          </option>
        </select>
        <p class="text-xs text-muted-foreground">
          <template v-if="scope !== 'effective'">{{ $st('Effective profile') }} · </template>
          <strong
            :data-testid="'agent-profile-' + row.kind + '-effective'"
            class="font-mono font-medium text-foreground"
          >
            {{ row.name }}
          </strong>
          <template v-if="scope !== 'effective'"> · {{ row.source }}</template>
        </p>
        <div v-if="row.dirty" class="settings-toolbar justify-end">
          <Button size="sm" variant="ghost" :disabled="busy" @click="selectDefault(row.kind, savedValue(row.kind))">
            {{ $st('Discard changes') }}
          </Button>
          <Button
            :data-testid="'agent-profile-save-' + row.kind + '-default'"
            :aria-label="row.label + ' · ' + $st('Save default')"
            size="sm"
            :disabled="busy"
            @click="saveDefault(row.kind)"
          >
            {{ saving === row.kind ? $st('Saving…') : $st('Save') }}
          </Button>
        </div>
      </div>
    </div>
  </SettingsCard>
</template>
