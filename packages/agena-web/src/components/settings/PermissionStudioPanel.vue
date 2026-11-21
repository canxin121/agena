<script setup lang="ts">
import { computed, defineAsyncComponent, nextTick, onBeforeUnmount, onMounted, ref, useId, watch, type Ref } from 'vue'
import {
  RouterLink,
  onBeforeRouteLeave,
  onBeforeRouteUpdate,
  useRoute,
  useRouter,
  type RouteLocationNormalized,
} from 'vue-router'
import {
  RiAddLine,
  RiCheckLine,
  RiCloseLine,
  RiCodeSSlashLine,
  RiFolderLine,
  RiGlobalLine,
  RiListSettingsLine,
  RiLockLine,
  RiRefreshLine,
  RiResetLeftLine,
  RiSave3Line,
  RiToolsLine,
} from '@remixicon/vue'
import { PopoverContent, PopoverPortal, PopoverRoot, PopoverTrigger } from 'radix-vue'

import Button from '@/components/ui/Button.vue'
import { findSettingsScope } from '@/components/settings/settingsScopeCatalog'
import IconButton from '@/components/ui/IconButton.vue'
import Input from '@/components/ui/Input.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import PermissionBulkActions from './permissions/PermissionBulkActions.vue'
import PermissionRuleTable from './permissions/PermissionRuleTable.vue'
import type { PermissionRuleKind, PermissionRuleRow } from './permissions/permissionRuleTable'
import PermissionAccessToolbar from './permissions/PermissionAccessToolbar.vue'
import PermissionPolicyRow from './permissions/PermissionPolicyRow.vue'
import PluginPermissionEditor from './permissions/PluginPermissionEditor.vue'
import PermissionHelp from './permissions/PermissionHelp.vue'
import SettingsSourceControl from './SettingsSourceControl.vue'
import SandboxSettingsPanel from './SandboxSettingsPanel.vue'
import {
  isPermissionAccessView,
  legacyPermissionAccessScope,
  PERMISSION_ACCESS_VIEW,
  permissionAccessHash,
  permissionSectionFromHash,
  permissionWorkspaceTarget,
  type PermissionAccessSection,
} from './permissions/permissionNavigation'
import { apiJson } from '@/lib/api'
import { confirmAction } from '@/lib/appConfirm'
import { deleteRuntimeSetting, setRuntimeSetting, type RuntimeSettingReadResponse } from '@/lib/runtimeSettings'
import { runtimeSettingBaseValue } from '@/lib/runtimeSettingLayer'
import {
  DEFAULT_PERMISSION_PROFILE_ID,
  permissionProfileDisplayName,
  permissionProfilesForLayer,
  type PermissionProfile,
} from '@/lib/permissionProfiles'
import { useChatStoreUnscoped } from '@/stores/chat'
import { lastSessionId } from '@/lib/lastSession'
import { resolveSettingsSessionId } from './settingsSessionScope'
import { useWorkspacePaneContext } from '@/app/workspace/workspacePaneContext'
import { useToastsStore } from '@/stores/toasts'
import type { JsonValue } from '@/types/json'
import { settingsText as st } from '@/i18n/settingsText'
import { parsePluginStatusList, type PluginRuntimeStatus } from '@/features/pluginWeb/hub'

import {
  SHELL_COMMAND_RULE_GROUP,
  canonicalPermissionPath,
  canonicalPermissionToolEntries,
  canonicalPermissionToolName,
  completePermissionConfig,
  isPathPermissionMode,
  mergeNetworkPermissionConfig,
  parsePermissionConfig,
  parseNetworkPermissionConfig,
  permissionConfigErrorMessage,
  renamePermissionRule,
  type PathPermissionMode,
  type PermissionConfig,
  type PermissionMode,
  type NetworkAction,
  type NetworkProtocol,
  type NetworkRule,
} from '@/lib/permissionConfig'

type PermissionSource = 'global' | 'workspace' | 'session' | 'effective'
type SettingsResponse = RuntimeSettingReadResponse
type SessionPermissionState = {
  selected_permission?: PermissionConfig
  effective_permission?: PermissionConfig
  inherited_permission?: PermissionConfig
  permission_profile?: string
}
type SessionStateResponse = {
  execution?: SessionPermissionState & { context?: SessionPermissionState; execution?: SessionPermissionState }
}
type ToolCatalogResponse = { permission_tools?: Array<{ name?: string; summary?: string }> }

const props = defineProps<{
  scope?: string
  sessionId?: string
  draftPermission?: PermissionConfig
  inheritedDraftPermission?: PermissionConfig | null
  draftDisabled?: boolean
  standaloneDraft?: boolean
}>()
const emit = defineEmits<{ 'update:draftPermission': [value: PermissionConfig] }>()
const profileEditor = computed(() => props.draftPermission !== undefined)
const route = useRoute()
const router = useRouter()
const workspacePane = useWorkspacePaneContext()
const editorElement = ref<HTMLElement | null>(null)
const sections = computed(() => [
  {
    id: 'filesystem' as const,
    label: st('File access'),
    icon: RiFolderLine,
    help: [
      st('Choose readonly, readwrite, or deny for workspace and external paths.'),
      st('Use concrete files or directories. Directories include all descendants; denied paths cannot be reopened.'),
    ].join(' '),
  },
  {
    id: 'network' as const,
    label: st('Network access'),
    icon: RiGlobalLine,
    help: [
      st('Each network setting has its own source. Changing one leaves the others unchanged.'),
      st('Deny rules take precedence over every allow rule.'),
      st(
        'macOS does not support per-destination outbound rules. Linux requires inbound and host-loopback access to remain denied.',
      ),
    ].join(' '),
  },
  {
    id: 'tools' as const,
    label: st('Tool execution & approvals'),
    icon: RiToolsLine,
    help: [
      st('Set the fallback policy and access for tools that only read data.'),
      st('Choose a tool to override its default access policy.'),
      st('All terminal tools use the same command rules. If several patterns match, the last one wins.'),
    ].join(' '),
  },
])
const chat = useChatStoreUnscoped()
const toasts = useToastsStore()
const PermissionsPanel = defineAsyncComponent(() => import('@/components/settings/PermissionsPanel.vue'))

const loading = ref(false)
const saving = ref(false)
const error = ref('')
const saveError = ref('')
const loaded = ref(false)
const appliedProfile = ref('')
const chosenProfile = ref('')
const hasCustomPermission = ref(false)
const permissionProfiles = ref<Record<string, PermissionProfile> | null>(null)
const linkedPermission = computed(() => !profileEditor.value && appliedProfile.value !== '')
const profileOptions = computed(() =>
  Object.entries(permissionProfiles.value ?? {}).sort((a, b) => a[1].name.localeCompare(b[1].name)),
)
const savedConfig = ref<PermissionConfig>({})
const rawEditorOpen = ref(false)
const rawJsonErrorId = useId()
const rulesPanel = ref<{ refresh: () => Promise<void>; busy?: boolean } | null>(null)
const sandboxSettings = ref<InstanceType<typeof SandboxSettingsPanel> | null>(null)
const rawPending = computed(() => rawJson.value !== JSON.stringify(config.value, null, 2))
const networkPending = computed(
  () => networkJson.value !== JSON.stringify(config.value.network || { version: 2 }, null, 2),
)
const dirty = computed(
  () =>
    JSON.stringify(config.value) !== JSON.stringify(savedConfig.value) ||
    rawPending.value ||
    networkPending.value ||
    Boolean(networkRuleDraft.value),
)
const sandboxSaving = computed(() => sandboxSettings.value?.saving === true)
const editingDisabled = computed(
  () =>
    !canEdit.value ||
    !loaded.value ||
    loading.value ||
    saving.value ||
    sandboxSaving.value ||
    props.draftDisabled ||
    linkedPermission.value,
)
const formDisabled = computed(() => editingDisabled.value || rawPending.value || networkPending.value)
// The workbench owns source selection; this editor only normalizes the layer.
const settingsScope = computed(() => normalizePermissionSource(props.scope))
const selectedSource = computed(() =>
  profileEditor.value ? (props.standaloneDraft ? 'global' : 'profile') : settingsScope.value,
)

function normalizePermissionSource(value: unknown): PermissionSource {
  return value === 'global' || value === 'workspace' || value === 'session' ? value : 'effective'
}
const config = ref<PermissionConfig>({})
// An explicit source choice can precede a value when the parent is unavailable.
// It remains an editor intent until the user supplies a real local value.
const localSourceIntents = ref<Set<string>>(new Set())
function setLocalSourceIntent(key: string, enabled: boolean) {
  const next = new Set(localSourceIntents.value)
  if (enabled) next.add(key)
  else next.delete(key)
  localSourceIntents.value = next
}
function sourceIsReadOnly(source: 'inherit' | 'local'): boolean {
  return selectedSource.value !== 'global' && source === 'inherit'
}
function sourceValueDisabled(source: 'inherit' | 'local'): boolean {
  return formDisabled.value || sourceIsReadOnly(source)
}
function toolRuleSource(tool: string): 'inherit' | 'local' {
  return hasToolEntry(config.value.tools?.rules, tool) ||
    localSourceIntents.value.has('tools.rules.' + canonicalPermissionToolName(tool))
    ? 'local'
    : 'inherit'
}
function inheritedEditError(): string {
  return st('Switch inherited settings to this scope before editing their values.')
}
const rawJson = ref('{}')
const rawJsonError = ref('')
const networkJson = ref(JSON.stringify({ version: 2 }, null, 2))
const networkJsonError = ref('')
const inheritedPermission = ref<PermissionConfig | null>(null)
const resolvedPermission = ref<PermissionConfig | null>(null)
const resolvedRawFingerprint = ref('')
const cleanResolvedPermission = computed(() =>
  !dirty.value && JSON.stringify(config.value) === resolvedRawFingerprint.value ? resolvedPermission.value : null,
)
const configPath = ref('')
const inheritedNetwork = computed(() => inheritedPermission.value?.network ?? null)
const inheritedNetworkError = ref('')
const activeWorkspaceTarget = computed(() => permissionWorkspaceTarget(route.query.targetWorkspaceId))
const workspaceId = ref<number | null>(null)
const workspaceConfigIsActive = ref<boolean | undefined>()
const configRevision = ref<string | undefined>()
type NetworkRuleList = 'allow' | 'deny'
type NetworkDefault = 'default' | 'inbound' | 'hostLoopback'
type NetworkField = NetworkDefault | NetworkRuleList
const networkRuleLists = ['allow', 'deny'] as const
const networkFields = ['default', 'inbound', 'hostLoopback', ...networkRuleLists] as const
const networkFullyLocal = computed(
  () => selectedSource.value === 'global' || networkFields.every((field) => networkSource(field) === 'local'),
)
const networkHasLocal = computed(
  () => selectedSource.value === 'global' || networkFields.some((field) => networkSource(field) === 'local'),
)
const networkJsonReadOnly = computed(
  () => editingDisabled.value || rawPending.value || Boolean(networkRuleDraft.value) || !networkHasLocal.value,
)
const networkJsonDisplay = computed(() =>
  networkHasLocal.value ? networkJson.value : JSON.stringify(displayedNetwork.value, null, 2),
)
const rawJsonReadOnly = computed(
  () =>
    editingDisabled.value ||
    networkPending.value ||
    Boolean(networkRuleDraft.value) ||
    (selectedSource.value !== 'global' && !Object.keys(config.value).length && !localSourceIntents.value.size),
)
const rawJsonDisplay = computed(() =>
  selectedSource.value !== 'global' && !Object.keys(config.value).length && !localSourceIntents.value.size
    ? JSON.stringify(inheritedPermission.value, null, 2)
    : rawJson.value,
)
function editNetworkJson(event: Event) {
  if (networkJsonReadOnly.value) return
  networkJson.value = (event.target as HTMLTextAreaElement).value
  networkJsonError.value = ''
}
function editRawJson(event: Event) {
  if (rawJsonReadOnly.value) return
  rawJson.value = (event.target as HTMLTextAreaElement).value
  rawJsonError.value = ''
}
function assertEditableOverrides(next: PermissionConfig) {
  if (selectedSource.value === 'global') return
  const blocked = (condition: boolean) => {
    if (condition) throw new Error(inheritedEditError())
  }
  for (const scope of ['workspace', 'external'] as const)
    blocked(sourceIsReadOnly(pathSource(scope)) && Object.hasOwn(next.path || {}, scope))
  blocked(sourceIsReadOnly(toolDefaultSource.value) && Object.hasOwn(next.tools || {}, 'default'))
  for (const field of networkFields)
    blocked(sourceIsReadOnly(networkSource(field)) && Object.hasOwn(next.network || {}, field))
  for (const path of Object.keys(inheritedPermission.value?.path?.rules || {}))
    blocked(!Object.hasOwn(config.value.path?.rules || {}, path) && Object.hasOwn(next.path?.rules || {}, path))
  for (const [name] of canonicalPermissionToolEntries(inheritedPermission.value?.tools?.names))
    blocked(!hasToolEntry(config.value.tools?.names, name) && hasToolEntry(next.tools?.names, name))
  for (const [tool] of canonicalPermissionToolEntries(next.tools?.rules))
    blocked(
      sourceIsReadOnly(toolRuleSource(tool)) &&
        (tool === '*' || hasToolEntry(inheritedPermission.value?.tools?.rules, tool)),
    )
}
function settleSourceIntents(next: PermissionConfig) {
  const declared = (key: string) =>
    key.startsWith('path.')
      ? Object.hasOwn(next.path || {}, key.slice(5))
      : key.startsWith('network.')
        ? Object.hasOwn(next.network || {}, key.slice(8))
        : key === 'tools.default'
          ? Object.hasOwn(next.tools || {}, 'default')
          : key.startsWith('tools.rules.') && hasToolEntry(next.tools?.rules, key.slice(12))
  localSourceIntents.value = new Set([...localSourceIntents.value].filter((key) => !declared(key)))
}
type NetworkRuleDraft = {
  action: NetworkRuleList
  index: number | null
  to: string
  protocol: NetworkProtocol
  ports: string
  except: string
}
const networkRuleDraft = ref<NetworkRuleDraft | null>(null)
const networkRuleError = ref('')
const networkCustom = computed(() => config.value.network !== undefined)
const displayedNetwork = computed(() => {
  try {
    if (cleanResolvedPermission.value) return cleanResolvedPermission.value.network ?? null
    return mergeNetworkPermissionConfig(inheritedNetwork.value, config.value.network)
  } catch {
    // Invalid drafts remain in the JSON editor until the user repairs them.
    return null
  }
})
const networkProtocolOptions = computed(() => [
  ...(['tcp', 'udp', 'icmp'] as const).map((value) => ({ value, label: value.toUpperCase() })),
  { value: 'any', label: st('Any protocol') },
])
const toolCatalog = ref<Array<{ name: string; summary: string }>>([])
const installedPlugins = ref<PluginRuntimeStatus[]>([])
const pluginCatalogUnavailable = ref(false)
const newPath = ref('')
const newPathMode = ref<PathPermissionMode>('readonly')
const newToolName = ref('')
const newToolNameMode = ref<PermissionMode>('ask')
const newCommandPattern = ref('')
const newCommandMode = ref<PermissionMode>('ask')
const selectedPathRuleKeys = ref<Set<string>>(new Set())
const selectedToolNameRuleKeys = ref<Set<string>>(new Set())
const selectedCommandRuleKeys = ref<Set<string>>(new Set())
const bulkPathMode = ref<PathPermissionMode>('readonly')
const bulkToolNameMode = ref<PermissionMode>('auto')
const bulkCommandMode = ref<PermissionMode>('ask')
const pathQuery = ref('')
const toolQuery = ref('')
const newRuleKind = ref<PermissionRuleKind | ''>('')

