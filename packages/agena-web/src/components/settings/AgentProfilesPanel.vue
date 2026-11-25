<script setup lang="ts">
import { useSettingsDraftGuard } from './useSettingsDraftGuard'
import { computed, defineAsyncComponent, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { RouterLink, useRoute } from 'vue-router'
import { RiAddLine, RiCpuLine, RiDeleteBinLine, RiRefreshLine, RiResetLeftLine, RiRobot2Line } from '@remixicon/vue'
import SettingsCard from './SettingsCard.vue'
import SettingsDisclosureRow from './SettingsDisclosureRow.vue'
import SettingsSaveBar from './SettingsSaveBar.vue'
import AgentProfileModelFields from './AgentProfileModelFields.vue'
import PromptReferenceList from './PromptReferenceList.vue'
import { readBuiltinPrompts, reusableBuiltinPrompts } from './promptCatalog'
import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import Input from '@/components/ui/Input.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import SegmentedControl from '@/components/ui/SegmentedControl.vue'
import SegmentedButton from '@/components/ui/SegmentedButton.vue'
import { confirmAction } from '@/lib/appConfirm'
import {
  DEFAULT_PRIMARY_AGENT_PROFILE,
  DEFAULT_SUBAGENT_PROFILE,
  agentProfileCatalog,
  agentProfileDisplayName,
  agentProfileInstructions,
  agentProfileLineage,
  agentProfileMetadata,
  selectableAgentProfiles,
  agentProfileReferences,
  isValidAgentProfileName,
  mergeAgentProfilePermission,
  mergedAgentProfiles,
  withBuiltinAgentProfiles,
  parseAgentProfileObject,
  resolveAgentProfileDefault,
  withAgentProfileInstructions,
  type AgentIsolation,
  type AgentProfileConfig,
  type ProjectInstructionsMode,
} from '@/lib/agentProfiles'
import {
  normalizePromptLayer,
  normalizePromptScopeConfig,
  promptLayerHasErrors,
  validatePromptLayer,
  type BuiltinPrompt,
  type PromptFragment,
} from '@/lib/promptLayers'
import {
  completePermissionConfig,
  parsePermissionConfig,
  permissionConfigErrorMessage,
  type PermissionConfig,
} from '@/lib/permissionConfig'
import { availableSavedPrompts } from '@/lib/promptLibrary'
import { permissionProfileDisplayName, permissionProfilesForLayer } from '@/lib/permissionProfiles'
import { createSkillsClient, type SkillCatalog } from '@/lib/skillsApi'
import { readRuntimeSettingSources, setRuntimeSetting, type RuntimeSettingsReadBundle } from '@/lib/runtimeSettings'
import { runtimeSettingBaseValue } from '@/lib/runtimeSettingLayer'
import { getSession, getSessionExecution, patchSessionMetadata, resolveWorkspace } from '@/stores/chat/api'
import { useSettingsSessionTarget } from './settingsSessionScope'
import { agentProfileDefaultsDestination } from './settingsDestination'
import { useDirectoryStore } from '@/stores/directory'
import { useModelSelectionCatalog } from '@/pages/chat/modelSelectionCatalog'
import type { JsonObject } from '@/types/json'
import { settingsText as st } from '@/i18n/settingsText'

const props = defineProps<{ scope: string; sessionId?: string; createForBot?: boolean }>()
const emit = defineEmits<{
  'workspace-config-path': [path: string]
  'profile-created': [name: string]
  'cancel-create': []
}>()
const directory = useDirectoryStore()
const route = useRoute()
const modelCatalog = useModelSelectionCatalog()
const sessionTarget = useSettingsSessionTarget()
const sessionId = computed(() => props.sessionId || sessionTarget.sessionId.value)
const scope = computed(() =>
  props.scope === 'session' ? 'session' : props.scope === 'workspace' ? 'workspace' : 'global',
)
const sources = ref<RuntimeSettingsReadBundle | null>(null)
const defaults = ref<RuntimeSettingsReadBundle | null>(null)
const subagentDefaults = ref<RuntimeSettingsReadBundle | null>(null)
const builtinSources = ref<RuntimeSettingsReadBundle | null>(null)
const promptSources = ref<RuntimeSettingsReadBundle | null>(null)
const promptCatalog = ref<BuiltinPrompt[]>([])
const permissionSources = ref<RuntimeSettingsReadBundle | null>(null)
const permissionCatalog = ref<RuntimeSettingsReadBundle | null>(null)
const completePermissions = computed(() =>
  Object.entries(permissionProfilesForLayer(permissionCatalog.value?.global)).filter(([, profile]) => profile.complete),
)
const builtinCatalog = computed(() => agentProfileCatalog(builtinSources.value?.effective.value))
const globalCatalog = computed(() => agentProfileCatalog(sources.value?.global.value))
const workspaceCatalog = computed(() => agentProfileCatalog(sources.value?.workspace.value))
const globalEffectiveCatalog = computed(() => withBuiltinAgentProfiles(builtinCatalog.value, globalCatalog.value))
const effectiveCatalog = computed(() =>
  withBuiltinAgentProfiles(builtinCatalog.value, mergedAgentProfiles(globalCatalog.value, workspaceCatalog.value)),
)
const globalSession = ref(false)
const subagentSession = ref(false)
const botSession = ref(false)
const visibleCatalog = computed(() =>
  scope.value === 'global' || (scope.value === 'session' && globalSession.value)
    ? globalEffectiveCatalog.value
    : effectiveCatalog.value,
)
const catalog = visibleCatalog
const rawCatalog = computed(() => (scope.value === 'workspace' ? workspaceCatalog.value : globalCatalog.value))
const inheritedCatalog = computed(() =>
  scope.value === 'workspace' ? globalEffectiveCatalog.value : builtinCatalog.value,
)
const sessionCatalog = computed(() => selectableAgentProfiles(visibleCatalog.value, subagentSession.value))
const entries = computed(() =>
  Object.entries(scope.value === 'session' ? sessionCatalog.value : catalog.value).sort(([a, av], [b, bv]) =>
    agentProfileDisplayName(a, av).localeCompare(agentProfileDisplayName(b, bv)),
  ),
)
function profileSource(id: string): string {
  if (
    scope.value !== 'global' &&
    !(scope.value === 'session' && globalSession.value) &&
    Object.hasOwn(workspaceCatalog.value, id)
  )
    return st('Workspace')
  if (Object.hasOwn(globalCatalog.value, id)) return st('Global')
  return st('Built-in')
}
function profileModeLabel(id: string): string {
  try {
    const { mode, readOnly } = agentProfileMetadata(visibleCatalog.value, id)
    const label =
      mode === 'primary'
        ? st('Primary sessions')
        : mode === 'subagent'
          ? st('Subagents')
          : st('Primary sessions and subagents')
    return readOnly ? `${label} · ${st('Read only')}` : label
  } catch {
    return ''
  }
}
const selectedName = ref('')
const name = ref('')
const draft = ref<AgentProfileConfig | null>(null)
const readOnlyAccess = computed({
  get: () => (draft.value?.read_only == null ? '' : String(draft.value.read_only)),
  set: (value: string) => {
    if (draft.value) draft.value.read_only = value === '' ? undefined : value === 'true'
  },
})
const inheritedReadOnly = computed(() => {
  const builtin = builtinCatalog.value[name.value]?.read_only
  if (builtin != null) return builtin
  const parent = draft.value?.extends
  try {
    return parent ? agentProfileMetadata(visibleCatalog.value, parent).readOnly : false
  } catch {
    return false
  }
})
const draftReadOnly = computed(() => draft.value?.read_only ?? inheritedReadOnly.value)
const isolation = computed({
  get: () => (draftReadOnly.value ? 'shared' : draft.value?.isolation || ''),
  set: (value: AgentIsolation | '') => {
    if (draft.value) draft.value.isolation = value || undefined
  },
})
const projectInstructions = computed({
  get: () => draft.value?.context?.project_instructions || '',
  set: (value: ProjectInstructionsMode | '') => {
    if (draft.value) draft.value.context = { ...draft.value.context, project_instructions: value || undefined }
  },
})
const skillCatalog = ref<SkillCatalog | null>(null)
const skillOptions = computed<PromptFragment[]>(() => {
  const found = new Map<string, PromptFragment>()
  for (const skill of skillCatalog.value?.skills || []) {
    if (!skill.enabled || !skill.valid) continue
    if (scope.value === 'global' ? skill.scope !== 'global' : skill.shadowed) continue
    if (found.has(skill.name)) continue
    found.set(skill.name, {
      id: skill.name,
      title: skill.name,
      category: skill.scope === 'global' ? st('Global') : st('Workspace'),
      body: skill.description,
      when: { models: [], requires_tools: null },
    })
  }
  return [...found.values()].sort((a, b) => a.id.localeCompare(b.id))
})
function inheritsPreloadedSkills(inherits: boolean) {
  if (!draft.value) return
  const context = { ...draft.value.context }
  if (inherits) {
    delete context.preload_skills
  } else {
    const profile = visibleCatalog.value[selectedName.value]
    context.preload_skills = [...(profile?.context?.preload_skills || [])]
  }
  draft.value.context = context
}
const promptJson = ref('{}')
const permission = ref<PermissionConfig>({})
const modelJson = ref('{}')
const parsedModel = computed(() => {
  try {
    return parseAgentProfileObject(modelJson.value)
  } catch {
    return null
  }
})
const approvalModel = computed<JsonObject>(() => {
  const value = permission.value.approval_model
  return value && typeof value === 'object' && !Array.isArray(value) ? value : {}
})
function setApprovalModel(value: JsonObject) {
  const next = { ...permission.value }
  if (Object.keys(value).length) next.approval_model = value
  else delete next.approval_model
  permission.value = next
}
const PermissionStudioPanel = defineAsyncComponent(() => import('./PermissionStudioPanel.vue'))
type PermissionEditor = { pendingRule: boolean; applyPendingRule: () => boolean }
const permissionEditors = ref<PermissionEditor[]>([])
const permissionOpened = ref(false)
const editorKey = ref(0)
const editorSections = computed(
  () =>
    [
      { id: 'profile', label: st('Profile configuration') },
      { id: 'instructions', label: st('Instructions') },
      { id: 'model', label: st('Model & limits') },
      { id: 'permissions', label: st('Permissions') },
    ] as const,
)
type EditorSection = (typeof editorSections.value)[number]['id']
const editorSection = ref<EditorSection>('profile')
function showEditorSection(section: EditorSection) {
  editorSection.value = section
  if (section === 'permissions') permissionOpened.value = true
}
const original = ref('')
const isNew = ref(false)
const effectiveSessionProfile = ref('')
const sessionProfile = ref('')
const savedSessionProfile = ref('')
const sessionReadFailed = ref(false)
const loading = ref(false)
const saving = ref(false)
const error = ref('')
const query = ref('')
let generation = 0
const contextKey = computed(() => JSON.stringify([scope.value, scope.value === 'session' ? sessionId.value : '']))
const snapshot = () => JSON.stringify([name.value, draft.value, promptJson.value, permission.value, modelJson.value])
const dirty = computed(() =>
  scope.value === 'session'
    ? sessionProfile.value !== savedSessionProfile.value
    : !!draft.value &&
      (isNew.value || snapshot() !== original.value || permissionEditors.value[0]?.pendingRule === true),
)
const profileRows = computed(() => {
  const search = query.value.trim().toLowerCase()
  const rows = (props.createForBot ? [] : entries.value)
    .filter(([id, profile]) => {
      if (id === selectedName.value && dirty.value) return true
      return [id, profile.title, profile.description, profileModeLabel(id)].some((value) =>
        value?.toLowerCase().includes(search),
      )
    })
    .map(([id, profile]) => ({
      key: id,
      id,
      label: agentProfileDisplayName(id, profile),
      summary: [id, profileSource(id), profileModeLabel(id)].filter(Boolean).join(' · '),
      open: Boolean(draft.value && !isNew.value && id === selectedName.value),
      isNew: false,
    }))
  if (isNew.value && draft.value)
    rows.unshift({
      key: '',
      id: '',
      label: draft.value.title || st('New profile'),
      summary: name.value,
      open: true,
      isNew: true,
    })
  return rows
})
const savedPrompts = computed(() =>
  availableSavedPrompts(
    normalizePromptScopeConfig(promptSources.value?.global.value),
    normalizePromptScopeConfig(promptSources.value?.workspace.value),
    scope.value === 'global' ? 'global' : 'workspace',
    reusableBuiltinPrompts(promptCatalog.value),
  )
    .map(({ fragment }) => fragment)
    .filter((fragment) => !normalizePromptLayer(parsedPrompts.value).inline.some(({ id }) => id === fragment.id)),
)
const parsedPrompts = computed(() => {
  try {
    return parseAgentProfileObject(promptJson.value)
  } catch {
    return null
  }
})
const attachedPrompts = computed(() => {
  const layer = normalizePromptLayer(parsedPrompts.value)
  return layer.order.filter((id) => !layer.inline.some((fragment) => fragment.id === id))
})
const namedPermission = computed(
  () => completePermissions.value.find(([id]) => id === draft.value?.permission_profile)?.[1],
)
const inheritedPermission = computed<PermissionConfig | null>(() => {
  if (!draft.value || scope.value === 'session') return null
  const layer = scope.value
  const policy = permissionSources.value?.[layer]
  try {
    let result = policy?.layer_value
    if (result === undefined) {
      result = mergeAgentProfilePermission(runtimeSettingBaseValue(policy, layer) ?? {}, policy?.value ?? {})
    }
    const parent = draft.value.extends
    if (parent)
      for (const ancestor of agentProfileLineage(catalog.value, parent)) {
        if (ancestor === name.value) return null
        const role = catalog.value[ancestor]
        if (role?.permission_profile) {
          const selected = completePermissions.value.find(([id]) => id === role.permission_profile)?.[1]
          if (!selected) return null
          result = completePermissionConfig(selected.permission)
        }
        result = mergeAgentProfilePermission(result ?? {}, role?.permission ?? {})
      }
    const builtin = builtinCatalog.value[name.value]?.permission
    if (builtin) result = mergeAgentProfilePermission(result ?? {}, builtin)
    return parsePermissionConfig(result ?? {})
  } catch {
    return null
  }
})
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
const defaultConfigScope = computed(() =>
  scope.value === 'global' || (scope.value === 'session' && globalSession.value) ? 'global' : 'workspace',
)
const primaryDefault = computed(
  () => resolveAgentProfileDefault(defaults.value, defaultConfigScope.value, DEFAULT_PRIMARY_AGENT_PROFILE).name,
)
const subagentDefault = computed(
  () => resolveAgentProfileDefault(subagentDefaults.value, defaultConfigScope.value, DEFAULT_SUBAGENT_PROFILE).name,
)
const restoreSessionProfile = computed(() => {
  const defaultRole =
    (!savedSessionProfile.value && effectiveSessionProfile.value) ||
    (subagentSession.value ? subagentDefault.value : botSession.value ? '' : primaryDefault.value)
  return [st('Restore default role'), defaultRole].filter(Boolean).join(' · ')
})
function defaultSettingsDestination(layer: 'global' | 'workspace') {
  return agentProfileDefaultsDestination(layer, route.query)
}
const hasOwnProfile = computed(() => !!selectedName.value && Object.hasOwn(rawCatalog.value, selectedName.value))
const restoresInherited = computed(
  () => !!selectedName.value && Object.hasOwn(inheritedCatalog.value, selectedName.value),
)

function beginEditing(id: string, profile: AgentProfileConfig, creating = false) {
  const authored = Object.hasOwn(rawCatalog.value, id)
    ? rawCatalog.value[id]
    : scope.value === 'workspace' && Object.hasOwn(globalCatalog.value, id)
      ? globalCatalog.value[id]
      : undefined
  selectedName.value = creating ? '' : id
  name.value = id
  const { permission: _, context, isolation, read_only, ...fields } = structuredClone(profile)
  draft.value = {
    ...fields,
    context: creating ? context : structuredClone(authored?.context || {}),
    isolation: creating ? isolation : authored?.isolation,
    read_only: creating ? read_only : authored?.read_only,
  }
  promptJson.value = JSON.stringify(profile.prompts || {}, null, 2)
  // A profile replacement keeps the settings from its source file. Built-in
  // permissions remain inherited, so editing a title does not pin them.
  permission.value = structuredClone((creating ? profile.permission : authored?.permission) || {})
  modelJson.value = JSON.stringify(profile.model || {}, null, 2)
  permissionOpened.value = false
  editorSection.value = 'profile'
  editorKey.value++
  isNew.value = creating
  original.value = snapshot()
  error.value = ''
}
function closeProfile() {
  draft.value = null
  selectedName.value = ''
  isNew.value = false
  error.value = ''
}
function discardProfile() {
  if (saving.value) return
  if (props.createForBot) {
    emit('cancel-create')
    return
  }
  const section = editorSection.value
  const selected = catalog.value[selectedName.value]
  if (isNew.value || !selected) closeProfile()
  else {
    beginEditing(selectedName.value, selected)
    showEditorSection(section)
  }
}

const { canDiscard } = useSettingsDraftGuard({
  dirty: () => dirty.value,
  busy: () => saving.value,
  dialog: () => ({ description: st('You have unsaved changes.') }),
  sameTarget: (to) =>
    to.path === route.path &&
    to.query.view === route.query.view &&
    to.query.scope === route.query.scope &&
    (scope.value !== 'session' || to.query.targetSessionId === route.query.targetSessionId),
})

async function load(preferred = selectedName.value) {
  const revision = ++generation
  const requestedSession = scope.value === 'session' ? sessionId.value : ''
  loading.value = true
  error.value = ''
  let sessionError = ''
  if (scope.value !== 'session') {
    const path = scope.value === 'workspace' ? directory.currentDirectory : null
    const workspace = path ? resolveWorkspace(path).then(({ id }) => id) : Promise.resolve(undefined)
    skillCatalog.value = null
    void workspace
      .then((id) => createSkillsClient(id).listSkills())
      .then(
        (result) => {
          if (revision === generation) skillCatalog.value = result
        },
        () => {
          if (revision === generation) skillCatalog.value = { skills: [], diagnostics: [], roots: [] }
        },
      )
  }
  try {
    const [
      profiles,
      defaultProfiles,
      prompts,
      session,
      summary,
      builtins,
      defaultSubagents,
      basePermission,
      namedPermissions,
      builtinPrompts,
    ] = await Promise.all([
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
      readRuntimeSettingSources('builtin_profiles'),
      readRuntimeSettingSources('default_subagent_profile'),
      scope.value === 'session' ? null : readRuntimeSettingSources('permission').catch(() => null),
      scope.value === 'session' ? null : readRuntimeSettingSources('access_control').catch(() => null),
      scope.value === 'session' ? [] : readBuiltinPrompts(),
      scope.value !== 'session' ? modelCatalog.loadProvidersAndModels() : Promise.resolve(),
    ])
    if (revision !== generation) return
    sources.value = profiles
    defaults.value = defaultProfiles
    builtinSources.value = builtins
    subagentDefaults.value = defaultSubagents
    promptSources.value = prompts
    promptCatalog.value = builtinPrompts
    permissionSources.value = basePermission
    permissionCatalog.value = namedPermissions
    emit('workspace-config-path', profiles.workspace.config_path || '')
    sessionReadFailed.value = !!sessionError
    error.value = sessionError
    // Summary identity survives a missing profile that prevents execution resolution.
    globalSession.value = summary?.workspace_id === null
    subagentSession.value = summary?.relation_kind === 'subagent'
    botSession.value = Boolean(summary?.owner_bot_id)
    effectiveSessionProfile.value = typeof session?.execution?.profile === 'string' ? session.execution.profile : ''
    sessionProfile.value = session?.execution?.profile_is_default ? '' : effectiveSessionProfile.value
    savedSessionProfile.value = sessionProfile.value
    if (props.createForBot) {
      beginNewProfile()
      return
    }
    const id = preferred && Object.hasOwn(catalog.value, preferred) ? preferred : Object.keys(catalog.value)[0]
    if (id && catalog.value[id]) beginEditing(id, catalog.value[id]!)
    else closeProfile()
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
  if (loading.value || !(await canDiscard())) return
  if (id === selectedName.value) closeProfile()
  else if (Object.hasOwn(catalog.value, id)) beginEditing(id, catalog.value[id]!)
}
async function addProfile() {
  if (loading.value || !(await canDiscard())) return
  beginNewProfile()
}
function beginNewProfile() {
  let id = 'profile'
  for (let index = 1; Object.hasOwn(visibleCatalog.value, id); index++) id = `profile-${index}`
  beginEditing(id, { title: '', description: '', ...(props.createForBot ? { mode: 'primary' } : {}) }, true)
}
function setSavedPrompts(order: string[]) {
  if (!parsedPrompts.value) return
  const layer = normalizePromptLayer(parsedPrompts.value)
  const slots = new Set(attachedPrompts.value)
  const remaining = [...order]
  layer.order = layer.order.flatMap((id) => (slots.has(id) ? remaining.splice(0, 1) : [id]))
  layer.order.push(...remaining)
  const disabled = new Set(layer.disabled.filter((id) => !order.includes(id)))
  for (const id of builtinCatalog.value[name.value]?.prompts?.order ?? []) {
    if (slots.has(id) && !order.includes(id)) disabled.add(id)
  }
  layer.disabled = [...disabled]
  promptJson.value = JSON.stringify(layer, null, 2)
}
function definition(): AgentProfileConfig {
  if (!draft.value || !isValidAgentProfileName(name.value))
    throw new Error(st('Use 1–64 lowercase letters, numbers, dots, underscores, or hyphens; do not start with a dot.'))
  if (isNew.value && Object.hasOwn(catalog.value, name.value))
    throw new Error(st('A profile with this name already exists.'))
  const profile: AgentProfileConfig = {
    ...draft.value,
    prompts: parseAgentProfileObject(promptJson.value),
    permission: parsePermissionConfig(permission.value),
  }
  if (profile.permission_profile) delete profile.permission
  else delete profile.permission_profile
  const model = parseAgentProfileObject(modelJson.value)
  if (Object.keys(model).length) profile.model = model
  else delete profile.model
  if (!profile.extends) delete profile.extends
  if (!profile.context?.project_instructions && profile.context?.preload_skills === undefined) delete profile.context
  if (profile.max_turns === null || profile.max_turns === undefined || String(profile.max_turns) === '')
    delete profile.max_turns
  else if (!Number.isSafeInteger(profile.max_turns) || profile.max_turns < 0)
    throw new Error(st('Maximum turns must be a non-negative integer.'))
  if (promptLayerHasErrors(validatePromptLayer(normalizePromptLayer(profile.prompts), true)))
    throw new Error(st('Fix the prompt layer before saving.'))
  const configured =
    scope.value === 'global' ? globalCatalog.value : mergedAgentProfiles(globalCatalog.value, workspaceCatalog.value)
  agentProfileMetadata(
    withBuiltinAgentProfiles(builtinCatalog.value, { ...configured, [name.value]: profile }),
    name.value,
  )
  return profile
}

async function saveProfile() {
  if (saving.value || scope.value === 'session' || !sources.value) return
  if (permissionEditors.value[0]?.applyPendingRule() === false) {
    showEditorSection('permissions')
    return
  }
  const source = scope.value
  const context = contextKey.value
  const section = editorSection.value
  saving.value = true
  error.value = ''
  try {
    const profile = definition()
    await setRuntimeSetting(
      'profiles',
      { ...rawCatalog.value, [name.value]: profile },
      { expected_revision: sources.value[source].revision },
      source,
    )
    if (props.createForBot) {
      emit('profile-created', name.value)
    } else if (context === contextKey.value) {
      await load(name.value)
      if (context === contextKey.value) showEditorSection(section)
    }
  } catch (reason) {
    if (context === contextKey.value) error.value = permissionConfigErrorMessage(reason, st)
  } finally {
    saving.value = false
  }
}
async function removeProfile() {
  if (saving.value || scope.value === 'session' || !sources.value || !selectedName.value || !hasOwnProfile.value) return
  const target = selectedName.value
  const source = scope.value
  const context = contextKey.value
  if (
    !restoresInherited.value &&
    (agentProfileReferences(visibleCatalog.value, target).length ||
      primaryDefault.value === target ||
      subagentDefault.value === target)
  ) {
    error.value = st('Choose another default and update inherited profiles before deleting this profile.')
    return
  }
  if (
    !(await confirmAction({
      title: restoresInherited.value ? st('Remove profile override?') : st('Delete agent profile?'),
      description: restoresInherited.value
        ? st('Removing this override restores the inherited profile.')
        : st('Sessions using this profile will need another profile before they can continue.'),
      confirmText: restoresInherited.value ? st('Restore inherited profile') : st('Delete'),
      cancelText: st('Cancel'),
    }))
  )
    return
  if (saving.value || !sources.value || context !== contextKey.value || target !== selectedName.value) return
  saving.value = true
  error.value = ''
  try {
    const next = { ...rawCatalog.value }
    delete next[target]
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
    if (sessionProfile.value && !Object.hasOwn(sessionCatalog.value, sessionProfile.value))
      throw new Error(st('Choose a profile compatible with this session.'))
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
watch(contextKey, () => void load(''))
onMounted(() => {
  void load('')
})
onBeforeUnmount(() => {
  generation++
})
</script>

<template>
  <section class="settings-stack" :aria-busy="loading || saving" data-agent-profiles>
    <div
      v-if="scope !== 'session' && !createForBot"
      class="settings-toolbar rounded-lg border border-border/70 bg-card/30 p-2.5"
    >
      <SearchInput
        v-model="query"
        class="settings-toolbar-search"
        :placeholder="$st('Search profiles…')"
        :input-aria-label="$st('Search profiles…')"
      />
      <RouterLink
        :to="defaultSettingsDestination(scope)"
        class="rounded-md px-2 py-1 text-xs font-medium text-muted-foreground hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >{{ $st('Default profile settings') }}</RouterLink
      >
      <IconButton
        size="sm"
        :loading="loading"
        :disabled="saving"
        :aria-label="$st('Refresh')"
        :tooltip="$st('Refresh')"
        @click="refresh"
        ><RiRefreshLine class="h-4 w-4" aria-hidden="true"
      /></IconButton>
      <Button size="sm" :disabled="loading || saving || !sources" @click="addProfile">
        <RiAddLine class="me-1.5 h-4 w-4" aria-hidden="true" />{{ $st('New profile') }}
      </Button>
    </div>
    <p
      v-if="error && (scope === 'session' || !dirty)"
      role="alert"
      class="break-words rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive"
    >
      {{ error }}
    </p>
    <p v-if="loading && !sources" role="status" class="px-2 text-sm text-muted-foreground">
      {{ $st('Loading…') }}
    </p>

    <SettingsCard v-if="scope === 'session'" :title="$st('Current session profile')" :icon="RiRobot2Line">
      <template #actions>
        <IconButton
          size="sm"
          :loading="loading"
          :disabled="saving"
          :aria-label="$st('Refresh')"
          :tooltip="$st('Refresh')"
          @click="refresh"
          ><RiRefreshLine class="h-4 w-4" aria-hidden="true"
        /></IconButton>
      </template>
      <div class="settings-stack">
        <p v-if="!sessionId" class="text-sm text-muted-foreground">
          {{ $st('Open a conversation to select its agent profile.') }}
        </p>
        <template v-else-if="sources">
          <div class="settings-field-row">
            <label class="profile-field">
              {{ $st('Agent profiles') }}
              <select
                v-model="sessionProfile"
                :aria-label="$st('Current session profile')"
                :disabled="loading || saving"
              >
                <option value="">
                  {{ restoreSessionProfile }}
                </option>
                <option v-if="sessionProfile && !Object.hasOwn(sessionCatalog, sessionProfile)" :value="sessionProfile">
                  {{ sessionProfile }}
                </option>
                <option v-for="[id, profile] in entries" :key="id" :value="id">
                  {{ agentProfileDisplayName(id, profile) }} · {{ id }} · {{ profileModeLabel(id) }}
                </option>
              </select>
            </label>
            <div class="min-w-0">
              <p v-if="effectiveSessionProfile" class="text-xs text-muted-foreground">
                {{ $st('Effective profile') }} · <code>{{ effectiveSessionProfile }}</code>
              </p>
              <p
                v-if="sessionProfile && visibleCatalog[sessionProfile]?.description"
                class="mt-1 text-xs text-muted-foreground"
              >
                {{ visibleCatalog[sessionProfile]?.description }}
              </p>
            </div>
          </div>
          <div v-if="dirty || sessionReadFailed || saving">
            <Button size="sm" :disabled="loading || saving" @click="saveSession">
              {{ saving ? $st('Saving…') : $st('Apply profile') }}
            </Button>
          </div>
        </template>
        <div class="settings-toolbar">
          <RouterLink
            :to="defaultSettingsDestination('global')"
            class="rounded-md border border-input px-2.5 py-1.5 text-xs font-medium hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >{{ $st('Global default profiles') }}</RouterLink
          >
          <RouterLink
            v-if="directory.currentDirectory"
            :to="defaultSettingsDestination('workspace')"
            class="rounded-md border border-input px-2.5 py-1.5 text-xs font-medium hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >{{ $st('Workspace default profiles') }}</RouterLink
          >
        </div>
      </div>
    </SettingsCard>

    <template v-else-if="sources">
      <div class="grid min-w-0 gap-2">
        <SettingsDisclosureRow
          v-for="row in profileRows"
          :key="row.key"
          :data-agent-profile="row.isNew ? '' : row.id"
          class="min-w-0 bg-card/30"
          body-class="min-w-0 p-2.5"
          :open="row.open"
          :label="row.label"
          :summary="row.summary"
          :tone="row.open && dirty ? 'dirty' : 'default'"
          :disabled="loading || saving"
          @toggle="row.isNew ? discardProfile() : selectProfile(row.id)"
        >
          <template #leading>
            <span class="profile-row-icon" aria-hidden="true"><RiRobot2Line class="h-4 w-4" /></span>
          </template>
          <template v-if="row.open && (row.isNew || hasOwnProfile)" #actions>
            <IconButton
              size="sm"
              :aria-label="
                row.isNew
                  ? $st('Discard changes')
                  : restoresInherited
                    ? $st('Restore inherited profile')
                    : $st('Delete agent profile?')
              "
              :tooltip="
                row.isNew
                  ? $st('Discard changes')
                  : restoresInherited
                    ? $st('Restore inherited profile')
                    : $st('Delete')
              "
              :disabled="loading || saving"
              @click="row.isNew ? discardProfile() : removeProfile()"
            >
              <RiResetLeftLine v-if="!row.isNew && restoresInherited" class="h-4 w-4" aria-hidden="true" />
              <RiDeleteBinLine v-else class="h-4 w-4 text-destructive" aria-hidden="true" />
            </IconButton>
          </template>

          <fieldset v-if="row.open && draft" :disabled="loading || saving" class="settings-stack">
            <SegmentedControl role="group" class="flex flex-wrap" :aria-label="$st('Profile configuration')">
              <SegmentedButton
                v-for="section in editorSections"
                :key="section.id"
                :active="editorSection === section.id"
                role="button"
                :aria-selected="undefined"
                :aria-pressed="editorSection === section.id"
                class="h-auto min-h-8 min-w-0 whitespace-normal px-2.5 py-1"
                :data-profile-section="section.id"
                @click="showEditorSection(section.id)"
                >{{ section.label }}</SegmentedButton
              >
            </SegmentedControl>

            <section
              v-if="editorSection === 'profile'"
              :aria-label="$st('Profile configuration')"
              class="profile-fields"
            >
              <p v-if="!isNew && !hasOwnProfile" class="mb-2 text-xs text-muted-foreground">
                {{ $st('This profile is inherited. Saving creates an override in this scope.') }}
              </p>
              <div class="settings-control-grid">
                <label class="profile-field">
                  {{ $st('Profile name') }}
                  <Input v-model="name" :disabled="!isNew" spellcheck="false" class="h-8 font-mono" />
                </label>
                <label class="profile-field">
                  {{ $st('Title') }}
                  <Input v-model="draft.title" class="h-8" />
                </label>
                <label class="profile-field">
                  {{ $st('Inherit from profile') }}
                  <select
                    :value="draft.extends || ''"
                    :aria-label="$st('Inherit from profile')"
                    @change="draft.extends = ($event.target as HTMLSelectElement).value || null"
                  >
                    <option value="">{{ $st('None') }}</option>
                    <option v-for="(profile, id) in visibleCatalog" :key="id" :value="id" :disabled="id === name">
                      {{ agentProfileDisplayName(id, profile) }} · {{ id }}
                    </option>
                  </select>
                </label>
                <label class="profile-field">
                  {{ $st('Role availability') }}
                  <select
                    :value="draft.mode || ''"
                    :aria-label="$st('Role availability')"
                    @change="
                      draft.mode = (($event.target as HTMLSelectElement).value || null) as AgentProfileConfig['mode']
                    "
                  >
                    <option v-if="!createForBot" value="">{{ $st('Inherit') }}</option>
                    <option value="primary">{{ $st('Primary sessions') }}</option>
                    <option v-if="!createForBot" value="subagent">{{ $st('Subagents') }}</option>
                    <option value="all">{{ $st('Primary sessions and subagents') }}</option>
                  </select>
                </label>
                <label class="profile-field">
                  {{ $st('Model discovery') }}
                  <select
                    :value="draft.hidden == null ? '' : String(draft.hidden)"
                    :aria-label="$st('Model discovery')"
                    @change="
                      draft.hidden =
                        ($event.target as HTMLSelectElement).value === ''
                          ? null
                          : ($event.target as HTMLSelectElement).value === 'true'
                    "
                  >
                    <option value="">{{ $st('Inherit') }}</option>
                    <option value="false">{{ $st('Visible to the model') }}</option>
                    <option value="true">{{ $st('Hidden from model discovery') }}</option>
                  </select>
                </label>
                <label class="profile-field">
                  {{ $st('Execution access') }}
                  <select v-model="readOnlyAccess" :aria-label="$st('Execution access')">
                    <option value="">
                      {{ inheritedReadOnly ? `${$st('Inherit')} · ${$st('Read only')}` : $st('Inherit') }}
                    </option>
                    <option value="true">{{ $st('Read only') }}</option>
                    <option value="false">{{ $st('Use configured permissions') }}</option>
                  </select>
                </label>
                <label v-if="draft.mode !== 'primary'" class="profile-field">
                  {{ $st('Subagent workspace') }}
                  <select
                    v-model="isolation"
                    :aria-label="$st('Subagent workspace')"
                    :disabled="draftReadOnly"
                  >
                    <option value="">{{ $st('Inherit') }}</option>
                    <option value="shared">{{ $st('Shared checkout') }}</option>
                    <option value="worktree">{{ $st('Git worktree') }}</option>
                  </select>
                </label>
                <label class="profile-field">
                  {{ $st('Project instructions') }}
                  <select v-model="projectInstructions" :aria-label="$st('Project instructions')">
                    <option value="">{{ $st('Inherit') }}</option>
                    <option value="startup">{{ $st('Load at startup') }}</option>
                    <option value="on_demand">{{ $st('Read on demand') }}</option>
                  </select>
                </label>
              </div>
              <label class="profile-field mt-2">
                {{ $st('Description') }}
                <textarea v-model="draft.description" rows="2" />
              </label>
              <fieldset class="mt-4 grid min-w-0 gap-2 border-t border-border/60 pt-2">
                <legend class="pe-2 text-xs font-medium">{{ $st('Preloaded skills') }}</legend>
                <label class="flex items-center gap-2 text-xs text-muted-foreground">
                  <input
                    type="checkbox"
                    :checked="draft.context?.preload_skills == null"
                    @change="inheritsPreloadedSkills(($event.target as HTMLInputElement).checked)"
                  />
                  {{ $st('Inherit preloaded skills') }}
                </label>
                <PromptReferenceList
                  v-if="draft.context?.preload_skills"
                  :model-value="draft.context.preload_skills"
                  :options="skillOptions"
                  :select-label="$st('Select a skill')"
                  :limit="8"
                  :disabled="loading || saving"
                  @update:model-value="draft.context = { ...draft.context, preload_skills: $event }"
                />
              </fieldset>
            </section>

            <section
              v-else-if="editorSection === 'instructions'"
              :aria-label="$st('Instructions')"
              class="profile-fields"
            >
              <div class="grid min-w-0 gap-2.5">
                <textarea
                  v-model="instructions"
                  :aria-label="$st('Instructions')"
                  :disabled="!parsedPrompts"
                  rows="8"
                  class="profile-textarea"
                />
                <div class="settings-toolbar items-end">
                  <label class="profile-field min-w-0 flex-[1_1_8rem]">
                    {{ $st('Layer mode') }}
                    <select v-model="promptMode" :aria-label="$st('Layer mode')" :disabled="!parsedPrompts">
                      <option value="append">{{ $st('Append') }}</option>
                      <option value="replace">{{ $st('Replace') }}</option>
                    </select>
                  </label>
                </div>
                <PromptReferenceList
                  :model-value="attachedPrompts"
                  :options="savedPrompts"
                  :disabled="!parsedPrompts"
                  @update:model-value="setSavedPrompts"
                />
                <details>
                  <summary class="cursor-pointer text-xs font-medium text-muted-foreground">
                    {{ $st('Prompt layer (JSON)') }}
                  </summary>
                  <textarea
                    v-model="promptJson"
                    :aria-label="$st('Prompt layer (JSON)')"
                    spellcheck="false"
                    rows="8"
                    class="profile-textarea mt-2 font-mono"
                  />
                </details>
              </div>
            </section>

            <section v-else-if="editorSection === 'model'" :aria-label="$st('Model & limits')" class="profile-fields">
              <div class="grid min-w-0 gap-2.5">
                <AgentProfileModelFields
                  :model-value="parsedModel || {}"
                  :providers="modelCatalog.providers.value"
                  :disabled="loading || saving || !parsedModel"
                  @update:model-value="modelJson = JSON.stringify($event, null, 2)"
                />
                <label class="profile-field max-w-48">
                  {{ $st('Maximum turns') }}
                  <input
                    v-model.number="draft.max_turns"
                    type="number"
                    min="0"
                    step="1"
                    :placeholder="$st('Inherit')"
                    :title="$st('Leave empty to inherit; 0 allows unlimited turns.')"
                  />
                </label>
                <details>
                  <summary class="cursor-pointer text-xs font-medium text-muted-foreground">
                    {{ $st('Model selection (JSON)') }}
                  </summary>
                  <textarea
                    v-model="modelJson"
                    :aria-label="$st('Model selection (JSON)')"
                    spellcheck="false"
                    rows="6"
                    class="profile-textarea mt-2 font-mono"
                  />
                </details>
              </div>
            </section>

            <div
              v-if="permissionOpened"
              v-show="editorSection === 'permissions'"
              class="settings-stack"
              data-profile-permissions
            >
              <label class="profile-field" v-if="completePermissions.length || draft.permission_profile">
                {{ $st('Permission profile') }}
                <select
                  :value="draft.permission_profile || ''"
                  @change="draft.permission_profile = ($event.target as HTMLSelectElement).value"
                >
                  <option value="">{{ $st('Custom permissions') }}</option>
                  <option v-for="[id, profile] in completePermissions" :key="id" :value="id">
                    {{ permissionProfileDisplayName(id, profile) }}
                  </option>
                </select>
              </label>
              <PermissionStudioPanel
                v-if="namedPermission"
                :key="draft.permission_profile"
                :draft-permission="completePermissionConfig(namedPermission.permission)"
                standalone-draft
                draft-disabled
              />
              <PermissionStudioPanel
                v-else
                :key="editorKey"
                ref="permissionEditors"
                v-model:draft-permission="permission"
                :inherited-draft-permission="inheritedPermission"
                :draft-disabled="loading || saving"
              />
              <SettingsCard v-if="!namedPermission" :title="$st('Automatic approval model')" :icon="RiCpuLine">
                <AgentProfileModelFields
                  :model-value="approvalModel"
                  :providers="modelCatalog.providers.value"
                  :disabled="loading || saving"
                  require-route
                  @update:model-value="setApprovalModel"
                />
              </SettingsCard>
            </div>
          </fieldset>
        </SettingsDisclosureRow>
        <p
          v-if="!profileRows.length && !loading"
          class="rounded-lg border border-dashed border-border/60 p-4 text-center text-sm text-muted-foreground"
        >
          {{
            entries.length
              ? $st('No profiles match this search.')
              : $st('Create a profile to reuse your agent configuration.')
          }}
        </p>
      </div>
      <SettingsSaveBar
        v-if="dirty || saving"
        :dirty="dirty"
        :saving="saving"
        :disabled="loading"
        :error="error"
        :save-label="!isNew && !hasOwnProfile ? $st('Save override') : $st('Save changes')"
        data-profile-savebar
        sticky
        @save="saveProfile"
        @discard="discardProfile"
      />
    </template>
  </section>
</template>

<style scoped>
@reference '../../style.css';

.profile-row-icon {
  @apply hidden h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-border/60 bg-muted/30 text-muted-foreground sm:flex;
}
.profile-field {
  @apply grid min-w-0 gap-1 text-xs text-muted-foreground;
}
.profile-fields {
  @apply min-w-0 rounded-lg border border-border/70 bg-card/30 p-3;
}
.profile-field > :is(select, input, textarea),
.profile-textarea {
  @apply min-w-0 w-full rounded-md border border-input bg-background px-2 py-1 text-[13px] text-foreground outline-none focus:border-ring disabled:opacity-60;
}
.profile-field > :is(select, input) {
  min-height: 2rem;
}
.profile-field > textarea,
.profile-textarea {
  resize: vertical;
}
</style>
