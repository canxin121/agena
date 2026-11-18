<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { onBeforeRouteLeave, onBeforeRouteUpdate } from 'vue-router'
import { RiAddLine, RiDeleteBinLine, RiRefreshLine } from '@remixicon/vue'
import SettingsCard from './SettingsCard.vue'
import Button from '@/components/ui/Button.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import { confirmAction } from '@/lib/appConfirm'
import {
  agentProfileCatalog,
  agentProfileDisplayName,
  agentProfileInstructions,
  agentProfileLineage,
  agentProfileReferences,
  isValidAgentProfileName,
  mergedAgentProfiles,
  parseAgentProfileObject,
  withAgentProfileInstructions,
  type AgentProfileConfig,
} from '@/lib/agentProfiles'
import {
  normalizePromptLayer,
  normalizePromptScopeConfig,
  promptLayerHasErrors,
  validatePromptLayer,
} from '@/lib/promptLayers'
import { parsePermissionConfig, permissionConfigErrorMessage } from '@/lib/permissionConfig'
import { availableSavedPrompts, savedPromptDisplayName } from '@/lib/promptLibrary'
import { readRuntimeSettingSources, setRuntimeSetting, type RuntimeSettingsReadBundle } from '@/lib/runtimeSettings'
import { getSession, getSessionExecution, patchSessionMetadata } from '@/stores/chat/api'
import { useSettingsSessionTarget } from './settingsSessionScope'
import { useDirectoryStore } from '@/stores/directory'
import { useModelSelectionCatalog } from '@/pages/chat/modelSelectionCatalog'
import { encodeModelSelectionKey, parseModelSlug } from '@/pages/chat/modelSelectionDefaults'
import { settingsText as st } from '@/i18n/settingsText'

const props = defineProps<{ scope: string; sessionId?: string }>()
const directory = useDirectoryStore()
const modelCatalog = useModelSelectionCatalog()
const sessionTarget = useSettingsSessionTarget()
const sessionId = computed(() => props.sessionId || sessionTarget.sessionId.value)
const scope = computed(() =>
  props.scope === 'session' ? 'session' : props.scope === 'workspace' ? 'workspace' : 'global',
)
const sources = ref<RuntimeSettingsReadBundle | null>(null)
const defaults = ref<RuntimeSettingsReadBundle | null>(null)
const promptSources = ref<RuntimeSettingsReadBundle | null>(null)
const globalCatalog = computed(() => agentProfileCatalog(sources.value?.global.value))
const workspaceCatalog = computed(() => agentProfileCatalog(sources.value?.workspace.value))
const effectiveCatalog = computed(() => mergedAgentProfiles(globalCatalog.value, workspaceCatalog.value))
const globalSession = ref(false)
const catalog = computed(() =>
  scope.value === 'global' || (scope.value === 'session' && globalSession.value)
    ? globalCatalog.value
    : scope.value === 'workspace'
      ? workspaceCatalog.value
      : effectiveCatalog.value,
)
const visibleCatalog = computed(() =>
  scope.value === 'global' || (scope.value === 'session' && globalSession.value)
    ? globalCatalog.value
    : effectiveCatalog.value,
)
const entries = computed(() =>
  Object.entries(catalog.value).sort(([a, av], [b, bv]) =>
    agentProfileDisplayName(a, av).localeCompare(agentProfileDisplayName(b, bv)),
  ),
)
const selectedName = ref('')
const name = ref('')
const draft = ref<AgentProfileConfig | null>(null)
const promptJson = ref('{}')
const permissionJson = ref('{}')
const modelJson = ref('{}')
const modelKey = computed(() => {
  try {
    return encodeModelSelectionKey(parseAgentProfileObject(modelJson.value))
  } catch {
    return ''
  }
})
const modelOptions = computed(() => {
  const options = modelCatalog.providers.value
    .flatMap((provider) =>
      provider.models.flatMap((model) => {
        const adapter = String(model.adapter_id || '')
        const value = encodeModelSelectionKey({ provider: provider.id, adapter, model: model.id })
        return value
          ? [
              {
                value,
                label: String(model.display_name || model.id),
                description: [provider.id, adapter, model.id].filter(Boolean).join(' / '),
              },
            ]
          : []
      }),
    )
    .sort((a, b) => a.description.localeCompare(b.description))
  if (modelKey.value && !options.some((option) => option.value === modelKey.value))
    options.unshift({
      value: modelKey.value,
      label: parseModelSlug(modelKey.value).model,
      description: st('Configured value'),
    })
  return options
})
function chooseModel(value: string) {
  if (!value) {
    modelJson.value = '{}'
    return
  }
  const model = parseModelSlug(value)
  modelJson.value = JSON.stringify(
    { provider: model.provider, ...(model.adapter ? { adapter: model.adapter } : {}), model: model.model },
    null,
    2,
  )
}
const original = ref('')
const isNew = ref(false)
const defaultName = ref('__inherit__')
const sessionProfile = ref('')
const savedSessionProfile = ref('')
const sessionReadFailed = ref(false)
const loading = ref(false)
const saving = ref(false)
const error = ref('')
const selectedPrompt = ref('')
let generation = 0
const contextKey = computed(() => JSON.stringify([scope.value, directory.currentDirectory, sessionId.value]))
const snapshot = () =>
  JSON.stringify([name.value, draft.value, promptJson.value, permissionJson.value, modelJson.value])