const activeSessionId = computed(() => {
  // An explicit target lets a settings surface describe a conversation other
  // than whichever chat tab last loaded; otherwise the current selection owns it.
  const value = Number(
    String(props.sessionId || '').trim() ||
      resolveSettingsSessionId(route.query.targetSessionId, lastSessionId.value || chat.selectedSessionId),
  )
  return Number.isSafeInteger(value) && value > 0 ? value : null
})
const canEdit = computed(
  () => selectedSource.value !== 'effective' && (selectedSource.value !== 'session' || Boolean(activeSessionId.value)),
)
const modeOptions = computed(() => [
  { value: 'allow', label: st('Allow'), description: st('Always permit matching access.') },
  { value: 'auto', label: st('Auto'), description: st('Let the approval model decide.') },
  { value: 'ask', label: st('Ask'), description: st('Ask before matching access.') },
  { value: 'deny', label: st('Deny'), description: st('Always block matching access.') },
])
const pathModeOptions = computed(() => [
  { value: 'readonly', label: st('Read only'), description: st('Read files without changing them.') },
  { value: 'readwrite', label: st('Read and write'), description: st('Read, create, change, and delete files.') },
  { value: 'deny', label: st('Deny'), description: st('Block access to this file or directory tree.') },
])
const toolOptions = computed(() =>
  toolCatalog.value.map((tool) => ({ value: tool.name, label: tool.name, description: tool.summary })),
)
const pathScopes = computed(() => [
  { id: 'workspace' as const, label: st('Workspace files'), description: st('Files inside the current project.') },
  { id: 'external' as const, label: st('External files'), description: st('Files outside the current project.') },
])
const networkActionOptions = computed(() => [
  { value: 'allow', label: st('Allow'), description: st('Allow matching network traffic.') },
  { value: 'deny', label: st('Deny'), description: st('Block matching network traffic.') },
])
const networkRuleActionOptions = computed(() =>
  networkActionOptions.value.map((option) => ({
    ...option,
    disabled: sourceIsReadOnly(networkSource(option.value as NetworkRuleList)),
  })),
)
function setNetworkRuleAction(value: string) {
  if (!networkRuleDraft.value || (value !== 'allow' && value !== 'deny') || sourceValueDisabled(networkSource(value)))
    return
  networkRuleDraft.value.action = value
}
const networkAdvancedDefaults = computed(() => [
  {
    id: 'inbound' as const,
    label: st('Inbound default'),
    description: st('Whether external peers can initiate connections.'),
  },
  {
    id: 'hostLoopback' as const,
    label: st('Host loopback'),
    description: st('Access to services on the host loopback interface.'),
  },
])
const pathRules = computed(() => Object.entries(config.value.path?.rules || {}))
const toolNameRules = computed(() => canonicalPermissionToolEntries(config.value.tools?.names))
const commandSource = computed(() => toolRuleSource(SHELL_COMMAND_RULE_GROUP))
const commandSourceKey = 'tools.rules.' + SHELL_COMMAND_RULE_GROUP
function commandRuleGroup(permission: PermissionConfig | null) {
  const rules = permission?.tools?.rules?.[SHELL_COMMAND_RULE_GROUP]
  return rules && typeof rules === 'object' && !Array.isArray(rules) ? rules : undefined
}
const commandRules = computed(() => Object.entries(commandRuleGroup(config.value) || {}))
const visiblePathRules = computed(() => pathRules.value.filter(([path]) => matchesQuery(path, pathQuery.value)))
const visibleToolNameRules = computed(() => toolNameRules.value.filter(([name]) => matchesQuery(name, toolQuery.value)))
const inheritedPathRules = computed(() =>
  Object.entries(inheritedPermission.value?.path?.rules || {}).filter(
    ([path]) => !Object.hasOwn(config.value.path?.rules || {}, path),
  ),
)
const inheritedToolNameRules = computed(() =>
  canonicalPermissionToolEntries(inheritedPermission.value?.tools?.names).filter(
    ([name]) => !hasToolEntry(config.value.tools?.names, name),
  ),
)
const inheritedCommandRules = computed(() =>
  hasToolEntry(config.value.tools?.rules, SHELL_COMMAND_RULE_GROUP)
    ? []
    : Object.entries(commandRuleGroup(inheritedPermission.value) || {}),
)
const visibleInheritedPathRules = computed(() =>
  inheritedPathRules.value.filter(([path]) => matchesQuery(path, pathQuery.value)),
)
const visibleInheritedToolNameRules = computed(() =>
  inheritedToolNameRules.value.filter(([name]) => matchesQuery(name, toolQuery.value)),
)
const toolDefaultEditorMode = computed(() => {
  const mode = config.value.tools?.default
  if (mode == null) return ''
  return isMode(mode) ? mode : '__invalid__'
})
const displayedToolDefault = computed<PermissionMode | undefined>(() => {
  const mode = cleanResolvedPermission.value
    ? cleanResolvedPermission.value.tools?.default
    : (config.value.tools?.default ?? inheritedPermission.value?.tools?.default)
  return isMode(mode) ? mode : undefined
})
const toolDefaultValueLabel = computed(() =>
  policyModeLabel(
    cleanResolvedPermission.value
      ? cleanResolvedPermission.value.tools?.default
      : (config.value.tools?.default ?? inheritedPermission.value?.tools?.default),
  ),
)
function readOnlyPolicy(policy: PermissionConfig | null): PermissionMode | undefined {
  const rule = policy?.tools?.rules?.['*']
  if (!rule || typeof rule !== 'object' || Array.isArray(rule)) return undefined
  // Runtime accepts these class aliases in insertion order; the last one wins.
  let mode: PermissionMode | undefined
  for (const [name, value] of Object.entries(rule)) {
    if (isReadOnlyClass(name) && isMode(value)) mode = value
  }
  return mode
}
const displayedReadOnlyPolicy = computed(() =>
  cleanResolvedPermission.value
    ? readOnlyPolicy(cleanResolvedPermission.value)
    : Object.hasOwn(config.value.tools?.rules || {}, '*')
      ? readOnlyPolicy(config.value)
      : readOnlyPolicy(inheritedPermission.value),
)
function invalidReadOnlyPolicy(policy: PermissionConfig | null): boolean {
  const group = policy?.tools?.rules?.['*']
  if (group === undefined) return false
  return (
    !group ||
    typeof group !== 'object' ||
    Array.isArray(group) ||
    Object.entries(group).some(([name, mode]) => !isReadOnlyClass(name) || !isMode(mode))
  )
}
const displayedReadOnlyInvalid = computed(() =>
  invalidReadOnlyPolicy(Object.hasOwn(config.value.tools?.rules || {}, '*') ? config.value : inheritedPermission.value),
)
const displayedReadOnlyUnknown = computed(
  () =>
    !Object.hasOwn(config.value.tools?.rules || {}, '*') &&
    inheritedPermission.value === null &&
    cleanResolvedPermission.value === null &&
    selectedSource.value !== 'effective',
)
const displayedReadOnlyMode = computed<PermissionMode | undefined>(() =>
  displayedReadOnlyInvalid.value || displayedReadOnlyUnknown.value
    ? undefined
    : displayedReadOnlyPolicy.value === 'auto'
      ? displayedToolDefault.value
      : displayedReadOnlyPolicy.value || 'allow',
)
const readOnlyEditorInvalid = computed(() => invalidReadOnlyPolicy(config.value))
const readOnlyInheritanceBlocked = computed(() => {
  const group = config.value.tools?.rules?.['*']
  // Restoring this one class cannot remove unrelated legacy entries. They must
  // be repaired in advanced configuration before the group can inherit.
  return Boolean(group && typeof group === 'object' && Object.keys(group).some((name) => !isReadOnlyClass(name)))
})
const readOnlyEditorMode = computed(() =>
  readOnlyEditorInvalid.value
    ? '__invalid__'
    : readOnlyPolicy(config.value) || (Object.hasOwn(config.value.tools?.rules || {}, '*') ? 'allow' : ''),
)
const readOnlyModeOptions = computed(() =>
  modeOptions.value.map((option) =>
    option.value === 'auto'
      ? {
          ...option,
          label: `${toolDefaultValueLabel.value} · ${st('Follow tool default')}`,
          description: st('Read-only Auto follows the default tool policy; named tool rules take precedence.'),
        }
      : option,
  ),
)
function modeLabel(mode: unknown): string {
  return modeOptions.value.find((option) => option.value === mode)?.label || '—'
}
function policyModeLabel(mode: unknown): string {
  return mode == null ? st('Unavailable') : isMode(mode) ? modeLabel(mode) : st('Invalid configuration')
}
function pathModeLabel(mode: unknown): string {
  return mode == null
    ? st('Unavailable')
    : pathModeOptions.value.find((option) => option.value === mode)?.label || st('Invalid configuration')
}
function displayedPathMode(scope: 'workspace' | 'external') {
  if (cleanResolvedPermission.value) return cleanResolvedPermission.value.path?.[scope]
  return config.value.path?.[scope] ?? inheritedPermission.value?.path?.[scope]
}
function toolInheritanceLabel() {
  return `${baseSourceLabel.value} · ${policyModeLabel(inheritedPermission.value?.tools?.default)}`
}
function readOnlyInheritedValueLabel() {
  if (inheritedPermission.value === null) return st('Unavailable')
  if (invalidReadOnlyPolicy(inheritedPermission.value)) return st('Invalid configuration')
  const parent = readOnlyPolicy(inheritedPermission.value)
  const mode = parent === 'auto' ? displayedToolDefault.value : parent || 'allow'
  return mode ? modeLabel(mode) : toolDefaultValueLabel.value
}
function readOnlyInheritanceLabel() {
  return `${baseSourceLabel.value} · ${readOnlyInheritedValueLabel()}`
}
const inheritedSourceLabel = computed(() =>
  selectedSource.value === 'global'
    ? defaultPermissionLabel()
    : selectedSource.value === 'workspace'
      ? st('Global layer')
      : st('Parent configuration'),
)
function defaultPermissionLabel() {
  const profile = permissionProfiles.value?.[DEFAULT_PERMISSION_PROFILE_ID]
  return profile ? permissionProfileDisplayName(DEFAULT_PERMISSION_PROFILE_ID, profile) : st('Built-in default')
}
const baseSourceLabel = computed(() => (selectedSource.value === 'global' ? st('Default') : st('Inherit')))
const restoreSourceLabel = computed(() =>
  selectedSource.value === 'global' ? st('Reset to defaults') : st('Use inherited value'),
)
function pathSource(scope: 'workspace' | 'external'): 'inherit' | 'local' {
  return modeAt(scope) === '' && !localSourceIntents.value.has('path.' + scope) ? 'inherit' : 'local'
}
const toolDefaultSource = computed(() =>
  toolDefaultEditorMode.value === '' && !localSourceIntents.value.has('tools.default') ? 'inherit' : 'local',
)
const readOnlySource = computed(() => toolRuleSource('*'))
const readOnlyValueMode = computed(() =>
  displayedReadOnlyUnknown.value
    ? '__unavailable__'
    : displayedReadOnlyInvalid.value
      ? '__invalid__'
      : displayedReadOnlyPolicy.value || 'allow',
)
const readOnlyValueLabel = computed(() =>
  displayedReadOnlyInvalid.value
    ? st('Invalid configuration')
    : displayedReadOnlyMode.value
      ? modeLabel(displayedReadOnlyMode.value)
      : st('Unavailable'),
)
function setPathSource(scope: 'workspace' | 'external', source: 'inherit' | 'local') {
  if (selectedSource.value === 'global' && source === 'local') return
  if (formDisabled.value || pathSource(scope) === source) return
  if (source === 'inherit') {
    setLocalSourceIntent('path.' + scope, false)
    writePathDefault(scope, '')
  } else {
    const mode = inheritedPermission.value?.path?.[scope]
    if (isPathPermissionMode(mode)) writePathDefault(scope, mode)
    else setLocalSourceIntent('path.' + scope, true)
  }
}
function setToolDefaultSource(source: 'inherit' | 'local') {
  if (selectedSource.value === 'global' && source === 'local') return
  if (formDisabled.value || toolDefaultSource.value === source) return
  if (source === 'inherit') {
    setLocalSourceIntent('tools.default', false)
    writeToolDefault('')
  } else if (isMode(inheritedPermission.value?.tools?.default))
    writeToolDefault(inheritedPermission.value.tools.default)
  else setLocalSourceIntent('tools.default', true)
}
function setReadOnlySource(source: 'inherit' | 'local') {
  if (selectedSource.value === 'global' && source === 'local') return
  if (formDisabled.value || readOnlySource.value === source) return
  if (source === 'inherit') {
    setLocalSourceIntent('tools.rules.*', false)
    writeReadOnlyValue('')
  } else if (inheritedPermission.value && !displayedReadOnlyInvalid.value)
    writeReadOnlyValue(readOnlyPolicy(inheritedPermission.value) || 'allow')
  else setLocalSourceIntent('tools.rules.*', true)
}

const ruleRowsByKind = computed<Record<PermissionRuleKind, PermissionRuleRow[]>>(() => {
  const rows: Record<PermissionRuleKind, PermissionRuleRow[]> = {
    filesystem: [],
    network: [],
    'tool-name': [],
    command: [],
  }
  const localSource = selectedSource.value === 'effective' ? 'effective' : 'local'
  const global = selectedSource.value === 'global'
  const sourceLabel = (source: PermissionRuleRow['source']) =>
    global
      ? ''
      : source === 'inherited'
        ? inheritedSourceLabel.value
        : source === 'effective'
          ? st('Effective')
          : profileEditor.value
            ? st('This profile')
            : st('This scope')
  function add(
    kind: PermissionRuleKind,
    target: string,
    condition: string,
    mode: string,
    source: PermissionRuleRow['source'],
    key: string,
  ) {
    const editable = canEdit.value && (global || source === 'local')
    const ownEditable = canEdit.value && source === 'local'
    const inheritedMatch =
      kind === 'filesystem'
        ? Object.hasOwn(inheritedPermission.value?.path?.rules || {}, target)
        : kind === 'tool-name'
          ? hasToolEntry(inheritedPermission.value?.tools?.names, target)
          : false
    const sourceAction =
      canEdit.value && !global && kind !== 'command' && source !== 'effective'
        ? source === 'inherited'
          ? ('local' as const)
          : inheritedMatch
            ? ('inherit' as const)
            : undefined
        : undefined
    const sourceActionLabel = sourceAction
      ? (sourceAction === 'local'
          ? profileEditor.value
            ? st('Set in this profile')
            : st('Set in this layer')
          : restoreSourceLabel.value) +
        ' · ' +
        target
      : undefined
    rows[kind].push({
      key,
      kind,
      target,
      condition,
      mode,
      source,
      sourceLabel: sourceLabel(source),
      readOnly: !global && source === 'inherited',
      modeLabel: kind === 'filesystem' ? pathModeLabel(mode) : modeLabel(mode),
      targetEditable: ownEditable && !(global && inheritedMatch) && kind !== 'network',
      selectable: ownEditable && kind !== 'network',
      removable: ownEditable && (kind === 'command' || !inheritedMatch),
      sourceAction,
      sourceActionLabel,
      resetAction: global && ownEditable && inheritedMatch,
      resetActionLabel: global && ownEditable && inheritedMatch ? st('Reset to defaults') + ': ' + target : undefined,
      modeOptions:
        editable && kind !== 'network'
          ? kind === 'filesystem'
            ? pathModeOptions.value
            : modeOptions.value
          : undefined,
    })
  }
  for (const [path, mode] of pathRules.value)
    add('filesystem', path, '', pathRuleMode(mode), localSource, `path:${path}`)
  for (const [path, mode] of inheritedPathRules.value)
    add('filesystem', path, '', pathRuleMode(mode), 'inherited', `parent:path:${path}`)
  for (const action of ['allow', 'deny'] as const)
    for (const [index, rule] of networkRules(action).entries()) {
      const source =
        selectedSource.value === 'effective' ? 'effective' : networkSource(action) === 'local' ? 'local' : 'inherited'
      const editable = canEdit.value && !sourceIsReadOnly(networkSource(action))
      rows.network.push({
        key: `network:${action}:${index}`,
        kind: 'network',
        target: rule.to === '*' ? st('All destinations') : rule.to,
        condition: [
          rule.protocol && rule.protocol !== 'any' ? rule.protocol.toUpperCase() : st('Any protocol'),
          networkPortsLabel(rule),
          rule.except?.length ? `${st('Excluded subnets')}: ${rule.except.join(', ')}` : '',
        ]
          .filter(Boolean)
          .join(' · '),
        mode: action,
        modeLabel: modeLabel(action),
        source,
        sourceLabel: sourceLabel(source),
        readOnly: !global && source === 'inherited',
        networkAction: action,
        removable: editable,
        editAction: editable,
      })
    }
  for (const [name, mode] of toolNameRules.value) add('tool-name', name, '', mode, localSource, `name:${name}`)
  for (const [name, mode] of inheritedToolNameRules.value)
    add('tool-name', name, '', mode, 'inherited', `parent:name:${name}`)
  for (const [pattern, mode] of commandRules.value) add('command', pattern, '', mode, localSource, `command:${pattern}`)
  for (const [pattern, mode] of inheritedCommandRules.value)
    add('command', pattern, '', mode, 'inherited', `parent:command:${pattern}`)
  return rows
})
const selectedRuleKeysByKind = computed<Record<PermissionRuleKind, string[]>>(() => ({
  filesystem: selectedPathRules.value.map(([path]) => `path:${path}`),
  network: [],
  'tool-name': selectedToolNameRules.value.map(([name]) => `name:${name}`),
  command: selectedCommandRules.value.map(([pattern]) => `command:${pattern}`),
}))
const ruleGroups = computed(() =>
  [
    { section: 'filesystem', kind: 'filesystem' as const, label: st('Path Rules'), addLabel: st('Add path rule') },
    { section: 'network', kind: 'network' as const, label: st('Network Rules'), addLabel: st('Add network rule') },
    { section: 'tools', kind: 'tool-name' as const, label: st('Name Rules'), addLabel: st('Add tool name') },
    { section: 'tools', kind: 'command' as const, label: st('Command Rules'), addLabel: st('Add command rule') },
  ].map((group) => ({
    ...group,
    rows: ruleRowsByKind.value[group.kind],
    selectedKeys: selectedRuleKeysByKind.value[group.kind],
  })),
)
const domainColumns = computed(() => {
  const domains = sections.value.map((section) => ({
    ...section,
    groups: ruleGroups.value.filter((group) => group.section === section.id),
  }))
  return [
    { id: 'resources', domains: domains.filter((domain) => domain.id !== 'tools') },
    { id: 'tools', domains: domains.filter((domain) => domain.id === 'tools') },
  ]
})

function selectRuleRow(row: PermissionRuleRow, selected: boolean) {
  if (!row.selectable) return
  const state =
    row.kind === 'filesystem'
      ? selectedPathRuleKeys
      : row.kind === 'tool-name'
        ? selectedToolNameRuleKeys
        : selectedCommandRuleKeys
  const keys = new Set(state.value)
  if (selected) keys.add(row.target)
  else keys.delete(row.target)
  state.value = keys
}
function selectVisibleRuleRows(kind: PermissionRuleKind, rows: PermissionRuleRow[], selected: boolean) {
  for (const row of rows) if (row.kind === kind) selectRuleRow(row, selected)
}
function setRuleRowMode(row: PermissionRuleRow, mode: string) {
  if (row.kind === 'filesystem') setPathRuleMode(row.target, mode)
  else if (row.kind === 'tool-name') setToolNameMode(row.target, mode)
  else if (row.kind === 'command') setCommandMode(row.target, mode)
}
function renameRuleTarget(row: PermissionRuleRow, event: Event) {
  if (row.kind === 'filesystem') renamePathRule(row.target, event)
  else if (row.kind === 'tool-name') renameToolName(row.target, event)
  else if (row.kind === 'command') renameCommandRule(row.target, event)
}
function editRuleRow(row: PermissionRuleRow) {
  if (row.kind === 'network' && row.networkAction) editNetworkRule(row.networkAction, Number(row.key.split(':').at(-1)))
}
function removeRuleRow(row: PermissionRuleRow) {
  if (row.kind === 'filesystem') removePathRule(row.target)
  else if (row.kind === 'tool-name') removeToolName(row.target)
  else if (row.kind === 'command') removeCommandRule(row.target)
  else if (row.networkAction) removeNetworkRule(row.networkAction, Number(row.key.split(':').at(-1)))
}
async function changeRuleSource(row: PermissionRuleRow) {
  if (selectedSource.value === 'global') return
  if (formDisabled.value || !row.sourceAction || row.kind === 'network') return
  if (row.sourceAction === 'inherit') removeRuleRow(row)
  else if (
    (row.kind === 'filesystem' && isPathPermissionMode(row.mode)) ||
    (row.kind === 'tool-name' && isMode(row.mode))
  )
    mutate((next) => {
      if (row.kind === 'filesystem') ((next.path ||= {}).rules ||= {})[row.target] = row.mode as PathPermissionMode
      else writeToolEntry(((next.tools ||= {}).names ||= {}), row.target, row.mode as PermissionMode)
    })
}
async function resetRuleRow(row: PermissionRuleRow) {
  if (selectedSource.value !== 'global' || formDisabled.value || !row.resetAction) return
  if (row.kind === 'filesystem' || row.kind === 'tool-name') removeRuleRow(row)
}
function copyCommandRules() {
  if (selectedSource.value === 'global' || formDisabled.value || commandSource.value === 'local') return
  if (!inheritedPermission.value) {
    setLocalSourceIntent(commandSourceKey, true)
    return
  }
  mutate((next) => {
    const rules = ((next.tools ||= {}).rules ||= {})
    rules[SHELL_COMMAND_RULE_GROUP] = clone(commandRuleGroup(inheritedPermission.value) ?? {})
  })
}
async function restoreCommandRules() {
  if (formDisabled.value) return
  if (!hasToolEntry(config.value.tools?.rules, SHELL_COMMAND_RULE_GROUP)) {
    setLocalSourceIntent(commandSourceKey, false)
    return
  }
  const context = contextKey.value
  const before = JSON.stringify(commandRuleGroup(config.value))
  const global = selectedSource.value === 'global'
  const confirmed = await confirmAction({
    title: global ? st('Restore default command rules?') : st('Restore inherited command rules?'),
    description: st('This removes all command rules defined in this layer. Save to apply.'),
    confirmText: restoreSourceLabel.value,
    cancelText: st('Cancel'),
  })
  if (
    !confirmed ||
    formDisabled.value ||
    context !== contextKey.value ||
    before !== JSON.stringify(commandRuleGroup(config.value))
  )
    return
  mutate((next) => {
    if (!next.tools?.rules) return
    delete next.tools.rules[SHELL_COMMAND_RULE_GROUP]
    if (!Object.keys(next.tools.rules).length) delete next.tools.rules
    if (isEmptyToolSection(next.tools)) delete next.tools
  })
}
function beginRule(kind: PermissionRuleKind) {
  if (kind === 'network') {
    const action = networkRuleLists.find((field) => !sourceIsReadOnly(networkSource(field)))
    if (!action) return
    editNetworkRule(action)
    return
  }
  newRuleKind.value = newRuleKind.value === kind ? '' : kind
}
function matchesQuery(value: string, query: string) {
  return value.toLowerCase().includes(query.trim().toLowerCase())
}
function normalizedClassKeyword(value: string): string {
  return value
    .trim()
    .replace(/[A-Z]/g, (letter) => letter.toLowerCase())
    .replaceAll('_', '-')
}
function isReadOnlyClass(value: string): boolean {
  return ['read-only', 'readonly'].includes(normalizedClassKeyword(value))
}
function toolEntry<T>(entries: Record<string, T> | undefined, name: string): T | undefined {
  return canonicalPermissionToolEntries(entries).find(([key]) => key === canonicalPermissionToolName(name))?.[1]
}
function hasToolEntry<T>(entries: Record<string, T> | undefined, name: string): boolean {
  return Object.keys(entries || {}).some(
    (key) => canonicalPermissionToolName(key) === canonicalPermissionToolName(name),
  )
}
function writeToolEntry<T>(entries: Record<string, T>, name: string, value: T | undefined) {
  const canonical = canonicalPermissionToolName(name)
  for (const key of Object.keys(entries)) if (canonicalPermissionToolName(key) === canonical) delete entries[key]
  if (value !== undefined) entries[canonical] = value
}
const selectedPathRules = computed(() => pathRules.value.filter(([path]) => selectedPathRuleKeys.value.has(path)))
const selectedToolNameRules = computed(() =>
  toolNameRules.value.filter(([name]) => selectedToolNameRuleKeys.value.has(name)),
)
const selectedCommandRules = computed(() =>
  commandRules.value.filter(([pattern]) => selectedCommandRuleKeys.value.has(pattern)),
)
const allPathRulesSelected = computed(() =>
  areAllSelected(
    visiblePathRules.value.map(([path]) => path),
    selectedPathRuleKeys.value,
  ),
)
const allToolNameRulesSelected = computed(() =>
  areAllSelected(
    visibleToolNameRules.value.map(([name]) => name),
    selectedToolNameRuleKeys.value,
  ),
)
function areAllSelected(keys: string[], selected: Set<string>): boolean {
  return keys.length > 0 && keys.every((key) => selected.has(key))
}

function toggleSelection(selection: Ref<Set<string>>, key: string, event: Event) {
  const next = new Set(selection.value)
  if ((event.target as HTMLInputElement).checked) next.add(key)
  else next.delete(key)
  selection.value = next
}

function toggleAllSelection(selection: Ref<Set<string>>, keys: string[]) {
  if (areAllSelected(keys, selection.value)) {
    const removing = new Set(keys)
    selection.value = new Set([...selection.value].filter((key) => !removing.has(key)))
  } else {
    selection.value = new Set([...selection.value, ...keys])
  }
}

function clearSelection(selection: Ref<Set<string>>) {
  selection.value = new Set()
}

function pruneSelection(selection: Ref<Set<string>>, keys: string[]) {
  const available = new Set(keys)
  const next = new Set([...selection.value].filter((key) => available.has(key)))
  if (next.size !== selection.value.size) selection.value = next
}

function togglePathRule(path: string, event: Event) {
  toggleSelection(selectedPathRuleKeys, path, event)
}
function toggleToolNameRule(name: string, event: Event) {
  toggleSelection(selectedToolNameRuleKeys, name, event)
}
function toggleAllPathRules() {
  toggleAllSelection(
    selectedPathRuleKeys,
    visiblePathRules.value.map(([path]) => path),
  )
}
function toggleAllToolNameRules() {
  toggleAllSelection(
    selectedToolNameRuleKeys,
    visibleToolNameRules.value.map(([name]) => name),
  )
}
function clearPathRuleSelection() {
  clearSelection(selectedPathRuleKeys)
}
function clearToolNameRuleSelection() {
  clearSelection(selectedToolNameRuleKeys)
}
function clearCommandRuleSelection() {
  clearSelection(selectedCommandRuleKeys)
}
function applySelectedPathMode() {
  setSelectedPathMode(bulkPathMode.value)
}
function clearAllRuleSelections() {
  clearSelection(selectedPathRuleKeys)
  clearSelection(selectedToolNameRuleKeys)
  clearSelection(selectedCommandRuleKeys)
}

watch(
  [visiblePathRules, visibleToolNameRules, commandRules],
  () => {
    pruneSelection(
      selectedPathRuleKeys,
      visiblePathRules.value.map(([path]) => path),
    )
    pruneSelection(
      selectedToolNameRuleKeys,
      visibleToolNameRules.value.map(([name]) => name),
    )
    pruneSelection(
      selectedCommandRuleKeys,
      commandRules.value.map(([pattern]) => pattern),
    )
  },
  { flush: 'sync' },
)

function setReadOnlyMode(value: string) {
  if (sourceValueDisabled(readOnlySource.value)) return
  writeReadOnlyValue(value)
}
function writeReadOnlyValue(value: string) {
  setLocalSourceIntent('tools.rules.*', false)
  if (!isMode(value) && readOnlyInheritanceBlocked.value) return
  mutate((next) => {
    const tools = (next.tools ||= {})
    const groups = (tools.rules ||= {})
    const current = groups['*']
    const rules = current && typeof current === 'object' ? current : {}
    for (const key of Object.keys(rules)) if (isReadOnlyClass(key)) delete rules[key]
    if (isMode(value)) rules['read-only'] = value
    writeToolEntry(groups, '*', Object.keys(rules).length ? rules : undefined)
    if (!Object.keys(groups).length) delete tools.rules
    if (isEmptyToolSection(tools)) delete next.tools
  })
}

function isMode(value: unknown): value is PermissionMode {
  return value === 'allow' || value === 'auto' || value === 'ask' || value === 'deny'
}

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T
}

function asConfig(value: unknown): PermissionConfig {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return {}
  try {
    return parsePermissionConfig(value)
  } catch {
    // Keep invalid legacy values visible in the raw editor so they can be repaired.
    return clone(value as PermissionConfig)
  }
}

function syncRaw(publish = false) {
  rawJson.value = JSON.stringify(config.value, null, 2)
  rawJsonError.value = ''
  networkJson.value = JSON.stringify(config.value.network || { version: 2 }, null, 2)
  networkJsonError.value = ''
  if (publish && profileEditor.value) emit('update:draftPermission', clone(config.value))
}