const dirty = computed(() =>
  scope.value === 'session'
    ? sessionProfile.value !== savedSessionProfile.value
    : !!draft.value && (isNew.value || snapshot() !== original.value),
)
const savedPrompts = computed(() =>
  availableSavedPrompts(
    normalizePromptScopeConfig(promptSources.value?.global.value),
    normalizePromptScopeConfig(promptSources.value?.workspace.value),
    scope.value === 'global' ? 'global' : 'workspace',
  ),
)
const parsedPrompts = computed(() => {
  try {
    return parseAgentProfileObject(promptJson.value)
  } catch {
    return null
  }
})
const parsedPermissions = computed(() => {
  try {
    return parsePermissionConfig(parseAgentProfileObject(permissionJson.value))
  } catch {
    return null
  }
})
function pathPermission(field: 'workspace' | 'external'): string {
  return parsedPermissions.value?.path?.[field] || ''
}
function setPathPermission(field: 'workspace' | 'external', value: string) {
  if (!parsedPermissions.value) return
  const next = structuredClone(parsedPermissions.value)
  const path = { ...next.path }
  if (value) path[field] = value as 'readonly' | 'readwrite' | 'deny'
  else delete path[field]
  if (Object.keys(path).length) next.path = path
  else delete next.path
  permissionJson.value = JSON.stringify(next, null, 2)
}
const instructions = computed({
  get: () => agentProfileInstructions(name.value, { prompts: parsedPrompts.value || undefined }),
  set: (body: string) => {
    if (!parsedPrompts.value) return
    const updated = withAgentProfileInstructions(name.value, { prompts: parsedPrompts.value }, body)
    promptJson.value = JSON.stringify(updated.prompts, null, 2)
  },
})
const promptMode = computed({
  get: () => (parsedPrompts.value?.mode === 'replace' ? 'replace' : 'append'),
  set: (mode: string) => {
    if (parsedPrompts.value) promptJson.value = JSON.stringify({ ...parsedPrompts.value, mode }, null, 2)
  },
})
const inheritedDefault = computed(() =>
  String((scope.value === 'global' ? defaults.value?.global.value : defaults.value?.effective.value) || ''),
)
const defaultDirty = computed(() => {
  if (scope.value === 'session' || !defaults.value) return false
  const value = defaults.value[scope.value].value
  return defaultName.value !== (typeof value === 'string' ? value : '__inherit__')
})

function beginEditing(id: string, profile: AgentProfileConfig, creating = false) {
  selectedName.value = creating ? '' : id
  name.value = id
  draft.value = structuredClone(profile)
  promptJson.value = JSON.stringify(profile.prompts || {}, null, 2)
  permissionJson.value = JSON.stringify(profile.permission || {}, null, 2)
  modelJson.value = JSON.stringify(profile.model || {}, null, 2)
  isNew.value = creating
  original.value = snapshot()
  error.value = ''
}