function mutate(mutator: (next: PermissionConfig) => void): boolean {
  if (editingDisabled.value) return false
  if (rawPending.value && !applyRawJson()) return false
  const next = clone(config.value)
  mutator(next)
  try {
    config.value = parsePermissionConfig(next)
    settleSourceIntents(config.value)
    syncRaw(true)
    error.value = ''
    return true
  } catch (reason) {
    // Invalid intermediate edits stay local so several legacy rules can be
    // repaired one by one. Save always validates the complete draft again.
    config.value = next
    settleSourceIntents(config.value)
    syncRaw(true)
    error.value = permissionConfigErrorMessage(reason, st)
    rawJsonError.value = error.value
    return true
  }
}

function setPluginPermission(id: string, enabled: boolean | undefined) {
  if (formDisabled.value) return
  mutate((next) => {
    const plugins = { ...next.plugins }
    if (enabled === undefined) delete plugins[id]
    else plugins[id] = enabled
    if (Object.keys(plugins).length) next.plugins = plugins
    else delete next.plugins
  })
}

function modeAt(scope: 'workspace' | 'external'): PathPermissionMode | '' | '__invalid__' {
  const mode = config.value.path?.[scope]
  if (mode == null) return ''
  return isPathPermissionMode(mode) ? mode : '__invalid__'
}

function pathInheritanceLabel(scope: 'workspace' | 'external'): string {
  const mode = inheritedPermission.value?.path?.[scope]
  if (mode == null) return baseSourceLabel.value
  return `${baseSourceLabel.value} · ${isPathPermissionMode(mode) ? pathModeLabel(mode) : st('Invalid configuration')}`
}

function setPathDefault(scope: 'workspace' | 'external', value: string) {
  if (sourceValueDisabled(pathSource(scope))) return
  writePathDefault(scope, value)
}
function writePathDefault(scope: 'workspace' | 'external', value: string) {
  setLocalSourceIntent('path.' + scope, false)
  mutate((next) => {
    next.path ||= {}
    if (isPathPermissionMode(value)) next.path[scope] = value
    else delete next.path[scope]
    if (isEmptyPathSection(next.path)) delete next.path
  })
}

// An emptied section is dropped whole - which is exactly what "every entry is
// showing its built-in default" looks like on disk.
function isEmptyPathSection(path: PermissionConfig['path']): boolean {
  return !path?.workspace && !path?.external && !Object.keys(path?.rules || {}).length
}

function isEmptyToolSection(tools: PermissionConfig['tools']): boolean {
  return !tools?.default && !Object.keys(tools?.names || {}).length && !Object.keys(tools?.rules || {}).length
}

function pathRuleMode(rule: unknown): PathPermissionMode | '' {
  return isPathPermissionMode(rule) ? rule : ''
}

function setPathRuleMode(path: string, value: string) {
  if (formDisabled.value || (selectedSource.value !== 'global' && !Object.hasOwn(config.value.path?.rules || {}, path)))
    return
  mutate((next) => {
    next.path ||= {}
    next.path.rules ||= {}
    if (isPathPermissionMode(value)) next.path.rules[path] = value
    else delete next.path.rules[path]
    if (isEmptyPathSection(next.path)) delete next.path
  })
}

function removePathRule(path: string) {
  if (formDisabled.value || !Object.hasOwn(config.value.path?.rules || {}, path)) return
  mutate((next) => {
    if (next.path?.rules) delete next.path.rules[path]
    if (isEmptyPathSection(next.path)) delete next.path
  })
}

function setSelectedPathMode(value: string) {
  const paths = selectedPathRules.value.map(([path]) => path)
  if (!paths.length || !isPathPermissionMode(value)) return
  mutate((next) => {
    for (const path of paths) {
      if (next.path?.rules?.[path]) next.path.rules[path] = value
    }
  })
}

function removeSelectedPathRules() {
  const paths = selectedPathRules.value.map(([path]) => path)
  if (!paths.length) return
  mutate((next) => {
    for (const path of paths) {
      if (next.path?.rules) delete next.path.rules[path]
    }
    if (isEmptyPathSection(next.path)) delete next.path
  })
  clearSelection(selectedPathRuleKeys)
}

function renamePathRule(path: string, event: Event) {
  const input = event.target as HTMLInputElement
  let nextPath: string
  try {
    nextPath = canonicalPermissionPath(input.value)
  } catch (reason) {
    error.value = permissionConfigErrorMessage(reason, st)
    input.value = path
    return
  }
  if (nextPath === path) {
    input.value = path
    return
  }
  if (
    selectedSource.value !== 'global' &&
    Object.hasOwn(inheritedPermission.value?.path?.rules || {}, nextPath) &&
    !Object.hasOwn(config.value.path?.rules || {}, nextPath)
  ) {
    error.value = inheritedEditError()
    input.value = path
    return
  }
  if (Object.prototype.hasOwnProperty.call(config.value.path?.rules || {}, nextPath)) {
    error.value = st('Path rule already exists: {nextPath}', { nextPath: nextPath })
    input.value = path
    return
  }
  mutate((next) => {
    const rules = next.path?.rules
    if (!rules || !Object.prototype.hasOwnProperty.call(rules, path)) return
    const value = rules[path]
    delete rules[path]
    rules[nextPath] = value
  })
  input.value = nextPath
}

function networkMode(field: NetworkDefault): NetworkAction | undefined {
  return displayedNetwork.value?.[field]
}

function networkSource(field: NetworkField): 'inherit' | 'local' {
  return Object.hasOwn(config.value.network || {}, field) || localSourceIntents.value.has('network.' + field)
    ? 'local'
    : 'inherit'
}

function networkCanCustomize(field: NetworkField): boolean {
  return displayedNetwork.value?.[field] !== undefined
}

function networkRuleListLabel(action: NetworkRuleList): string {
  return action === 'allow' ? st('Allowed destinations') : st('Denied destinations')
}

function networkBaseValueLabel(field: NetworkField): string {
  const value = inheritedNetwork.value?.[field]
  return value === undefined
    ? st('Unavailable')
    : Array.isArray(value)
      ? st('{count} rules', { count: value.length })
      : modeLabel(value)
}

function networkParentError(): string {
  return st('Network defaults could not be loaded. Refresh before customizing them.')
}

function setNetworkSource(field: NetworkField, value: 'inherit' | 'local') {
  if (selectedSource.value === 'global' && value === 'local') return
  if (formDisabled.value || networkRuleDraft.value || networkSource(field) === value) return
  if (value === 'inherit') {
    setLocalSourceIntent('network.' + field, false)
    mutate((next) => {
      if (!next.network) return
      delete next.network[field]
      if (!networkFields.some((key) => Object.hasOwn(next.network!, key))) delete next.network
    })
  } else {
    const current = displayedNetwork.value?.[field]
    if (current === undefined) {
      setLocalSourceIntent('network.' + field, true)
      return
    }
    mutate((next) => {
      next.network = { ...next.network, version: 2, [field]: clone(current) }
    })
  }
}

function setNetworkMode(field: NetworkDefault, value: string) {
  if (sourceValueDisabled(networkSource(field)) || networkRuleDraft.value || (value !== 'allow' && value !== 'deny'))
    return
  setLocalSourceIntent('network.' + field, false)
  mutate((next) => {
    const network = (next.network ||= { version: 2 })
    network[field] = value
  })
}

function networkRules(action: NetworkRuleList): NetworkRule[] {
  const rules = displayedNetwork.value?.[action]
  return Array.isArray(rules) ? rules : []
}

function networkPortsLabel(rule: NetworkRule): string {
  return rule.protocol === 'icmp' ? '—' : rule.ports?.map(String).join(', ') || st('All ports')
}

function editNetworkRule(action: NetworkRuleList, index: number | null = null) {
  if (sourceValueDisabled(networkSource(action)) || networkRuleDraft.value) return
  const rule = index === null ? undefined : networkRules(action)[index]
  if (index !== null && !rule) return
  newRuleKind.value = 'network'
  networkRuleDraft.value = {
    action,
    index,
    to: rule?.to || '',
    protocol: rule?.protocol || (index === null && action === 'allow' ? 'tcp' : 'any'),
    ports: rule?.ports?.map(String).join(', ') || '',
    except: rule?.except?.join(', ') || '',
  }
  networkRuleError.value = ''
}

function cancelNetworkRule() {
  networkRuleDraft.value = null
  networkRuleError.value = ''
  if (newRuleKind.value === 'network') newRuleKind.value = ''
}

function setNetworkRuleProtocol(value: string) {
  if (
    !networkRuleDraft.value ||
    sourceValueDisabled(networkSource(networkRuleDraft.value.action)) ||
    !['tcp', 'udp', 'icmp', 'any'].includes(value)
  )
    return
  networkRuleDraft.value.protocol = value as NetworkProtocol
  if (value === 'icmp') networkRuleDraft.value.ports = ''
  networkRuleError.value = ''
}

function applyNetworkRule(): boolean {
  const draft = networkRuleDraft.value
  if (!draft || sourceValueDisabled(networkSource(draft.action)) || rawPending.value || networkPending.value)
    return false
  try {
    const rule: NetworkRule = { to: draft.to.trim() }
    if (draft.protocol !== 'any') rule.protocol = draft.protocol
    if (draft.ports.trim())
      rule.ports = draft.ports.split(',').map((entry) => {
        const value = entry.trim()
        return /^(0|[1-9][0-9]*)$/.test(value) ? Number(value) : value
      })
    if (draft.except.trim()) rule.except = draft.except.split(',').map((entry) => entry.trim())
    const next = clone(config.value)
    const network = (next.network ||= { version: 2 })
    // Editing one list fixes only that list in this layer, including an empty list.
    const rules = (network[draft.action] ||= clone(networkRules(draft.action)))
    if (draft.index === null) rules.push(rule)
    else rules[draft.index] = rule
    const checked = parsePermissionConfig(next)
    config.value = checked
    setLocalSourceIntent('network.' + draft.action, false)
    cancelNetworkRule()
    syncRaw(true)
    error.value = ''
    return true
  } catch (reason) {
    networkRuleError.value = permissionConfigErrorMessage(reason, st)
    return false
  }
}

function removeNetworkRule(action: NetworkRuleList, index: number) {
  if (sourceValueDisabled(networkSource(action)) || networkRuleDraft.value || !networkRules(action)[index]) return
  mutate((next) => {
    const network = (next.network ||= { version: 2 })
    const rules = (network[action] ||= clone(networkRules(action)))
    rules.splice(index, 1)
  })
}

function applyNetworkJson(): boolean {
  if (networkJsonReadOnly.value) return false
  try {
    const next = clone(config.value)
    next.network = parseNetworkPermissionConfig(JSON.parse(networkJson.value))
    assertEditableOverrides(next)
    config.value = parsePermissionConfig(next)
    settleSourceIntents(config.value)
    syncRaw()
    error.value = ''
    return true
  } catch (reason) {
    networkJsonError.value = permissionConfigErrorMessage(reason, st)
    return false
  }
}

function useNetworkExample() {
  if (networkJsonReadOnly.value || !networkFullyLocal.value) return
  networkJson.value = JSON.stringify(
    { version: 2, default: 'deny', allow: [{ to: '192.0.2.10', protocol: 'tcp', ports: [443, '8000-8099'] }] },
    null,
    2,
  )
  networkJsonError.value = ''
}

function setToolDefault(value: string) {
  if (sourceValueDisabled(toolDefaultSource.value)) return
  writeToolDefault(value)
}
function writeToolDefault(value: string) {
  setLocalSourceIntent('tools.default', false)
  mutate((next) => {
    next.tools ||= {}
    if (isMode(value)) next.tools.default = value
    else delete next.tools.default
    if (isEmptyToolSection(next.tools)) delete next.tools
  })
}

function setToolNameMode(name: string, value: string) {
  if (formDisabled.value || (selectedSource.value !== 'global' && !hasToolEntry(config.value.tools?.names, name)))
    return
  mutate((next) => {
    next.tools ||= {}
    next.tools.names ||= {}
    writeToolEntry(next.tools.names, name, isMode(value) ? value : undefined)
  })
}

function removeToolName(name: string) {
  if (formDisabled.value || !hasToolEntry(config.value.tools?.names, name)) return
  mutate((next) => {
    if (next.tools?.names) writeToolEntry(next.tools.names, name, undefined)
    if (isEmptyToolSection(next.tools)) delete next.tools
  })
}

function setSelectedToolNameMode(value: string) {
  const names = selectedToolNameRules.value.map(([name]) => name)
  if (!names.length || !isMode(value)) return
  mutate((next) => {
    next.tools ||= {}
    next.tools.names ||= {}
    for (const name of names) writeToolEntry(next.tools.names, name, value)
  })
}

function removeSelectedToolNameRules() {
  const names = selectedToolNameRules.value.map(([name]) => name)
  if (!names.length) return
  mutate((next) => {
    for (const name of names) {
      if (next.tools?.names) writeToolEntry(next.tools.names, name, undefined)
    }
    if (isEmptyToolSection(next.tools)) delete next.tools
  })
  clearSelection(selectedToolNameRuleKeys)
}

function renameToolName(name: string, event: Event) {
  const input = event.target as HTMLInputElement
  const nextName = canonicalPermissionToolName(input.value.trim())
  if (!nextName || nextName === name) {
    input.value = name
    return
  }
  if (
    selectedSource.value !== 'global' &&
    hasToolEntry(inheritedPermission.value?.tools?.names, nextName) &&
    !hasToolEntry(config.value.tools?.names, nextName)
  ) {
    error.value = inheritedEditError()
    input.value = name
    return
  }
  if (hasToolEntry(config.value.tools?.names, nextName)) {
    error.value = st('Tool name rule already exists: {nextName}', { nextName: nextName })
    input.value = name
    return
  }
  mutate((next) => {
    const names = next.tools?.names
    if (!names || !hasToolEntry(names, name)) return
    const value = toolEntry(names, name)
    writeToolEntry(names, name, undefined)
    writeToolEntry(names, nextName, value)
  })
  error.value = ''
  input.value = nextName
}

function setCommandMode(pattern: string, value: string) {
  if (sourceValueDisabled(commandSource.value) || !isMode(value)) return
  mutate((next) => setCommandRuleInConfig(next, pattern, value))
}

function removeCommandRule(pattern: string) {
  if (sourceValueDisabled(commandSource.value)) return
  mutate((next) => setCommandRuleInConfig(next, pattern))
}

function setCommandRuleInConfig(next: PermissionConfig, pattern: string, mode?: PermissionMode) {
  const groups = ((next.tools ||= {}).rules ||= {})
  const current = commandRuleGroup(next) || commandRuleGroup(inheritedPermission.value) || {}
  const entries = new Map(Object.entries(current))
  if (mode) entries.set(pattern, mode)
  else entries.delete(pattern)
  groups[SHELL_COMMAND_RULE_GROUP] = Object.fromEntries(entries)
}

function setSelectedCommandMode(value: string) {
  if (sourceValueDisabled(commandSource.value) || !isMode(value)) return
  mutate((next) => {
    for (const [pattern] of selectedCommandRules.value) setCommandRuleInConfig(next, pattern, value)
  })
}

function removeSelectedCommandRules() {
  if (sourceValueDisabled(commandSource.value)) return
  mutate((next) => {
    for (const [pattern] of selectedCommandRules.value) setCommandRuleInConfig(next, pattern)
  })
  clearSelection(selectedCommandRuleKeys)
}

function renameCommandRule(pattern: string, event: Event) {
  const input = event.target as HTMLInputElement
  const name = input.value.trim()
  const rules = commandRuleGroup(config.value)
  if (
    sourceValueDisabled(commandSource.value) ||
    !name ||
    name === pattern ||
    !rules ||
    !Object.hasOwn(rules, pattern)
  ) {
    input.value = pattern
    return
  }
  if (Object.hasOwn(rules, name)) {
    error.value = st('Command pattern already exists: {pattern}', { pattern: name })
    input.value = pattern
    return
  }
  if (
    mutate((next) => {
      next.tools!.rules![SHELL_COMMAND_RULE_GROUP] = renamePermissionRule(rules, pattern, name)
    })
  )
    input.value = name
  else input.value = pattern
}

function addPathRule() {
  if (formDisabled.value) return
  let path: string
  try {
    path = canonicalPermissionPath(newPath.value)
  } catch (reason) {
    error.value = permissionConfigErrorMessage(reason, st)
    return
  }
  if (
    selectedSource.value !== 'global' &&
    Object.hasOwn(inheritedPermission.value?.path?.rules || {}, path) &&
    !Object.hasOwn(config.value.path?.rules || {}, path)
  ) {
    error.value = inheritedEditError()
    return
  }
  if (Object.prototype.hasOwnProperty.call(config.value.path?.rules || {}, path)) {
    error.value = st('Path rule already exists: {path}', { path: path })
    return
  }
  const accepted = mutate((next) => {
    next.path ||= {}
    next.path.rules ||= {}
    next.path.rules[path] = newPathMode.value
  })
  if (accepted) newPath.value = ''
}

function addToolName() {
  if (formDisabled.value) return
  const name = canonicalPermissionToolName(newToolName.value.trim())
  if (!name) return
  if (
    selectedSource.value !== 'global' &&
    hasToolEntry(inheritedPermission.value?.tools?.names, name) &&
    !hasToolEntry(config.value.tools?.names, name)
  ) {
    error.value = inheritedEditError()
    return
  }
  if (hasToolEntry(config.value.tools?.names, name)) {
    error.value = st('Tool name rule already exists: {name}', { name: name })
    return
  }
  mutate((next) => {
    next.tools ||= {}
    next.tools.names ||= {}
    writeToolEntry(next.tools.names, name, newToolNameMode.value)
  })
  error.value = ''
  newToolName.value = ''
}

function addCommandRule() {
  const pattern = newCommandPattern.value.trim()
  if (!pattern || sourceValueDisabled(commandSource.value)) return
  const rules = commandRuleGroup(config.value) || commandRuleGroup(inheritedPermission.value)
  if (Object.hasOwn(rules || {}, pattern)) {
    error.value = st('Command pattern already exists: {pattern}', { pattern })
    return
  }
  if (mutate((next) => setCommandRuleInConfig(next, pattern, newCommandMode.value))) newCommandPattern.value = ''
}

const pendingRule = computed(() =>
  Boolean(
    networkRuleDraft.value ||
    (newRuleKind.value === 'filesystem' && newPath.value) ||
    (newRuleKind.value === 'tool-name' && newToolName.value) ||
    (newRuleKind.value === 'command' && newCommandPattern.value),
  ),
)
function applyPendingRule(): boolean {
  if (networkRuleDraft.value) return applyNetworkRule()
  if (!pendingRule.value) return true
  if (newRuleKind.value === 'filesystem') addPathRule()
  else if (newRuleKind.value === 'tool-name') addToolName()
  else addCommandRule()
  return !pendingRule.value
}

function applyRawJson(): boolean {
  if (rawJsonReadOnly.value) return false
  try {
    const parsed = JSON.parse(rawJson.value) as JsonValue
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed))
      throw new Error(st('Permission config must be a JSON object.'))
    const next = parsePermissionConfig(parsed)
    assertEditableOverrides(next)
    config.value = next
    settleSourceIntents(config.value)
    syncRaw()
    return true
  } catch (reason) {
    rawJsonError.value = permissionConfigErrorMessage(reason, st)
    rawEditorOpen.value = true
    return false
  }
}

const contextKey = computed(() => {
  if (profileEditor.value) return 'profile'
  const source = selectedSource.value
  if (source === 'workspace') return JSON.stringify([source, activeWorkspaceTarget.value])
  return JSON.stringify(source === 'session' || source === 'effective' ? [source, activeSessionId.value] : [source])
})
type PermissionDraft = {
  config: PermissionConfig
  saved: PermissionConfig
  raw: string
  network: string
  networkRule: NetworkRuleDraft | null
  sourceIntents: string[]
  revision?: string
}
const drafts = new Map<string, PermissionDraft>()
let loadedContext = ''
let loadGeneration = 0
let loadController: AbortController | undefined

function rememberDraft() {
  if (!loadedContext || !loaded.value) return
  if (dirty.value || localSourceIntents.value.size)
    drafts.set(loadedContext, {
      config: clone(config.value),
      saved: clone(savedConfig.value),
      raw: rawJson.value,
      network: networkJson.value,
      networkRule: clone(networkRuleDraft.value),
      sourceIntents: [...localSourceIntents.value],
      revision: configRevision.value,
    })
  else drafts.delete(loadedContext)
}

type PolicyRead = {
  policy: PermissionConfig
  profile?: string
  inherited?: unknown
  resolved?: unknown
  configPath?: string
  workspaceId?: number | null
  activeWorkspace?: boolean
  revision?: string
}
async function readPolicy(
  source: PermissionSource,
  sessionId: number | null,
  workspaceTarget: string,
  signal: AbortSignal,
): Promise<PolicyRead> {
  if (props.draftPermission !== undefined) return { policy: asConfig(props.draftPermission) }
  if (source === 'global' || source === 'workspace') {
    const params = new URLSearchParams({ path: 'permission' })
    if (source === 'workspace' && workspaceTarget) params.set('workspace_id', workspaceTarget)
    const profileParams = new URLSearchParams(params)
    profileParams.set('path', 'permission_profile')
    const [response, selection] = await Promise.all([
      apiJson<SettingsResponse>(`/api/v1/settings/layers/${source}?${params}`, { signal }),
      apiJson<SettingsResponse>(`/api/v1/settings/layers/${source}?${profileParams}`, { signal }),
    ])
    if (
      source === 'workspace' &&
      workspaceTarget &&
      [response, selection].some((item) => item.workspace_id !== Number(workspaceTarget))
    ) {
      throw new Error(st('The server did not return the selected workspace.'))
    }
    if (response.revision !== selection.revision)
      throw new Error(st('The configuration changed. Refresh and try again.'))
    return {
      policy: asConfig(response.value),
      profile:
        source === 'global' && typeof selection.layer_value === 'string'
          ? selection.layer_value
          : typeof selection.value === 'string'
            ? selection.value
            : '',
      inherited: runtimeSettingBaseValue(response, source),
      resolved: response.layer_value,
      configPath: response.config_path,
      workspaceId: response.workspace_id,
      activeWorkspace: response.active_workspace,
      revision: response.revision,
    }
  }
  if (sessionId) {
    const session = await apiJson<SessionStateResponse>(`/api/v1/sessions/${sessionId}/state`, { signal })
    const execution = session.execution
    const effective =
      execution?.execution?.effective_permission ||
      execution?.effective_permission ||
      execution?.context?.effective_permission
    const value =
      source === 'session'
        ? execution?.execution?.selected_permission ||
          execution?.selected_permission ||
          execution?.context?.selected_permission
        : effective
    const inherited =
      execution?.execution?.inherited_permission ||
      execution?.inherited_permission ||
      execution?.context?.inherited_permission
    const policy = asConfig(value)
    if (source === 'session') {
      const profile =
        execution?.execution?.permission_profile ??
        execution?.permission_profile ??
        execution?.context?.permission_profile
      return {
        policy,
        inherited,
        resolved: profile ? effective : undefined,
        profile,
      }
    }
    if (Object.keys(policy).length) return { policy, inherited }
  }
  if (source === 'session') return { policy: {} }
  const response = await apiJson<SettingsResponse>('/api/v1/settings?source=effective&path=permission', { signal })
  return { policy: asConfig(response.value) }
}

async function load() {
  const key = contextKey.value
  const source = settingsScope.value
  const sessionId = activeSessionId.value
  const workspaceTarget = activeWorkspaceTarget.value
  const generation = ++loadGeneration
  loadController?.abort()
  const controller = new AbortController()
  loadController = controller
  if (loadedContext !== key) {
    rememberDraft()
    loadedContext = key
    const draft = drafts.get(key)
    cancelNetworkRule()
    newRuleKind.value = ''
    inheritedPermission.value = null
    appliedProfile.value = ''
    chosenProfile.value = ''
    hasCustomPermission.value = Boolean(draft && Object.keys(draft.saved).length)
    resolvedPermission.value = null
    configPath.value = ''
    workspaceId.value = null
    workspaceConfigIsActive.value = undefined
    configRevision.value = draft?.revision
    inheritedNetworkError.value = ''
    localSourceIntents.value = new Set(draft?.sourceIntents || [])
    config.value = clone(draft?.config || {})
    savedConfig.value = clone(draft?.saved || {})
    syncRaw()
    if (draft) {
      rawJson.value = draft.raw
      networkJson.value = draft.network
      networkRuleDraft.value = clone(draft.networkRule)
    }
    loaded.value = Boolean(draft)
    clearAllRuleSelections()
  }
  loading.value = true
  error.value = ''
  saveError.value = ''
  try {
    const [policy, catalog, profiles, plugins] = await Promise.allSettled([
      readPolicy(source, sessionId, workspaceTarget, controller.signal),
      apiJson<ToolCatalogResponse>('/api/v1/plugins/surface', { signal: controller.signal }),
      !profileEditor.value && source !== 'effective'
        ? apiJson<SettingsResponse>('/api/v1/settings/layers/global?path=access_control', { signal: controller.signal })
        : null,
      apiJson<unknown>('/api/v1/plugins', { signal: controller.signal }).then(parsePluginStatusList),
    ])
    if (generation !== loadGeneration || key !== contextKey.value) return
    if (catalog.status === 'fulfilled') {
      toolCatalog.value = (Array.isArray(catalog.value?.permission_tools) ? catalog.value.permission_tools : [])
        .map((item) => ({ name: String(item?.name || '').trim(), summary: String(item?.summary || '').trim() }))
        .filter((item) => item.name)
        .sort((a, b) => a.name.localeCompare(b.name))
    }
    pluginCatalogUnavailable.value = plugins.status === 'rejected'
    if (plugins.status === 'fulfilled') installedPlugins.value = plugins.value
    if (policy.status === 'rejected') throw policy.reason
    permissionProfiles.value =
      profiles.status === 'fulfilled' && profiles.value ? permissionProfilesForLayer(profiles.value) : null
    configPath.value = String(policy.value.configPath || '')
    workspaceId.value = policy.value.workspaceId ?? null
    workspaceConfigIsActive.value = policy.value.activeWorkspace
    inheritedPermission.value = null
    resolvedPermission.value = null
    inheritedNetworkError.value = ''
    if (policy.value.resolved !== undefined) {
      try {
        resolvedPermission.value = parsePermissionConfig(policy.value.resolved)
        resolvedRawFingerprint.value = JSON.stringify(policy.value.policy)
      } catch {
        // The raw editor remains available when resolved metadata is invalid.
      }
    }
    if (profileEditor.value) {
      inheritedPermission.value = props.inheritedDraftPermission ?? null
    } else if (source !== 'effective') {
      try {
        const defaults =
          source === 'global' ? permissionProfiles.value?.[DEFAULT_PERMISSION_PROFILE_ID]?.permission : undefined
        const parent = defaults ? completePermissionConfig(defaults) : parsePermissionConfig(policy.value.inherited)
        inheritedPermission.value = parent
        if (parent.network === undefined) throw new Error(networkParentError())
      } catch {
        inheritedNetworkError.value = networkParentError()
      }
    }
    // A refresh updates server data only when the editor has no pending work.
    if (!dirty.value || !loaded.value) {
      appliedProfile.value = policy.value.profile ?? ''
      chosenProfile.value = appliedProfile.value
      hasCustomPermission.value = Boolean(Object.keys(policy.value.policy).length)
      const next = linkedPermission.value
        ? (resolvedPermission.value ??
          completePermissionConfig(permissionProfiles.value?.[appliedProfile.value]?.permission ?? {}))
        : policy.value.policy
      config.value = clone(next)
      savedConfig.value = clone(next)
      syncRaw()
      configRevision.value = policy.value.revision
    }
    if (linkedPermission.value) {
      inheritedPermission.value = null
      inheritedNetworkError.value = ''
    }
    loaded.value = true
    try {
      parsePermissionConfig(config.value)
    } catch (reason) {
      rawJsonError.value = permissionConfigErrorMessage(reason, st)
      rawEditorOpen.value = true
      if (profileEditor.value) error.value = rawJsonError.value
    }
  } catch (reason) {
    if (generation === loadGeneration && key === contextKey.value && !controller.signal.aborted)
      error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    if (generation === loadGeneration) loading.value = false
  }
}

function discard() {
  if (saving.value) return
  localSourceIntents.value = new Set()
  cancelNetworkRule()
  config.value = clone(savedConfig.value)
  syncRaw()
  error.value = ''
  saveError.value = ''
  drafts.delete(loadedContext)
  clearAllRuleSelections()
}

async function save() {
  if (profileEditor.value || editingDisabled.value || !dirty.value) return
  if (networkRuleDraft.value && !applyNetworkRule()) return
  if (networkPending.value && !applyNetworkJson()) return
  if (rawPending.value && !applyRawJson()) return
  const key = contextKey.value
  const source = settingsScope.value
  const sessionId = activeSessionId.value
  const workspaceTarget = activeWorkspaceTarget.value
  let snapshot: PermissionConfig
  try {
    snapshot = parsePermissionConfig(config.value)
  } catch (reason) {
    saveError.value = permissionConfigErrorMessage(reason, st)
    rawJsonError.value = saveError.value
    rawEditorOpen.value = true
    return
  }
  const scopeLabel =
    findSettingsScope('permissions', PERMISSION_ACCESS_VIEW, selectedSource.value)?.label || st('Permission')
  saving.value = true
  error.value = ''
  saveError.value = ''
  try {
    let savedPermission = snapshot
    let revision = configRevision.value
    if (source === 'session') {
      const response = await chat.setSessionPermission(String(sessionId), snapshot as JsonValue)
      const execution = response.execution as SessionStateResponse['execution']
      savedPermission = asConfig(
        execution?.execution?.selected_permission ||
          execution?.selected_permission ||
          execution?.context?.selected_permission,
      )
    } else {
      const options = {
        reload: true,
        workspace_id: source === 'workspace' ? workspaceTarget || undefined : undefined,
        expected_revision: revision,
      }
      const response = !Object.keys(snapshot).length
        ? await deleteRuntimeSetting('permission', options, source as 'global' | 'workspace')
        : await setRuntimeSetting('permission', snapshot as JsonValue, options, source as 'global' | 'workspace')
      // `current` is the persisted layer value even when runtime reload is
      // deferred. Do not replace it with a potentially older effective read.
      // Older response shapes must be confirmed by reading the same raw layer.
      if (response.current !== undefined) {
        savedPermission = parsePermissionConfig(response.current ?? {})
        revision = response.after_revision
      } else {
        const read = await readPolicy(source, sessionId, workspaceTarget, new AbortController().signal)
        savedPermission = parsePermissionConfig(read.policy)
        revision = read.revision
      }
    }
    drafts.delete(key)
    if (key === contextKey.value) {
      resolvedPermission.value = null
      localSourceIntents.value = new Set()
      configRevision.value = revision
      hasCustomPermission.value = Boolean(Object.keys(savedPermission).length)
      savedConfig.value = clone(savedPermission)
      config.value = clone(savedPermission)
      syncRaw()
    }
    toasts.push('success', st('{scope} updated', { scope: scopeLabel }))
  } catch (reason) {
    const message = reason instanceof Error ? reason.message : String(reason)
    if (key === contextKey.value) saveError.value = message
    else toasts.push('error', message)
  } finally {
    saving.value = false
  }
}