async function canDiscard() {
  if (saving.value) return false
  if (!dirty.value && !defaultDirty.value) return true
  return confirmAction({
    title: st('Discard unsaved changes?'),
    description: st('You have unsaved changes.'),
    confirmText: st('Discard changes'),
    cancelText: st('Keep editing'),
  })
}

async function load(preferred = selectedName.value) {
  const revision = ++generation
  const requestedSession = scope.value === 'session' ? sessionId.value : ''
  loading.value = true
  error.value = ''
  let sessionError = ''
  try {
    const [profiles, defaultProfiles, prompts, session, summary] = await Promise.all([
      readRuntimeSettingSources('profiles'),
      readRuntimeSettingSources('default_profile'),
      readRuntimeSettingSources('prompts'),
      requestedSession
        ? getSessionExecution(requestedSession).catch((reason: unknown) => {
            sessionError = reason instanceof Error ? reason.message : String(reason)
            return null
          })
        : Promise.resolve(null),
      requestedSession ? getSession(requestedSession) : Promise.resolve(null),
      scope.value !== 'session' ? modelCatalog.loadProvidersAndModels() : Promise.resolve(),
    ])
    if (revision !== generation) return
    sources.value = profiles
    defaults.value = defaultProfiles
    promptSources.value = prompts
    sessionReadFailed.value = !!sessionError
    error.value = sessionError
    // Summary identity survives a missing profile that prevents execution resolution.
    globalSession.value = summary?.workspace_id === null
    const currentDefault = scope.value === 'session' ? null : defaultProfiles[scope.value].value
    defaultName.value = typeof currentDefault === 'string' ? currentDefault : '__inherit__'
    sessionProfile.value = typeof session?.execution?.profile === 'string' ? session.execution.profile : ''
    savedSessionProfile.value = sessionProfile.value
    const id = preferred && catalog.value[preferred] ? preferred : Object.keys(catalog.value)[0]
    if (id && catalog.value[id]) beginEditing(id, catalog.value[id]!)
    else {
      draft.value = null
      selectedName.value = ''
      isNew.value = false
    }
    error.value = sessionError
  } catch (reason) {
    if (revision === generation) {
      sources.value = null
      error.value = reason instanceof Error ? reason.message : String(reason)
    }
  } finally {
    if (revision === generation) loading.value = false
  }
}

async function selectProfile(id: string) {
  if (await canDiscard()) beginEditing(id, catalog.value[id]!)
}
async function addProfile() {
  if (!(await canDiscard())) return
  let id = 'profile'
  for (let index = 1; visibleCatalog.value[id]; index++) id = `profile-${index}`
  beginEditing(id, { title: '', description: '' }, true)
}
function attachSavedPrompt() {
  if (!selectedPrompt.value || !parsedPrompts.value) return
  const layer = normalizePromptLayer(parsedPrompts.value)
  if (!layer.order.includes(selectedPrompt.value)) layer.order.push(selectedPrompt.value)
  layer.disabled = layer.disabled.filter((id) => id !== selectedPrompt.value)
  promptJson.value = JSON.stringify(layer, null, 2)
  selectedPrompt.value = ''
}
function definition(): AgentProfileConfig {
  if (!draft.value || !isValidAgentProfileName(name.value))
    throw new Error(st('Use 1–64 lowercase letters, numbers, dots, underscores, or hyphens; do not start with a dot.'))
  if (isNew.value && Object.hasOwn(catalog.value, name.value))
    throw new Error(st('A profile with this name already exists.'))
  const profile: AgentProfileConfig = {
    ...draft.value,
    prompts: parseAgentProfileObject(promptJson.value),
    permission: parsePermissionConfig(parseAgentProfileObject(permissionJson.value)),
  }
  const model = parseAgentProfileObject(modelJson.value)
  if (Object.keys(model).length) profile.model = model
  else delete profile.model
  if (!profile.extends) delete profile.extends
  if (profile.max_turns === null || profile.max_turns === undefined || String(profile.max_turns) === '')
    delete profile.max_turns
  else if (!Number.isSafeInteger(profile.max_turns) || profile.max_turns < 0)
    throw new Error(st('Maximum turns must be a non-negative integer.'))
  if (promptLayerHasErrors(validatePromptLayer(normalizePromptLayer(profile.prompts), true)))
    throw new Error(st('Fix the prompt layer before saving.'))
  agentProfileLineage({ ...visibleCatalog.value, [name.value]: profile }, name.value)
  return profile
}

async function saveProfile() {
  if (saving.value || scope.value === 'session' || !sources.value) return
  const source = scope.value
  const context = contextKey.value
  const pendingDefault = defaultDirty.value ? defaultName.value : undefined
  saving.value = true
  error.value = ''
  try {
    const profile = definition()
    await setRuntimeSetting(
      'profiles',
      { ...catalog.value, [name.value]: profile },
      { expected_revision: sources.value[source].revision },
      source,
    )
    if (context === contextKey.value) {
      await load(name.value)
      if (pendingDefault !== undefined) defaultName.value = pendingDefault
    }
  } catch (reason) {
    if (context === contextKey.value) error.value = permissionConfigErrorMessage(reason, st)
  } finally {
    saving.value = false
  }
}
async function saveDefault() {
  if (saving.value || scope.value === 'session' || !defaults.value) return
  const source = scope.value
  const context = contextKey.value
  saving.value = true
  error.value = ''
  try {
    await setRuntimeSetting(
      'default_profile',
      defaultName.value === '__inherit__' ? null : defaultName.value,
      { expected_revision: defaults.value[source].revision },
      source,
    )
    if (context === contextKey.value) await load()
  } catch (reason) {
    if (context === contextKey.value) error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    saving.value = false
  }
}
async function removeProfile() {
  if (saving.value || scope.value === 'session' || !sources.value || !selectedName.value) return
  if (
    agentProfileReferences(visibleCatalog.value, selectedName.value).length ||
    inheritedDefault.value === selectedName.value
  ) {
    error.value = st('Choose another default and update inherited profiles before deleting this profile.')
    return
  }
  if (
    !(await confirmAction({
      title: st('Delete agent profile?'),
      description: st('Sessions using this profile will need another profile before they can continue.'),
      confirmText: st('Delete'),
      cancelText: st('Cancel'),
    }))
  )
    return
  const source = scope.value
  const context = contextKey.value
  saving.value = true
  error.value = ''
  try {
    const next = { ...catalog.value }
    delete next[selectedName.value]
    await setRuntimeSetting('profiles', next, { expected_revision: sources.value[source].revision }, source)
    if (context === contextKey.value) await load('')
  } catch (reason) {
    if (context === contextKey.value) error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    saving.value = false
  }
}
async function saveSession() {
  if (saving.value || !sessionId.value) return
  const context = contextKey.value
  const target = sessionId.value
  saving.value = true
  error.value = ''
  try {
    await patchSessionMetadata(target, { profile: sessionProfile.value })
    if (context === contextKey.value) await load()
  } catch (reason) {
    if (context === contextKey.value) error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    saving.value = false
  }
}
async function refresh() {
  if (await canDiscard()) await load()
}
defineExpose({ refresh })
watch(contextKey, () => {
  void load('')
})
onMounted(() => {
  void load('')
})
onBeforeUnmount(() => {
  generation++
})
onBeforeRouteLeave(canDiscard)
onBeforeRouteUpdate(canDiscard)
</script>