async function applyPermissionProfile() {
  if (profileEditor.value || !canEdit.value || !loaded.value || loading.value || saving.value || sandboxSaving.value)
    return
  const id = chosenProfile.value
  if (id === appliedProfile.value || (id && !Object.hasOwn(permissionProfiles.value ?? {}, id))) return
  const key = contextKey.value
  if (dirty.value || pendingRule.value) {
    const confirmed = await confirmAction({
      title: st('Discard unsaved permission changes?'),
      confirmText: st('Discard changes'),
      cancelText: st('Keep editing'),
    })
    if (!confirmed || key !== contextKey.value || id !== chosenProfile.value || saving.value) return
  }
  const source = settingsScope.value
  saving.value = true
  saveError.value = ''
  try {
    if (source === 'session') {
      await chat.setSessionPermissionProfile(String(activeSessionId.value), id || null)
    } else if (source === 'global' || source === 'workspace') {
      const options = {
        workspace_id: source === 'workspace' ? activeWorkspaceTarget.value || undefined : undefined,
        expected_revision: configRevision.value,
      }
      if (id || source === 'global') await setRuntimeSetting('permission_profile', id, options, source)
      else await deleteRuntimeSetting('permission_profile', options, source)
    }
    drafts.delete(key)
    if (key === contextKey.value) {
      loaded.value = false
      cancelNetworkRule()
      localSourceIntents.value = new Set()
      clearAllRuleSelections()
      await load()
    }
  } catch (reason) {
    if (key === contextKey.value) saveError.value = permissionConfigErrorMessage(reason, st)
  } finally {
    saving.value = false
  }
}

async function clearSource() {
  if (profileEditor.value || editingDisabled.value) return
  const key = contextKey.value
  const confirmed = await confirmAction({
    title:
      selectedSource.value === 'global'
        ? st('Restore default global permissions?')
        : st('Reset permissions in this scope?'),
    description:
      selectedSource.value === 'global'
        ? st('This restores all global permission settings on this page, including advanced fields. Save to apply.')
        : st('This resets all permission overrides in this scope, including advanced fields. Save to apply the reset.'),
    confirmText: selectedSource.value === 'global' ? st('Reset to defaults') : st('Reset'),
    cancelText: st('Cancel'),
  })
  if (!confirmed || editingDisabled.value || key !== contextKey.value) return
  cancelNetworkRule()
  localSourceIntents.value = new Set()
  config.value = {}
  syncRaw()
  error.value = ''
  clearAllRuleSelections()
}

async function canLeave() {
  if (saving.value || sandboxSaving.value || rulesPanel.value?.busy) return false
  if (!dirty.value && !drafts.size) return true
  return confirmAction({
    title: st('Discard unsaved permission changes?'),
    description: st('Your permission changes have not been saved.'),
    confirmText: st('Discard changes'),
    cancelText: st('Keep editing'),
  })
}
function routeWindowId(location: Pick<RouteLocationNormalized, 'query'>): string {
  const value = location.query.windowId
  return String(Array.isArray(value) ? value[0] || '' : value || '').trim()
}
function ownsNavigation(to: Pick<RouteLocationNormalized, 'query'>): boolean {
  if (!workspacePane) return true
  const destination = routeWindowId(to)
  return destination ? destination === workspacePane.windowId.value : workspacePane.isFocused.value
}
onBeforeRouteUpdate(async (to, from) => {
  // All mounted settings panes share route records. A navigation in another
  // pane must neither ask about nor discard this pane's retained draft.
  if (profileEditor.value || !ownsNavigation(to)) return true
  const sourceRoute =
    workspacePane && routeWindowId(from) && routeWindowId(from) !== workspacePane.windowId.value ? route : from
  const source = normalizePermissionSource(sourceRoute.query.scope || selectedSource.value)
  const recordedSessionId = lastSessionId.value || chat.selectedSessionId
  if (
    to.params.section === 'permissions' &&
    isPermissionAccessView(to.query.view || PERMISSION_ACCESS_VIEW) &&
    normalizePermissionSource(
      legacyPermissionAccessScope(to.query.view, to.query.scope) || to.query.scope || source,
    ) === source &&
    (source !== 'workspace' ||
      permissionWorkspaceTarget(to.query.targetWorkspaceId) ===
        permissionWorkspaceTarget(sourceRoute.query.targetWorkspaceId)) &&
    ((source !== 'session' && source !== 'effective') ||
      resolveSettingsSessionId(to.query.targetSessionId, recordedSessionId) ===
        resolveSettingsSessionId(sourceRoute.query.targetSessionId, recordedSessionId))
  )
    return true
  if (!(await canLeave())) return false
  discard()
  drafts.clear()
  return true
})
onBeforeRouteLeave((to) => profileEditor.value || !ownsNavigation(to) || canLeave())
function beforeUnload(event: BeforeUnloadEvent) {
  if (!dirty.value && !drafts.size) return
  event.preventDefault()
  event.returnValue = ''
}
async function refresh() {
  if (saving.value) return
  await Promise.allSettled([load(), rulesPanel.value?.refresh(), sandboxSettings.value?.refresh()])
}
defineExpose({ refresh, pendingRule, applyPendingRule })
function sectionHash(section: PermissionAccessSection) {
  return profileEditor.value ? '#' + rawJsonErrorId + '-' + section : permissionAccessHash(section)
}
function ruleGroupHash(kind: PermissionRuleKind) {
  return profileEditor.value ? '#' + rawJsonErrorId + '-rules-' + kind : '#permission-rules-' + kind
}
async function scrollToPermissionElement(selector: string) {
  await nextTick()
  const element = editorElement.value?.querySelector<HTMLElement>(selector)
  if (!element) return
  const toolbar = editorElement.value?.querySelector<HTMLElement>('[data-permission-toolbar]')
  element.style.scrollMarginTop = `${(toolbar?.getBoundingClientRect().height || 72) + 8}px`
  element.scrollIntoView({ block: 'start' })
  element.focus({ preventScroll: true })
}
function scrollToSection(section: PermissionAccessSection) {
  return scrollToPermissionElement(sectionHash(section))
}
function selectSection(section: PermissionAccessSection) {
  const hash = sectionHash(section)
  if (profileEditor.value || route.hash === hash) void scrollToSection(section)
  else void router.replace({ path: route.path, query: route.query, hash })
}
function selectRuleGroup(kind: PermissionRuleKind) {
  const hash = ruleGroupHash(kind)
  if (profileEditor.value || route.hash === hash)
    void scrollToPermissionElement(`[data-permission-rule-table="${kind}"]`)
  else void router.replace({ path: route.path, query: route.query, hash })
}
watch(
  [loaded, () => route.hash],
  ([ready, hash]) => {
    if (profileEditor.value || !ready) return
    const section = permissionSectionFromHash(hash)
    if (section) void scrollToSection(section)
    else {
      const group = ruleGroups.value.find((item) => hash === '#permission-rules-' + item.kind)
      if (group) void scrollToPermissionElement(`[data-permission-rule-table="${group.kind}"]`)
    }
  },
  { flush: 'post' },
)
watch(contextKey, () => void load())
watch(
  () => props.inheritedDraftPermission,
  (permission) => {
    if (profileEditor.value) inheritedPermission.value = permission ?? null
  },
)
watch(
  () => props.draftPermission,
  (permission) => {
    if (!permission || !loaded.value || JSON.stringify(permission) === JSON.stringify(config.value)) return
    config.value = asConfig(permission)
    syncRaw()
  },
)
onMounted(() => {
  void load()
  if (!profileEditor.value) window.addEventListener('beforeunload', beforeUnload)
})
onBeforeUnmount(() => {
  loadGeneration += 1
  loadController?.abort()
  window.removeEventListener('beforeunload', beforeUnload)
})
</script>

<template>
  <section
    ref="editorElement"
    class="permission-studio min-w-0"
    :class="{ 'is-profile': profileEditor }"
    :aria-busy="loading || saving"
  >
    <PermissionAccessToolbar
      v-if="!profileEditor"
      :scope="settingsScope"
      :session-id="sessionId"
      :workspace-target="activeWorkspaceTarget"
      :config-path="configPath"
      :disabled="saving || sandboxSaving || Boolean(rulesPanel?.busy)"
    >
      <template #status>
        <span class="permission-status" role="status" aria-live="polite">
          <RiLockLine v-if="selectedSource === 'effective'" class="h-3.5 w-3.5" />
          <RiRefreshLine v-else-if="loading || saving" class="h-3.5 w-3.5 animate-spin" />
          <span v-else-if="dirty" class="h-1.5 w-1.5 rounded-full bg-primary" />
          <RiCheckLine v-else-if="loaded" class="h-3.5 w-3.5" />
          {{
            selectedSource === 'effective'
              ? $st('Read-only')
              : saving
                ? $st('Saving…')
                : loading
                  ? $st('Loading…')
                  : dirty
                    ? $st('Unsaved changes')
                    : $st('All changes saved.')
          }}
        </span>
      </template>
      <template #actions>
        <div
          v-if="canEdit && loaded && (dirty || saving)"
          class="flex items-center gap-1"
          data-permission-savebar
          :title="$st('Save applies file, network, and tool changes together in this scope.')"
        >
          <Button variant="ghost" size="sm" class="h-8 px-2 text-[13px]" :disabled="saving" @click="discard">{{
            $st('Discard changes')
          }}</Button>
          <Button size="sm" class="h-8 px-2 text-[13px]" :disabled="editingDisabled" @click="save"
            ><RiSave3Line class="mr-1 h-4 w-4" />{{ saving ? $st('Saving…') : $st('Save permission') }}</Button
          >
        </div>
        <IconButton
          v-if="canEdit && !linkedPermission"
          variant="ghost"
          size="sm"
          class="h-8 w-8"
          :disabled="editingDisabled || !Object.keys(config).length"
          :tooltip="selectedSource === 'global' ? $st('Reset to defaults') : $st('Reset scope')"
          :aria-label="selectedSource === 'global' ? $st('Reset to defaults') : $st('Reset scope')"
          data-permission-reset-all
          @click="clearSource"
          ><RiResetLeftLine class="h-4 w-4"
        /></IconButton>
        <IconButton
          variant="ghost"
          size="sm"
          class="h-8 w-8"
          :tooltip="$st('Refresh permissions')"
          :aria-label="$st('Refresh permissions')"
          :disabled="loading || saving"
          @click="refresh"
          ><RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''"
        /></IconButton>
      </template>
    </PermissionAccessToolbar>
    <p
      v-if="error || saveError"
      role="alert"
      class="my-2 rounded border border-destructive/30 bg-destructive/5 px-2 py-1.5 text-[13px] text-destructive"
    >
      {{ saveError || error }}
    </p>
    <p v-if="selectedSource === 'session' && !activeSessionId" class="py-3 text-[13px] text-muted-foreground">
      {{ $st('Open a session to edit current-session permission.') }}
    </p>
    <div v-else-if="!loaded && loading" class="permission-domain-grid" aria-hidden="true">
      <div v-for="n in 2" :key="n" class="h-36 animate-pulse rounded-xl border border-border/75 bg-card/75" />
    </div>
    <template v-else-if="loaded">
      <SandboxSettingsPanel
        v-if="!profileEditor"
        :id="sectionHash('sandbox').slice(1)"
        ref="sandboxSettings"
        class="permission-sandbox"
        data-permission-section="sandbox"
        :aria-label="$st('Sandbox & resource preflight')"
        tabindex="-1"
        :scope="selectedSource"
        :workspace-id="activeWorkspaceTarget"
        :active-workspace="workspaceConfigIsActive"
        :disabled="saving"
        @saved="load"
      />
      <div
        v-if="!profileEditor && canEdit"
        class="settings-toolbar mt-2 rounded-md border border-border/70 px-2.5 py-1.5"
        data-permission-profile-picker
      >
        <label class="flex min-w-0 flex-[1_1_14rem] flex-wrap items-center gap-x-2 gap-y-1 text-xs font-medium">
          {{ $st('Permission profile') }}
          <select
            v-model="chosenProfile"
            :disabled="loading || saving || sandboxSaving"
            class="h-8 min-w-0 flex-1 rounded-md border border-input bg-background px-2 text-sm"
          >
            <option value="">
              {{ settingsScope === 'global' || hasCustomPermission ? $st('Custom permissions') : $st('Inherit') }}
            </option>
            <option v-if="appliedProfile && !permissionProfiles?.[appliedProfile]" :value="appliedProfile">
              {{ appliedProfile }}
            </option>
            <option v-for="[id, profile] in profileOptions" :key="id" :value="id">
              {{ permissionProfileDisplayName(id, profile) }}
            </option>
          </select>
        </label>
        <Button
          v-if="chosenProfile !== appliedProfile"
          size="sm"
          :disabled="loading || saving || sandboxSaving"
          @click="applyPermissionProfile"
          >{{ $st('Apply') }}</Button
        >
        <RouterLink
          :to="{ path: '/settings/permissions', query: { ...route.query, view: 'profiles', scope: 'global' } }"
          class="rounded-md px-2 py-1.5 text-xs font-medium text-muted-foreground hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring"
          >{{ $st('Manage permission profiles') }}</RouterLink
        >
        <p
          v-if="linkedPermission && permissionProfiles && !permissionProfiles[appliedProfile]"
          role="alert"
          class="basis-full break-words text-xs text-destructive"
        >
          {{ $st('The selected permission configuration is missing.') }}
        </p>
      </div>
      <p
        v-if="selectedSource === 'workspace' && workspaceConfigIsActive === false"
        class="mt-3 text-xs text-muted-foreground"
      >
        {{ $st('This configuration file is loaded when the server starts in this workspace.') }}
      </p>
      <div class="permission-domain-grid" data-permission-rule-layout>
        <div v-for="column in domainColumns" :key="column.id" class="permission-domain-column">
          <section
            v-for="domain in column.domains"
            :id="sectionHash(domain.id).slice(1)"
            :key="domain.id"
            class="permission-domain"
            :data-permission-section="domain.id"
            :aria-label="domain.label"
            tabindex="-1"
          >
            <header class="permission-domain-heading">
              <h2>
                <a :href="sectionHash(domain.id)" @click.prevent="selectSection(domain.id)">
                  <component :is="domain.icon" class="h-4 w-4 shrink-0" aria-hidden="true" />
                  {{ domain.label }}
                </a>
              </h2>
              <nav class="permission-rule-navigation" :aria-label="$st('Access rules')">
                <a
                  v-for="group in domain.groups"
                  :key="group.kind"
                  :href="ruleGroupHash(group.kind)"
                  :data-permission-rule-link="group.kind"
                  @click.prevent="selectRuleGroup(group.kind)"
                >
                  {{ group.label }}<span class="tabular-nums">{{ group.rows.length }}</span>
                </a>
              </nav>
              <PermissionHelp class="permission-domain-help" :text="domain.help" />
            </header>
            <div class="permission-domain-defaults">
              <template v-if="domain.id === 'filesystem'">
                <PermissionPolicyRow
                  :scope="selectedSource"
                  v-for="scope in pathScopes"
                  :key="scope.id"
                  :label="scope.label"
                  :hint="scope.description"
                  :value="pathModeLabel(displayedPathMode(scope.id))"
                  :editable="canEdit"
                  :source="pathSource(scope.id)"
                  :inherited-value="pathModeLabel(inheritedPermission?.path?.[scope.id])"
                  :inherited-source="inheritedSourceLabel"
                  :disabled="formDisabled"
                  :data-permission-default="'path.' + scope.id"
                  @source="setPathSource(scope.id, $event)"
                >
                  <OptionPicker
                    :model-value="String(displayedPathMode(scope.id) || '__invalid__')"
                    :options="pathModeOptions"
                    :include-empty="false"
                    :display-label="
                      !isPathPermissionMode(displayedPathMode(scope.id))
                        ? pathModeLabel(displayedPathMode(scope.id))
                        : undefined
                    "
                    :title="$st('Path access mode')"
                    :disabled="sourceValueDisabled(pathSource(scope.id))"
                    size="sm"
                    @update:model-value="setPathDefault(scope.id, $event)"
                  />
                </PermissionPolicyRow>
              </template>
              <template v-else-if="domain.id === 'network'">
                <PermissionPolicyRow
                  :scope="selectedSource"
                  :label="$st('Outbound default')"
                  :hint="$st('Action when no outbound rule matches.')"
                  :value="modeLabel(networkMode('default'))"
                  :editable="canEdit"
                  :source="networkSource('default')"
                  :inherited-value="networkBaseValueLabel('default')"
                  :inherited-source="inheritedSourceLabel"
                  :disabled="formDisabled || Boolean(networkRuleDraft)"
                  data-permission-default="network.default"
                  @source="setNetworkSource('default', $event)"
                  ><OptionPicker
                    :model-value="networkMode('default') || ''"
                    :options="networkActionOptions"
                    :include-empty="false"
                    :display-label="networkMode('default') ? undefined : $st('Unavailable')"
                    :title="$st('Outbound default')"
                    :disabled="sourceValueDisabled(networkSource('default')) || Boolean(networkRuleDraft)"
                    size="sm"
                    @update:model-value="setNetworkMode('default', $event)"
                /></PermissionPolicyRow>
                <PermissionPolicyRow
                  :scope="selectedSource"
                  v-for="field in networkAdvancedDefaults"
                  :key="field.id"
                  :label="field.label"
                  :hint="field.description"
                  :value="modeLabel(networkMode(field.id))"
                  :editable="canEdit"
                  :source="networkSource(field.id)"
                  :inherited-value="networkBaseValueLabel(field.id)"
                  :inherited-source="inheritedSourceLabel"
                  :disabled="formDisabled || Boolean(networkRuleDraft)"
                  :data-permission-default="'network.' + field.id"
                  @source="setNetworkSource(field.id, $event)"
                  ><OptionPicker
                    :model-value="networkMode(field.id) || ''"
                    :options="networkActionOptions"
                    :include-empty="false"
                    :display-label="networkMode(field.id) ? undefined : $st('Unavailable')"
                    :title="field.label"
                    :disabled="sourceValueDisabled(networkSource(field.id)) || Boolean(networkRuleDraft)"
                    size="sm"
                    @update:model-value="setNetworkMode(field.id, $event)"
                /></PermissionPolicyRow>
                <p v-if="inheritedNetworkError" role="alert" class="py-1 text-[13px] text-destructive">
                  {{ inheritedNetworkError }}
                </p>
              </template>
              <template v-else-if="domain.id === 'tools'">
                <PluginPermissionEditor
                  :plugins="installedPlugins"
                  :local="config.plugins"
                  :inherited="cleanResolvedPermission?.plugins ?? inheritedPermission?.plugins"
                  :editable="canEdit"
                  :disabled="formDisabled"
                  :load-failed="pluginCatalogUnavailable"
                  @change="setPluginPermission"
                  @retry="load"
                />
                <PermissionPolicyRow
                  :scope="selectedSource"
                  :label="$st('Default tool mode')"
                  :hint="$st('Used when no tool-specific rule matches.')"
                  :value="toolDefaultValueLabel"
                  :editable="canEdit"
                  :source="toolDefaultSource"
                  :inherited-value="policyModeLabel(inheritedPermission?.tools?.default)"
                  :inherited-source="inheritedSourceLabel"
                  :disabled="formDisabled"
                  data-permission-default="tools.default"
                  @source="setToolDefaultSource"
                  ><OptionPicker
                    :model-value="displayedToolDefault || '__invalid__'"
                    :options="modeOptions"
                    :include-empty="false"
                    :display-label="!displayedToolDefault ? toolDefaultValueLabel : undefined"
                    :title="$st('Default tool mode')"
                    :disabled="sourceValueDisabled(toolDefaultSource)"
                    size="sm"
                    @update:model-value="setToolDefault"
                /></PermissionPolicyRow>
                <PermissionPolicyRow
                  :scope="selectedSource"
                  :label="$st('Read-only tools')"
                  :hint="$st('Read-only Auto follows the default tool policy; named tool rules take precedence.')"
                  :value="readOnlyValueLabel"
                  :editable="canEdit"
                  :source="readOnlySource"
                  :inherited-value="readOnlyInheritedValueLabel()"
                  :inherited-source="inheritedSourceLabel"
                  :disabled="formDisabled"
                  :inherit-disabled="readOnlyInheritanceBlocked"
                  data-permission-default="tools.read-only"
                  @source="setReadOnlySource"
                  ><OptionPicker
                    :model-value="readOnlyValueMode"
                    :options="readOnlyModeOptions"
                    :include-empty="false"
                    :display-label="
                      displayedReadOnlyInvalid || displayedReadOnlyUnknown ? readOnlyValueLabel : undefined
                    "
                    :title="$st('Read-only tools')"
                    :disabled="sourceValueDisabled(readOnlySource)"
                    size="sm"
                    @update:model-value="setReadOnlyMode($event)"
                /></PermissionPolicyRow>
              </template>
            </div>
            <div class="permission-domain-rules">
              <PermissionRuleTable
                v-for="group in domain.groups"
                :id="ruleGroupHash(group.kind).slice(1)"
                :key="group.kind"
                class="permission-rule-group"
                tabindex="-1"
                :kind="group.kind"
                :scope="selectedSource"
                :rows="group.rows"
                :selected-keys="group.selectedKeys"
                :disabled="formDisabled || Boolean(networkRuleDraft)"
                @select="selectRuleRow"
                @select-visible="(rows, selected) => selectVisibleRuleRows(group.kind, rows, selected)"
                @mode="setRuleRowMode"
                @rename-target="renameRuleTarget"
                @edit="editRuleRow"
                @remove="removeRuleRow"
                @source="changeRuleSource"
                @reset="resetRuleRow"
              >
                <template v-if="canEdit" #actions>
                  <SettingsSourceControl
                    v-if="group.kind === 'command'"
                    :scope="selectedSource"
                    :model-value="commandSource"
                    :label="group.label"
                    :inherited-source="inheritedSourceLabel"
                    :show-inherited-summary="false"
                    :disabled="formDisabled"
                    @update:model-value="$event === 'local' ? copyCommandRules() : restoreCommandRules()"
                  />
                  <PopoverRoot v-if="group.kind === 'network'">
                    <PopoverTrigger as-child>
                      <Button
                        variant="ghost"
                        size="sm"
                        class="permission-network-source-trigger gap-1 px-2 text-[13px] text-muted-foreground"
                        data-permission-network-sources
                        :aria-label="group.label + ' · ' + $st('Source')"
                        :title="$st('Source')"
                        :disabled="formDisabled || Boolean(networkRuleDraft)"
                      >
                        <RiListSettingsLine class="h-4 w-4" aria-hidden="true" />
                        <span>{{ $st('Source') }}</span>
                      </Button>
                    </PopoverTrigger>
                    <PopoverPortal>
                      <PopoverContent
                        class="permission-network-source-popover"
                        align="end"
                        :side-offset="6"
                        :collision-padding="12"
                        :aria-label="group.label + ' · ' + $st('Source')"
                        data-permission-network-source-menu
                      >
                        <div
                          v-for="action in networkRuleLists"
                          :key="action"
                          class="permission-network-list-source"
                          :data-permission-default="'network.' + action"
                        >
                          <span>{{ networkRuleListLabel(action) }}</span>
                          <SettingsSourceControl
                            :scope="selectedSource"
                            :model-value="networkSource(action)"
                            :label="networkRuleListLabel(action)"
                            :inherited-value="networkBaseValueLabel(action)"
                            :inherited-source="inheritedSourceLabel"
                            :show-inherited-summary="false"
                            :disabled="formDisabled || Boolean(networkRuleDraft)"
                            @update:model-value="setNetworkSource(action, $event)"
                          />
                        </div>
                        <p class="text-xs leading-snug text-muted-foreground">
                          {{
                            $st(
                              'Each list replaces only the same list from the base settings. An empty local list clears those rules.',
                            )
                          }}
                        </p>
                      </PopoverContent>
                    </PopoverPortal>
                  </PopoverRoot>
                  <Button
                    variant="outline"
                    size="sm"
                    class="h-8 px-2 text-[13px]"
                    :disabled="
                      formDisabled ||
                      Boolean(networkRuleDraft) ||
                      (group.kind === 'command' && sourceIsReadOnly(commandSource)) ||
                      (group.kind === 'network' &&
                        networkRuleLists.every((action) => sourceIsReadOnly(networkSource(action))))
                    "
                    :data-permission-create-toggle="group.kind"
                    :aria-expanded="newRuleKind === group.kind"
                    @click="beginRule(group.kind)"
                    ><RiAddLine class="mr-1 h-3.5 w-3.5" />{{ group.addLabel }}</Button
                  >
                </template>
                <template #bulk="{ count }">
                  <PermissionBulkActions
                    v-if="group.kind === 'filesystem'"
                    v-model="bulkPathMode"
                    :count="count"
                    :options="pathModeOptions"
                    :disabled="formDisabled"
                    @apply="applySelectedPathMode"
                    @clear="clearPathRuleSelection"
                    @remove="removeSelectedPathRules"
                  />
                  <PermissionBulkActions
                    v-else-if="group.kind === 'tool-name'"
                    v-model="bulkToolNameMode"
                    :count="count"
                    :options="modeOptions"
                    :disabled="formDisabled"
                    @apply="setSelectedToolNameMode(bulkToolNameMode)"
                    @clear="clearToolNameRuleSelection"
                    @remove="removeSelectedToolNameRules"
                  />
                  <PermissionBulkActions
                    v-else-if="group.kind === 'command'"
                    v-model="bulkCommandMode"
                    :count="count"
                    :options="modeOptions"
                    :disabled="formDisabled"
                    @apply="setSelectedCommandMode(bulkCommandMode)"
                    @clear="clearCommandRuleSelection"
                    @remove="removeSelectedCommandRules"
                  />
                </template>
                <template #create>
                  <fieldset
                    v-if="group.kind === newRuleKind && group.kind !== 'network'"
                    class="permission-rule-form"
                    :disabled="formDisabled"
                    data-permission-create-form
                  >
                    <div class="mb-1 flex items-center justify-between gap-2">
                      <strong class="text-[13px]">{{
                        newRuleKind === 'filesystem'
                          ? $st('Add path rule')
                          : newRuleKind === 'tool-name'
                            ? $st('Add tool name')
                            : $st('Add command rule')
                      }}</strong
                      ><IconButton
                        variant="ghost"
                        size="sm"
                        :aria-label="$st('Cancel')"
                        :tooltip="$st('Cancel')"
                        @click="newRuleKind = ''"
                        ><RiCloseLine class="h-4 w-4"
                      /></IconButton>
                    </div>
                    <template v-if="newRuleKind === 'filesystem'"
                      ><form class="permission-create-form" @submit.prevent="addPathRule">
                        <label class="permission-control"
                          ><span>{{ $st('Path') }}</span
                          ><Input
                            v-model="newPath"
                            :placeholder="$st('Exact file or directory')"
                            :aria-label="$st('Path')"
                            class="h-8 font-mono text-[13px]"
                        /></label>
                        <div class="permission-create-actions">
                          <OptionPicker
                            v-model="newPathMode"
                            :options="pathModeOptions"
                            :include-empty="false"
                            :title="$st('New path access mode')"
                            size="sm"
                            :disabled="formDisabled"
                          /><Button
                            type="submit"
                            variant="outline"
                            size="sm"
                            :disabled="formDisabled || !newPath.trim()"
                            >{{ $st('Add path rule') }}</Button
                          >
                        </div>
                      </form></template
                    ><template v-else-if="newRuleKind === 'tool-name'"
                      ><form class="permission-create-form" @submit.prevent="addToolName">
                        <label class="permission-control"
                          ><span>{{ $st('Tool name') }}</span
                          ><OptionPicker
                            v-model="newToolName"
                            :options="toolOptions"
                            :include-empty="false"
                            :allow-custom="true"
                            :title="$st('Tool name')"
                            :placeholder="$st('Choose or enter a tool name')"
                            :disabled="formDisabled"
                            size="sm"
                            monospace
                        /></label>
                        <div class="permission-create-actions">
                          <OptionPicker
                            v-model="newToolNameMode"
                            :options="modeOptions"
                            :include-empty="false"
                            :title="$st('Tool name mode')"
                            :disabled="formDisabled"
                            size="sm"
                          /><Button
                            type="submit"
                            variant="outline"
                            size="sm"
                            :disabled="formDisabled || !newToolName.trim()"
                            >{{ $st('Add tool name') }}</Button
                          >
                        </div>
                      </form></template
                    ><template v-else
                      ><form class="permission-create-form" @submit.prevent="addCommandRule">
                        <label class="permission-control"
                          ><span>{{ $st('Command pattern') }}</span
                          ><Input
                            v-model="newCommandPattern"
                            :disabled="sourceValueDisabled(commandSource)"
                            class="h-8 font-mono text-[13px]"
                            placeholder="git push *"
                            :aria-label="$st('Command pattern')"
                        /></label>
                        <div class="permission-create-actions">
                          <OptionPicker
                            v-model="newCommandMode"
                            :options="modeOptions"
                            :include-empty="false"
                            :title="$st('Command rule mode')"
                            :disabled="sourceValueDisabled(commandSource)"
                            size="sm"
                          /><Button
                            type="submit"
                            variant="outline"
                            size="sm"
                            :disabled="sourceValueDisabled(commandSource) || !newCommandPattern.trim()"
                            >{{ $st('Add command rule') }}</Button
                          >
                        </div>
                      </form></template
                    >
                  </fieldset>
                  <form
                    v-if="group.kind === 'network' && networkRuleDraft"
                    class="permission-create-form my-1.5 rounded border border-primary/20 bg-primary/5 p-2"
                    @submit.prevent="applyNetworkRule"
                    data-permission-create-form
                  >
                    <label v-if="networkRuleDraft.index === null" class="permission-control"
                      ><span>{{ $st('Mode') }}</span
                      ><OptionPicker
                        :model-value="networkRuleDraft.action"
                        :options="networkRuleActionOptions"
                        @update:model-value="setNetworkRuleAction"
                        :include-empty="false"
                        :title="$st('Mode')"
                        size="sm"
                        :disabled="editingDisabled"
                    /></label>
                    <label class="permission-control"
                      ><span>{{ $st('Destination') }}</span
                      ><Input
                        v-model="networkRuleDraft.to"
                        class="h-8 font-mono text-[13px]"
                        placeholder="192.0.2.10"
                        :disabled="editingDisabled"
                        :aria-label="$st('Destination')"
                        :title="$st('One IP address, CIDR, or * for all destinations.')"
                        @update:model-value="networkRuleError = ''"
                    /></label>
                    <div class="permission-create-actions">
                      <label class="permission-control"
                        ><span>{{ $st('Protocol') }}</span
                        ><OptionPicker
                          :model-value="networkRuleDraft.protocol"
                          :options="networkProtocolOptions"
                          :include-empty="false"
                          :title="$st('Protocol')"
                          :disabled="editingDisabled"
                          size="sm"
                          @update:model-value="setNetworkRuleProtocol" /></label
                      ><label class="permission-control"
                        ><span>{{ $st('Ports') }}</span
                        ><Input
                          v-model="networkRuleDraft.ports"
                          data-permission-network-ports
                          class="h-8 font-mono text-[13px]"
                          placeholder="443, 8000-8099"
                          :disabled="editingDisabled || networkRuleDraft.protocol === 'icmp'"
                          :aria-label="$st('Ports')"
                          :title="$st('Separate ports and ranges with commas. Leave blank for all ports.')"
                          @update:model-value="networkRuleError = ''"
                      /></label>
                    </div>
                    <details :open="Boolean(networkRuleDraft.except)">
                      <summary class="cursor-pointer text-[13px]">{{ $st('Excluded subnets') }}</summary>
                      <Input
                        v-model="networkRuleDraft.except"
                        class="mt-1 h-8 font-mono text-[13px]"
                        placeholder="10.20.10.0/24"
                        :disabled="editingDisabled"
                        :aria-label="$st('Excluded subnets')"
                        :title="$st('Exclude subnets from this rule only. Use a deny rule to block them everywhere.')"
                        @update:model-value="networkRuleError = ''"
                      />
                    </details>
                    <p v-if="networkRuleError" role="alert" class="text-[13px] text-destructive">
                      {{ networkRuleError }}
                    </p>
                    <div class="flex gap-1">
                      <Button
                        type="submit"
                        variant="outline"
                        size="sm"
                        :disabled="editingDisabled"
                        data-permission-network-apply
                        >{{ $st('Apply rule') }}</Button
                      ><Button
                        type="button"
                        variant="ghost"
                        size="sm"
                        :disabled="editingDisabled"
                        @click="cancelNetworkRule"
                        >{{ $st('Cancel') }}</Button
                      >
                    </div>
                  </form>
                </template>
              </PermissionRuleTable>
            </div>
            <div v-if="domain.id === 'network' && !profileEditor" class="permission-domain-extras">
              <details class="permission-details">
                <summary class="permission-summary">
                  <RiCodeSSlashLine class="h-4 w-4" />{{ $st('Network policy JSON')
                  }}<span v-if="networkPending" class="ml-auto h-1.5 w-1.5 rounded-full bg-primary" />
                </summary>
                <div class="permission-json-body">
                  <p class="text-[13px] text-muted-foreground">
                    {{
                      $st(
                        'Version 2 stores only local overrides. Omitted fields use base settings; empty lists clear the corresponding rules.',
                      )
                    }}
                  </p>
                  <textarea
                    :value="networkJsonDisplay"
                    class="permission-json"
                    :class="{ 'is-inherited': !networkHasLocal }"
                    rows="10"
                    spellcheck="false"
                    :readonly="networkJsonReadOnly"
                    :aria-label="$st('Network policy JSON')"
                    :aria-invalid="Boolean(networkJsonError)"
                    @input="editNetworkJson"
                  />
                  <p v-if="networkJsonError" class="text-[13px] text-destructive" role="alert">
                    {{ networkJsonError }}
                  </p>
                  <div v-if="canEdit" class="flex flex-wrap gap-1">
                    <Button
                      variant="outline"
                      size="sm"
                      :disabled="networkJsonReadOnly || !networkPending"
                      @click="applyNetworkJson"
                      >{{ $st('Apply JSON') }}</Button
                    ><Button
                      variant="ghost"
                      size="sm"
                      :disabled="networkJsonReadOnly || !networkFullyLocal"
                      @click="useNetworkExample"
                      >{{ $st('Load CIDR example') }}</Button
                    >
                  </div>
                </div>
              </details>
            </div>
          </section>
        </div>
      </div>
      <div v-if="!profileEditor" class="permission-secondary">
        <PermissionsPanel ref="rulesPanel" :scope="settingsScope" :session-id="sessionId" :workspace-id="workspaceId" />
        <details
          class="permission-details permission-advanced"
          data-permission-advanced
          :open="rawEditorOpen"
          @toggle="rawEditorOpen = ($event.target as HTMLDetailsElement).open"
        >
          <summary class="permission-summary permission-advanced-heading">
            <RiCodeSSlashLine class="h-4 w-4 shrink-0 text-primary" aria-hidden="true" />
            <span class="min-w-0 flex-1">{{ $st('Advanced permission configuration') }}</span>
            <span v-if="rawPending" class="text-xs font-medium text-primary">{{ $st('Unsaved') }}</span>
          </summary>
          <div class="permission-advanced-body">
            <p class="text-[13px] text-muted-foreground">
              {{
                rawJsonReadOnly
                  ? $st('Read-only')
                  : $st('Edit file, network, and tool permissions together as JSON for this scope.')
              }}
            </p>
            <textarea
              :value="rawJsonDisplay"
              class="permission-json"
              :class="{ 'is-inherited': rawJsonReadOnly }"
              rows="12"
              dir="ltr"
              spellcheck="false"
              autocapitalize="off"
              :readonly="rawJsonReadOnly"
              :aria-label="$st('Raw PermissionConfig')"
              :aria-invalid="Boolean(rawJsonError)"
              :aria-describedby="rawJsonError ? rawJsonErrorId : undefined"
              @input="editRawJson"
            />
            <p v-if="rawJsonError" :id="rawJsonErrorId" class="break-words text-[13px] text-destructive" role="alert">
              {{ rawJsonError }}
            </p>
            <div v-if="!rawJsonReadOnly" class="permission-advanced-footer">
              <p class="text-xs text-muted-foreground">
                {{ $st('Changes take effect after you select {action}.', { action: $st('Save permission') }) }}
              </p>
              <Button variant="outline" size="sm" :disabled="!rawPending" @click="applyRawJson">
                {{ $st('Apply JSON') }}
              </Button>
            </div>
          </div>
        </details>
      </div>
    </template>
  </section>