<template>
  <div class="grid min-w-0 gap-4" :aria-busy="loading || saving">
    <SettingsCard
      :title="$st('Agent profiles')"
      :description="$st('Reuse permissions, system instructions, and model preferences across sessions.')"
    >
      <template #actions
        ><Button size="sm" variant="ghost" :disabled="loading || saving" @click="refresh"
          ><RiRefreshLine class="h-4 w-4" /><span class="sr-only">{{ $st('Refresh') }}</span></Button
        ></template
      >
      <p
        v-if="error"
        class="mb-3 break-words rounded-md border border-destructive/40 p-3 text-sm text-destructive"
        role="alert"
      >
        {{ error }}
      </p>
      <p v-if="loading" role="status" class="text-sm text-muted-foreground">{{ $st('Loading…') }}</p>
      <template v-else-if="sources">
        <div v-if="scope === 'session'" class="grid gap-3">
          <p v-if="!sessionId" class="text-sm text-muted-foreground">
            {{ $st('Open a conversation to select its agent profile.') }}
          </p>
          <template v-else>
            <label class="grid gap-1 text-sm"
              >{{ $st('Current session profile') }}
              <select
                v-model="sessionProfile"
                :disabled="saving"
                class="rounded-md border border-input bg-background px-3 py-2"
              >
                <option value="">{{ $st('No profile') }}</option>
                <option v-if="sessionProfile && !visibleCatalog[sessionProfile]" :value="sessionProfile">
                  {{ sessionProfile }}
                </option>
                <option v-for="[id, profile] in entries" :key="id" :value="id">
                  {{ agentProfileDisplayName(id, profile) }} · {{ id }}
                </option>
              </select>
            </label>
            <p v-if="sessionProfile" class="text-sm text-muted-foreground">
              {{ visibleCatalog[sessionProfile]?.description }}
            </p>
            <p class="text-xs text-muted-foreground">
              {{ $st('Session overrides remain in effect. Profile edits apply to future runs.') }}
            </p>
            <div>
              <Button :disabled="saving || (!dirty && !sessionReadFailed)" @click="saveSession">{{
                saving ? $st('Saving…') : $st('Apply profile')
              }}</Button>
            </div>
          </template>
        </div>
        <template v-else>
          <div class="mb-5 flex flex-wrap items-end gap-3 border-b border-border/60 pb-4">
            <label class="grid min-w-52 flex-1 gap-1 text-sm"
              >{{ $st('Default profile for new sessions') }}
              <select
                v-model="defaultName"
                :disabled="saving"
                class="rounded-md border border-input bg-background px-3 py-2"
              >
                <option value="__inherit__">{{ $st('Inherit') }}</option>
                <option value="">{{ $st('No profile') }}</option>
                <option v-for="(profile, id) in visibleCatalog" :key="id" :value="id">
                  {{ agentProfileDisplayName(id, profile) }} · {{ id }}
                </option>
              </select>
            </label>
            <Button size="sm" variant="outline" :disabled="saving || !defaultDirty || dirty" @click="saveDefault">{{
              $st('Save default')
            }}</Button>
          </div>
          <div class="grid min-w-0 gap-5 md:grid-cols-[minmax(11rem,0.7fr)_minmax(0,1.7fr)]">
            <div class="grid min-w-0 content-start gap-2">
              <Button variant="outline" size="sm" :disabled="saving" @click="addProfile"
                ><RiAddLine class="mr-1 h-4 w-4" />{{ $st('New profile') }}</Button
              >
              <button
                v-for="[id, profile] in entries"
                :key="id"
                type="button"
                :disabled="saving"
                class="min-w-0 rounded-md border px-3 py-2 text-left hover:bg-muted/50"
                :class="selectedName === id ? 'border-primary/50 bg-primary/10' : 'border-border/60'"
                @click="selectProfile(id)"
              >
                <span class="block truncate text-sm font-medium">{{ agentProfileDisplayName(id, profile) }}</span>
                <span class="block truncate text-xs text-muted-foreground">{{ id }}</span>
                <span v-if="profile.description" class="mt-1 block text-xs text-muted-foreground">{{
                  profile.description
                }}</span>
              </button>
              <p v-if="!entries.length" class="text-sm text-muted-foreground">
                {{ $st('Create a profile to reuse your agent configuration.') }}
              </p>
            </div>
            <div v-if="draft" class="grid min-w-0 content-start gap-3">
              <div class="grid gap-3 sm:grid-cols-2">
                <label class="grid gap-1 text-xs text-muted-foreground"
                  >{{ $st('Profile name')
                  }}<input
                    v-model="name"
                    :disabled="saving || !isNew"
                    spellcheck="false"
                    class="rounded-md border border-input bg-background px-3 py-2 font-mono text-sm text-foreground"
                /></label>
                <label class="grid gap-1 text-xs text-muted-foreground"
                  >{{ $st('Title')
                  }}<input
                    v-model="draft.title"
                    :disabled="saving"
                    class="rounded-md border border-input bg-background px-3 py-2 text-sm text-foreground"
                /></label>
              </div>
              <label class="grid gap-1 text-xs text-muted-foreground"
                >{{ $st('Description')
                }}<textarea
                  v-model="draft.description"
                  :disabled="saving"
                  rows="2"
                  class="rounded-md border border-input bg-background px-3 py-2 text-sm text-foreground"
                />
              </label>
              <label class="grid gap-1 text-xs text-muted-foreground"
                >{{ $st('Inherit from profile') }}
                <select
                  :value="draft.extends || ''"
                  :disabled="saving"
                  class="rounded-md border border-input bg-background px-3 py-2 text-sm text-foreground"
                  @change="draft.extends = ($event.target as HTMLSelectElement).value || null"
                >
                  <option value="">{{ $st('None') }}</option>
                  <option v-for="(profile, id) in visibleCatalog" :key="id" :value="id" :disabled="id === name">
                    {{ agentProfileDisplayName(id, profile) }} · {{ id }}
                  </option>
                </select>
              </label>
              <label class="grid gap-1 text-xs text-muted-foreground"
                >{{ $st('Instructions')
                }}<textarea
                  v-model="instructions"
                  :disabled="saving || !parsedPrompts"
                  rows="8"
                  class="min-h-36 w-full resize-y rounded-md border border-input bg-background px-3 py-2 text-sm text-foreground"
                />
              </label>
              <label class="grid min-w-0 gap-1 text-xs text-muted-foreground"
                >{{ $st('Model route') }}
                <OptionPicker
                  :model-value="modelKey"
                  :options="modelOptions"
                  :title="$st('Model route')"
                  :empty-label="$st('Inherit model selection')"
                  :search-placeholder="$st('Search configured models...')"
                  :disabled="saving"
                  @update:model-value="chooseModel"
                />
              </label>
              <div class="flex flex-wrap gap-2">
                <label class="grid gap-1 text-xs text-muted-foreground"
                  >{{ $st('Layer mode')
                  }}<select
                    v-model="promptMode"
                    :disabled="saving || !parsedPrompts"
                    class="rounded-md border border-input bg-background px-3 py-2 text-sm text-foreground"
                  >
                    <option value="append">{{ $st('Append') }}</option>
                    <option value="replace">{{ $st('Replace') }}</option>
                  </select></label
                >
                <label class="grid min-w-32 flex-1 gap-1 text-xs text-muted-foreground"
                  >{{ $st('Saved prompts')
                  }}<select
                    v-model="selectedPrompt"
                    :disabled="saving || !parsedPrompts"
                    class="rounded-md border border-input bg-background px-3 py-2 text-sm text-foreground"
                  >
                    <option value="">{{ $st('Select a saved prompt') }}</option>
                    <option v-for="option in savedPrompts" :key="option.id" :value="option.id">
                      {{ savedPromptDisplayName(option.fragment) }}
                    </option>
                  </select></label
                >
                <Button
                  class="self-end"
                  size="sm"
                  variant="outline"
                  :disabled="saving || !selectedPrompt"
                  @click="attachSavedPrompt"
                  >{{ $st('Add') }}</Button
                >
              </div>
              <div class="grid gap-3 sm:grid-cols-2">
                <label class="grid gap-1 text-xs text-muted-foreground"
                  >{{ $st('Workspace files') }}
                  <select
                    :value="pathPermission('workspace')"
                    :disabled="saving || !parsedPermissions"
                    class="rounded-md border border-input bg-background px-3 py-2 text-sm text-foreground"
                    @change="setPathPermission('workspace', ($event.target as HTMLSelectElement).value)"
                  >
                    <option value="">{{ $st('Inherit') }}</option>
                    <option value="readonly">{{ $st('Read only') }}</option>
                    <option value="readwrite">{{ $st('Read and write') }}</option>
                    <option value="deny">{{ $st('Deny') }}</option>
                  </select>
                </label>
                <label class="grid gap-1 text-xs text-muted-foreground"
                  >{{ $st('External files') }}
                  <select
                    :value="pathPermission('external')"
                    :disabled="saving || !parsedPermissions"
                    class="rounded-md border border-input bg-background px-3 py-2 text-sm text-foreground"
                    @change="setPathPermission('external', ($event.target as HTMLSelectElement).value)"
                  >
                    <option value="">{{ $st('Inherit') }}</option>
                    <option value="readonly">{{ $st('Read only') }}</option>
                    <option value="readwrite">{{ $st('Read and write') }}</option>
                    <option value="deny">{{ $st('Deny') }}</option>
                  </select>
                </label>
              </div>
              <label class="grid gap-1 text-xs text-muted-foreground"
                >{{ $st('Permissions (JSON)')
                }}<textarea
                  v-model="permissionJson"
                  :disabled="saving"
                  spellcheck="false"
                  rows="7"
                  class="w-full resize-y rounded-md border border-input bg-background px-3 py-2 font-mono text-xs text-foreground"
                />
              </label>
              <p class="text-xs text-muted-foreground">
                {{ $st('Use path, network, and tools rules. An empty object inherits permissions.') }}
              </p>
              <details class="rounded-md border border-border/60 p-3">
                <summary class="cursor-pointer text-sm">{{ $st('Model, limits, and prompt layer') }}</summary>
                <div class="mt-3 grid gap-3">
                  <label class="grid gap-1 text-xs text-muted-foreground"
                    >{{ $st('Model selection (JSON)')
                    }}<textarea
                      v-model="modelJson"
                      :disabled="saving"
                      spellcheck="false"
                      rows="5"
                      class="w-full resize-y rounded-md border border-input bg-background px-3 py-2 font-mono text-xs text-foreground"
                    />
                  </label>
                  <p class="text-xs text-muted-foreground">
                    {{
                      $st('Use provider, adapter, model, and response preferences. An empty object inherits the model.')
                    }}
                  </p>
                  <label class="grid gap-1 text-xs text-muted-foreground"
                    >{{ $st('Maximum turns')
                    }}<input
                      v-model.number="draft.max_turns"
                      :disabled="saving"
                      type="number"
                      min="0"
                      step="1"
                      :placeholder="$st('Inherit')"
                      class="rounded-md border border-input bg-background px-3 py-2 text-sm text-foreground"
                  /></label>
                  <p class="text-xs text-muted-foreground">
                    {{ $st('Leave empty to inherit; 0 allows unlimited turns.') }}
                  </p>
                  <label class="grid gap-1 text-xs text-muted-foreground"
                    >{{ $st('Prompt layer (JSON)')
                    }}<textarea
                      v-model="promptJson"
                      :disabled="saving"
                      spellcheck="false"
                      rows="10"
                      class="w-full resize-y rounded-md border border-input bg-background px-3 py-2 font-mono text-xs text-foreground"
                    />
                  </label>
                </div>
              </details>
              <div class="flex items-center justify-between gap-3">
                <Button variant="ghost" color="danger" :disabled="saving || isNew" @click="removeProfile"
                  ><RiDeleteBinLine class="mr-1 h-4 w-4" />{{ $st('Delete') }}</Button
                >
                <Button :disabled="saving || !dirty" @click="saveProfile">{{
                  saving ? $st('Saving…') : $st('Save changes')
                }}</Button>
              </div>
            </div>
          </div>
        </template>
      </template>
    </SettingsCard>
  </div>
</template>