</template>

<style scoped>
@reference '../../style.css';

.permission-studio {
  container: permissions / inline-size;
  width: 100%;
  max-width: 92rem;
  margin-inline: auto;
  font-size: 0.8125rem;
}
.permission-status {
  @apply inline-flex items-center gap-1 rounded-full bg-muted/40 px-2 py-0.5 text-muted-foreground;
  font-size: 0.75rem;
  line-height: 1.35;
}
.permission-sandbox {
  margin-block-start: 0.875rem;
  scroll-margin-top: 8rem;
}
.permission-domain-grid,
.permission-domain-column {
  display: grid;
  align-content: start;
  align-items: start;
  gap: 0.875rem;
  min-width: 0;
}
.permission-domain-grid {
  margin-block-start: 0.875rem;
}
.is-profile .permission-domain-grid {
  margin-block-start: 0;
}
.permission-domain {
  @apply rounded-xl border border-border/75 bg-card/75;
  container: permission-domain / inline-size;
  min-width: 0;
  scroll-margin-top: 8rem;
}
.permission-domain:focus,
.permission-rule-group:focus {
  outline: none;
}
.permission-domain:focus-visible .permission-domain-heading h2,
.permission-rule-group:focus-visible :deep(.rule-table-title) {
  @apply text-primary decoration-primary underline underline-offset-4;
}
.permission-domain-heading {
  @apply rounded-t-xl border-b border-border/65 bg-muted/18;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  column-gap: 0.375rem;
  min-height: 2.75rem;
  padding: 0.25rem 0.5rem 0.25rem 0.75rem;
}
.permission-domain-heading h2 {
  flex: 1 1 auto;
  min-width: 0;
  font-size: 0.9375rem;
  font-weight: 650;
  line-height: 1.35;
}
.permission-domain-heading h2 a {
  @apply rounded-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring;
  display: inline-flex;
  align-items: center;
  gap: 0.4375rem;
}
.permission-domain-heading h2 svg {
  @apply text-primary;
}
.permission-rule-navigation {
  display: flex;
  flex-wrap: wrap;
  gap: 0.125rem;
}
.permission-rule-navigation a {
  @apply rounded-md px-1.5 text-muted-foreground hover:bg-muted/55 hover:text-foreground
    focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring;
  display: inline-flex;
  align-items: center;
  gap: 0.3125rem;
  min-height: 1.75rem;
  font-size: 0.75rem;
  line-height: 1.25;
}
.permission-rule-navigation span {
  @apply font-medium text-foreground;
}
.permission-domain-defaults {
  padding: 0.1875rem 0.75rem;
}
.permission-domain-defaults :deep(.permission-policy-row + .permission-policy-row) {
  @apply border-t border-border/50;
}
.permission-domain-rules {
  @apply border-t border-border/75;
  min-width: 0;
}
.permission-rule-group {
  min-width: 0;
  scroll-margin-top: 8rem;
}
.permission-domain-extras {
  @apply rounded-b-xl border-t border-border/75 bg-muted/12;
  min-width: 0;
  padding-inline: 0.75rem;
}
.permission-control {
  display: grid;
  flex: 1 1 12rem;
  min-width: 0;
  gap: 0.25rem;
}
.permission-control > span {
  @apply text-muted-foreground;
}
.permission-rule-form {
  @apply rounded-lg border border-primary/20 bg-primary/5;
  margin-block: 0.375rem;
  padding: 0.625rem;
  min-width: 0;
}
.permission-create-form {
  display: flex;
  flex-wrap: wrap;
  align-items: end;
  gap: 0.5rem;
  min-width: 0;
}
.permission-create-actions {
  display: grid;
  flex: 1 1 17rem;
  grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
  align-items: end;
  gap: 0.5rem;
}
.permission-create-actions :deep(button) {
  max-width: 100%;
  white-space: normal;
}
.permission-create-form > details,
.permission-create-form > p {
  flex: 1 1 100%;
}
.permission-network-source-popover {
  @apply z-[80] grid gap-1 rounded-lg border border-border bg-popover p-2.5 text-popover-foreground shadow-lg outline-none;
  width: min(22rem, calc(100vw - 1.5rem));
  max-width: var(--radix-popover-content-available-width);
}
.permission-network-source-trigger[data-state='open'] {
  @apply bg-muted text-foreground;
}
@container permission-rules (max-width: 26rem) {
  .permission-network-source-trigger > span {
    @apply sr-only;
  }
}
.permission-network-list-source {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 0.25rem 0.5rem;
  min-width: 0;
  min-height: 2rem;
  font-size: 0.8125rem;
  line-height: 1.25;
}
.permission-secondary {
  display: grid;
  gap: 0.875rem;
  margin-block-start: 0.875rem;
}
.permission-advanced {
  @apply rounded-xl border border-border/75 bg-card/75;
}
.permission-summary.permission-advanced-heading {
  @apply rounded-xl hover:bg-muted/25;
  min-height: 2.75rem;
  padding: 0.5rem 0.75rem;
  font-size: 0.9375rem;
  font-weight: 650;
  overflow-wrap: anywhere;
}
.permission-advanced[open] > .permission-advanced-heading {
  @apply rounded-b-none border-b border-border/65 bg-muted/18;
}
.permission-advanced-body {
  display: grid;
  gap: 0.5rem;
  min-width: 0;
  padding: 0.75rem;
}
.permission-advanced-body .permission-json[aria-invalid='true'] {
  @apply border-destructive/60;
}
.permission-advanced-footer {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 0.375rem 0.75rem;
}
.permission-advanced-footer > button {
  margin-inline-start: auto;
}
.permission-details {
  min-width: 0;
}
.permission-summary {
  @apply rounded-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring;
  display: flex;
  min-height: 2.375rem;
  cursor: pointer;
  list-style: none;
  align-items: center;
  gap: 0.4375rem;
  font-size: 0.8125rem;
  font-weight: 550;
}
.permission-summary::-webkit-details-marker {
  display: none;
}
.permission-summary::after {
  @apply ml-auto text-muted-foreground;
  content: '';
  width: 0.4375rem;
  height: 0.4375rem;
  border: solid currentColor;
  border-width: 0 1px 1px 0;
  transform: rotate(45deg) translateY(-2px);
  margin-inline-end: 0.25rem;
  flex-shrink: 0;
}
.permission-details[open] > .permission-summary::after {
  transform: rotate(225deg) translateY(-1px);
}
.permission-json-body {
  display: grid;
  gap: 0.5rem;
  min-width: 0;
  padding: 0.25rem 0 0.625rem;
}
.permission-json {
  @apply rounded-md border border-input bg-background p-2 outline-none focus-visible:ring-1 focus-visible:ring-ring;
  width: 100%;
  min-width: 0;
  resize: vertical;
  font-family: var(--font-mono);
  font-size: 0.8125rem;
  line-height: 1.5;
}
.permission-json.is-inherited {
  color: var(--color-muted-foreground);
}
.permission-studio :deep(button[aria-haspopup='dialog']) {
  min-height: 2rem;
  height: auto;
  font-size: 0.8125rem;
}
.permission-domain :deep(button[aria-haspopup='dialog'] .truncate) {
  overflow: visible;
  overflow-wrap: anywhere;
  text-overflow: clip;
  white-space: normal;
}
@container permissions (min-width: 58rem) {
  .permission-domain-grid {
    grid-template-columns: minmax(0, 0.94fr) minmax(0, 1.06fr);
  }
}
@container permission-domain (max-width: 24rem) {
  .permission-domain-heading {
    column-gap: 0.125rem;
    padding-inline-start: 0.625rem;
  }
  .permission-domain-heading h2 {
    flex: 1 1 9rem;
  }
  .permission-domain-defaults,
  .permission-domain-extras {
    padding-inline: 0.625rem;
  }
}
</style>
