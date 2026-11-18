<script setup lang="ts">
import { computed, defineAsyncComponent, nextTick, onBeforeUnmount, onMounted, ref, watch, type Ref } from 'vue'
import { onBeforeRouteLeave, onBeforeRouteUpdate, useRoute, useRouter, type RouteLocationNormalized } from 'vue-router'
import {
  RiAddLine,
  RiCheckLine,
  RiCloseLine,
  RiCodeSSlashLine,
  RiFolderLine,
  RiGlobalLine,
  RiLockLine,
  RiRefreshLine,
  RiResetLeftLine,
  RiSave3Line,
  RiTerminalBoxLine,
  RiToolsLine,
} from '@remixicon/vue'

import Button from '@/components/ui/Button.vue'
import { findSettingsScope } from '@/components/settings/settingsScopeCatalog'
import IconButton from '@/components/ui/IconButton.vue'
import Input from '@/components/ui/Input.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import PermissionRuleTable from './permissions/PermissionRuleTable.vue'
import type { PermissionRuleKind, PermissionRuleRow } from './permissions/permissionRuleTable'
import PermissionAccessToolbar from './permissions/PermissionAccessToolbar.vue'
import PermissionPolicyRow from './permissions/PermissionPolicyRow.vue'
import PermissionHelp from './permissions/PermissionHelp.vue'
import {
  isPermissionAccessView,
  PERMISSION_ACCESS_VIEW,
  permissionAccessHash,
  permissionSectionFromHash,
  type PermissionAccessSection,
} from './permissions/permissionNavigation'
import { apiJson } from '@/lib/api'
import { confirmAction } from '@/lib/appConfirm'
import { deleteRuntimeSetting, setRuntimeSetting } from '@/lib/runtimeSettings'
import { useChatStoreUnscoped } from '@/stores/chat'
import { lastSessionId } from '@/lib/lastSession'
import { resolveSettingsSessionId } from './settingsSessionScope'
import { useWorkspacePaneContext } from '@/app/workspace/workspacePaneContext'
import { useToastsStore } from '@/stores/toasts'
import type { JsonValue } from '@/types/json'
import { settingsText as st } from '@/i18n/settingsText'

import {
  canonicalPermissionPath,
  canonicalPermissionToolEntries,
  canonicalPermissionToolName,
  isPathPermissionMode,
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
type SettingsResponse = { value?: JsonValue; inherited_value?: JsonValue; config_found?: boolean; config_path?: string }
type SessionStateResponse = {
  execution?: {
    selected_permission?: PermissionConfig
    effective_permission?: PermissionConfig
    inherited_permission?: PermissionConfig
    context?: {
      selected_permission?: PermissionConfig
      effective_permission?: PermissionConfig
      inherited_permission?: PermissionConfig
    }
    execution?: {
      selected_permission?: PermissionConfig
      effective_permission?: PermissionConfig
      inherited_permission?: PermissionConfig
    }
  }
}
type ToolCatalogResponse = { permission_tools?: Array<{ name?: string; summary?: string }> }

const SHELL_LAUNCH_TOOLS = ['agena.shell.exec', 'agena.shell.spawn', 'agena.shell.watch', 'agena.shell.open'] as const
const SHELL_CAPABLE_TOOLS = [...SHELL_LAUNCH_TOOLS, 'agena.shell.write', 'agena.monitor.start'] as const

const props = defineProps<{ scope: string; sessionId?: string }>()
const route = useRoute()
const router = useRouter()
const workspacePane = useWorkspacePaneContext()
const editorElement = ref<HTMLElement | null>(null)
const sections = computed(() => [
  { id: 'filesystem' as const, label: st('File access'), icon: RiFolderLine },
  { id: 'network' as const, label: st('Network access'), icon: RiGlobalLine },
  { id: 'tools' as const, label: st('Tool execution & approvals'), icon: RiToolsLine },
])
const chat = useChatStoreUnscoped()
const toasts = useToastsStore()
const PermissionsPanel = defineAsyncComponent(() => import('@/components/settings/PermissionsPanel.vue'))

const loading = ref(false)
const saving = ref(false)
const error = ref('')
const saveError = ref('')
const loaded = ref(false)
const savedConfig = ref<PermissionConfig>({})
const rawEditorOpen = ref(false)
const rulesPanel = ref<{ refresh: () => Promise<void>; busy?: boolean } | null>(null)
const rawPending = computed(() => rawJson.value !== JSON.stringify(config.value, null, 2))
const networkPending = computed(() => networkJson.value !== JSON.stringify(config.value.network || {}, null, 2))
const dirty = computed(
  () =>
    JSON.stringify(config.value) !== JSON.stringify(savedConfig.value) ||
    rawPending.value ||
    networkPending.value ||
    Boolean(networkRuleDraft.value),
)
const editingDisabled = computed(() => !canEdit.value || !loaded.value || loading.value || saving.value)
const formDisabled = computed(() => editingDisabled.value || rawPending.value || networkPending.value)
// The workbench owns source selection; this editor only normalizes the layer.
const selectedSource = computed<PermissionSource>(() => normalizePermissionSource(props.scope))

function normalizePermissionSource(value: unknown): PermissionSource {
  return value === 'global' || value === 'workspace' || value === 'session' ? value : 'effective'
}
const config = ref<PermissionConfig>({})
const rawJson = ref('{}')
const rawJsonError = ref('')
const networkJson = ref('{}')
const networkJsonError = ref('')
const inheritedPermission = ref<PermissionConfig | null>(null)
const configPath = ref('')
const inheritedNetwork = computed(() => inheritedPermission.value?.network ?? null)
const inheritedNetworkError = ref('')
type NetworkRuleList = 'allow' | 'deny'
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
const displayedNetwork = computed(() => config.value.network ?? inheritedNetwork.value)
const networkSourceOptions = computed(() => [
  { value: 'inherit', label: st('Inherit parent policy') },
  { value: 'custom', label: st('Custom network policy') },
])
const networkProtocolOptions = computed(() => [
  ...(['tcp', 'udp', 'icmp'] as const).map((value) => ({ value, label: value.toUpperCase() })),
  { value: 'any', label: st('Any protocol') },
])
const toolCatalog = ref<Array<{ name: string; summary: string }>>([])
const newPath = ref('')
const newPathMode = ref<PathPermissionMode>('readonly')
const newToolName = ref('')
const newToolNameMode = ref<PermissionMode>('ask')
const newCommandTool = ref<string>(SHELL_LAUNCH_TOOLS[0])
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
const commandQuery = ref('')
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
const shellToolOptions = SHELL_CAPABLE_TOOLS.map((value) => ({ value, label: value }))
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
const commandClasses = computed(() => [
  { id: 'no-op', label: st('No-op commands') },
  { id: 'routine', label: st('Routine commands') },
  { id: 'dangerous', label: st('Dangerous commands') },
])

const pathRules = computed(() => Object.entries(config.value.path?.rules || {}))
const toolNameRules = computed(() => canonicalPermissionToolEntries(config.value.tools?.names))
const commandRules = computed(() => {
  return commandRows(config.value.tools?.rules)
})
function commandRows(groups: NonNullable<PermissionConfig['tools']>['rules']) {
  const rows: Array<{ tool: string; command: string; mode: PermissionMode }> = []
  for (const [tool, rawRules] of canonicalPermissionToolEntries(groups)) {
    // Shell launch/input tools and command monitors carry command rules; `*`
    // carries the read-only class, which applies to every tool whose
    // permission contract is read-only. Anything else would be inert.
    if (!isShellCapableTool(tool) && tool !== '*') continue
    if (isMode(rawRules)) {
      if (!isShellCapableTool(tool)) continue
      rows.push({ tool, command: '*', mode: rawRules })
      continue
    }
    if (!rawRules || typeof rawRules !== 'object' || Array.isArray(rawRules)) continue
    for (const [command, mode] of Object.entries(rawRules)) {
      if (tool === '*' && !isReadOnlyClass(command)) continue
      if (isMode(mode)) rows.push({ tool, command, mode })
    }
  }
  return rows
}
// Configured classes are real command rules and remain visible alongside patterns.
const customCommandRules = computed(() => commandRules.value.filter((row) => row.tool !== '*'))
const visiblePathRules = computed(() => pathRules.value.filter(([path]) => matchesQuery(path, pathQuery.value)))
const visibleToolNameRules = computed(() => toolNameRules.value.filter(([name]) => matchesQuery(name, toolQuery.value)))
const visibleCommandRules = computed(() =>
  customCommandRules.value.filter((row) => matchesQuery(`${row.tool} ${row.command}`, commandQuery.value)),
)
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
  commandRows(inheritedPermission.value?.tools?.rules).filter(
    (row) => row.tool !== '*' && !hasToolEntry(config.value.tools?.rules, row.tool),
  ),
)
const visibleInheritedPathRules = computed(() =>
  inheritedPathRules.value.filter(([path]) => matchesQuery(path, pathQuery.value)),
)
const visibleInheritedToolNameRules = computed(() =>
  inheritedToolNameRules.value.filter(([name]) => matchesQuery(name, toolQuery.value)),
)
const visibleInheritedCommandRules = computed(() =>
  inheritedCommandRules.value.filter((row) => matchesQuery(`${row.tool} ${row.command}`, commandQuery.value)),
)
const displayedToolDefault = computed<PermissionMode>(
  () => config.value.tools?.default || inheritedPermission.value?.tools?.default || 'allow',
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
  Object.hasOwn(config.value.tools?.rules || {}, '*')
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
    Object.entries(group).some(([name, mode]) => isReadOnlyClass(name) && !isMode(mode))
  )
}
const displayedReadOnlyInvalid = computed(() =>
  invalidReadOnlyPolicy(Object.hasOwn(config.value.tools?.rules || {}, '*') ? config.value : inheritedPermission.value),
)
const displayedReadOnlyMode = computed<PermissionMode | undefined>(() =>
  displayedReadOnlyInvalid.value
    ? undefined
    : displayedReadOnlyPolicy.value === 'auto'
      ? displayedToolDefault.value
      : displayedReadOnlyPolicy.value || 'allow',
)
const readOnlyEditorMode = computed(
  () =>
    readOnlyPolicy(config.value) ||
    (Object.hasOwn(config.value.tools?.rules || {}, '*') && !displayedReadOnlyInvalid.value ? 'allow' : ''),
)
const readOnlyModeOptions = computed(() =>
  modeOptions.value.map((option) =>
    option.value === 'auto'
      ? {
          ...option,
          label: `${modeLabel(displayedToolDefault.value)} · ${st('Follow tool default')}`,
          description: st('Read-only Auto follows the default tool policy; named tool rules take precedence.'),
        }
      : option,
  ),
)
function modeLabel(mode: unknown): string {
  return modeOptions.value.find((option) => option.value === mode)?.label || '—'
}
function pathModeLabel(mode: unknown): string {
  return pathModeOptions.value.find((option) => option.value === mode)?.label || '—'
}
function displayedPathMode(scope: 'workspace' | 'external') {
  return config.value.path?.[scope] || inheritedPermission.value?.path?.[scope]
}
function toolInheritanceLabel() {
  return `${st('Inherit')} · ${modeLabel(inheritedPermission.value?.tools?.default || 'allow')}`
}
function readOnlyInheritanceLabel() {
  if (displayedReadOnlyInvalid.value) return '—'
  const parent = readOnlyPolicy(inheritedPermission.value)
  return `${st('Inherit')} · ${modeLabel(parent === 'auto' ? displayedToolDefault.value : parent || 'allow')}`
}
function sectionRuleCount(section: PermissionAccessSection) {
  if (section === 'filesystem') return pathRules.value.length + inheritedPathRules.value.length
  if (section === 'network') return networkRules('allow').length + networkRules('deny').length
  return (
    toolNameRules.value.length +
    inheritedToolNameRules.value.length +
    customCommandRules.value.length +
    inheritedCommandRules.value.length
  )
}
function sectionSummary(section: PermissionAccessSection) {
  if (section === 'filesystem')
    return st('Workspace: {workspace} · External: {external}', {
      workspace: pathModeLabel(displayedPathMode('workspace')),
      external: pathModeLabel(displayedPathMode('external')),
    })
  if (section === 'network') return st('Outbound: {mode}', { mode: modeLabel(networkMode('default')) })
  return st('Default: {mode} · Read only: {readonly}', {
    mode: modeLabel(displayedToolDefault.value),
    readonly: modeLabel(displayedReadOnlyMode.value),
  })
}

const ruleRowsByKind = computed<Record<PermissionRuleKind, PermissionRuleRow[]>>(() => {
  const rows: Record<PermissionRuleKind, PermissionRuleRow[]> = {
    filesystem: [],
    network: [],
    'tool-name': [],
    command: [],
  }
  const localSource = selectedSource.value === 'effective' ? 'effective' : 'local'
  const sourceLabel = (source: PermissionRuleRow['source']) =>
    source === 'inherited' ? st('Inherited') : source === 'effective' ? st('Effective') : st('This scope')
  function add(
    kind: PermissionRuleKind,
    target: string,
    condition: string,
    mode: string,
    source: PermissionRuleRow['source'],
    key: string,
  ) {
    const editable = canEdit.value && source === 'local'
    rows[kind].push({
      key,
      kind,
      target,
      condition,
      mode,
      source,
      sourceLabel: sourceLabel(source),
      modeLabel: kind === 'filesystem' ? pathModeLabel(mode) : modeLabel(mode),
      targetEditable: editable && (kind === 'filesystem' || kind === 'tool-name'),
      conditionEditable: editable && kind === 'command',
      selectable: editable && kind !== 'network',
      removable: editable,
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
      const source = selectedSource.value === 'effective' ? 'effective' : networkCustom.value ? 'local' : 'inherited'
      const editable = canEdit.value && networkCustom.value
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
        networkAction: action,
        removable: editable,
        editAction: editable,
      })
    }
  for (const [name, mode] of toolNameRules.value) add('tool-name', name, '', mode, localSource, `name:${name}`)
  for (const [name, mode] of inheritedToolNameRules.value)
    add('tool-name', name, '', mode, 'inherited', `parent:name:${name}`)
  for (const row of customCommandRules.value)
    add('command', row.tool, row.command, row.mode, localSource, `command:${commandRuleKey(row.tool, row.command)}`)
  for (const row of inheritedCommandRules.value)
    add(
      'command',
      row.tool,
      row.command,
      row.mode,
      'inherited',
      `parent:command:${commandRuleKey(row.tool, row.command)}`,
    )
  return rows
})
const selectedRuleKeysByKind = computed<Record<PermissionRuleKind, string[]>>(() => ({
  filesystem: selectedPathRules.value.map(([path]) => `path:${path}`),
  network: [],
  'tool-name': selectedToolNameRules.value.map(([name]) => `name:${name}`),
  command: selectedCommandRules.value.map((row) => `command:${commandRuleKey(row.tool, row.command)}`),
}))
const ruleGroups = computed(() =>
  [
    { kind: 'filesystem' as const, label: st('Path Rules'), addLabel: st('Add path rule') },
    { kind: 'network' as const, label: st('Network Rules'), addLabel: st('Add network rule') },
    { kind: 'tool-name' as const, label: st('Name Rules'), addLabel: st('Add tool name') },
    { kind: 'command' as const, label: st('Command Rules'), addLabel: st('Add command rule') },
  ].map((group) => ({
    ...group,
    rows: ruleRowsByKind.value[group.kind],
    selectedKeys: selectedRuleKeysByKind.value[group.kind],
  })),
)
function selectRuleRow(row: PermissionRuleRow, selected: boolean) {
  if (!row.selectable) return
  const state =
    row.kind === 'filesystem'
      ? selectedPathRuleKeys
      : row.kind === 'tool-name'
        ? selectedToolNameRuleKeys
        : selectedCommandRuleKeys
  const key = row.kind === 'command' ? commandRuleKey(row.target, row.condition) : row.target
  const keys = new Set(state.value)
  if (selected) keys.add(key)
  else keys.delete(key)
  state.value = keys
}
function selectVisibleRuleRows(kind: PermissionRuleKind, rows: PermissionRuleRow[], selected: boolean) {
  for (const row of rows) if (row.kind === kind) selectRuleRow(row, selected)
}
function setRuleRowMode(row: PermissionRuleRow, mode: string) {
  if (row.kind === 'filesystem') setPathRuleMode(row.target, mode)
  else if (row.kind === 'tool-name') setToolNameMode(row.target, mode)
  else if (row.kind === 'command') setCommandMode(row.target, row.condition, mode)
}
function renameRuleTarget(row: PermissionRuleRow, event: Event) {
  if (row.kind === 'filesystem') renamePathRule(row.target, event)
  else if (row.kind === 'tool-name') renameToolName(row.target, event)
}
function editRuleRow(row: PermissionRuleRow) {
  if (row.kind === 'network' && row.networkAction) editNetworkRule(row.networkAction, Number(row.key.split(':').at(-1)))
}
function removeRuleRow(row: PermissionRuleRow) {
  if (row.kind === 'filesystem') removePathRule(row.target)
  else if (row.kind === 'tool-name') removeToolName(row.target)
  else if (row.kind === 'command') removeCommandRule(row.target, row.condition)
  else if (row.networkAction) removeNetworkRule(row.networkAction, Number(row.key.split(':').at(-1)))
}
function beginRule(kind: PermissionRuleKind) {
  if (kind === 'network') {
    if (networkCustom.value) editNetworkRule('allow')
    return
  }
  newRuleKind.value = newRuleKind.value === kind ? '' : kind
}
function matchesQuery(value: string, query: string) {
  return value.toLowerCase().includes(query.trim().toLowerCase())
}
function isCommandClass(tool: string, command: string) {
  const keyword = normalizedClassKeyword(command)
  return canonicalPermissionToolName(tool) === 'agena.shell.write'
    ? keyword === 'dangerous'
    : ['no-op', 'noop', 'routine', 'dangerous'].includes(keyword)
}
function normalizedClassKeyword(value: string): string {
  return value
    .trim()
    .replace(/[A-Z]/g, (letter) => letter.toLowerCase())
    .replaceAll('_', '-')
}
function commandClassIdentity(value: string): string {
  const keyword = normalizedClassKeyword(value)
  return keyword === 'noop' ? 'no-op' : keyword
}
function isReadOnlyClass(value: string): boolean {
  return ['read-only', 'readonly'].includes(normalizedClassKeyword(value))
}
function readClassMode(
  rules: PermissionMode | Record<string, PermissionMode> | undefined,
  command: string,
): PermissionMode | '' {
  if (!rules || typeof rules !== 'object') return ''
  let mode: PermissionMode | '' = ''
  for (const [name, value] of Object.entries(rules))
    if (commandClassIdentity(name) === commandClassIdentity(command) && isMode(value)) mode = value
  return mode
}
function classMode(tool: string, command: string): PermissionMode | '' {
  if (tool === '*' && command === 'read-only') return readOnlyPolicy(config.value) || ''
  return readClassMode(toolEntry(config.value.tools?.rules, tool), command)
}
function classInheritanceLabel(tool: string, command: string): string {
  if (hasToolEntry(config.value.tools?.rules, tool)) return st('Default / unset')
  const parent = readClassMode(toolEntry(inheritedPermission.value?.tools?.rules, tool), command)
  return `${st('Inherit')} · ${parent ? modeLabel(parent) : st('Default / unset')}`
}
function displayedClassLabel(tool: string, command: string): string {
  const policy = hasToolEntry(config.value.tools?.rules, tool) ? config.value : inheritedPermission.value
  const mode = readClassMode(toolEntry(policy?.tools?.rules, tool), command)
  return mode ? modeLabel(mode) : st('Default / unset')
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
  customCommandRules.value.filter((row) => selectedCommandRuleKeys.value.has(commandRuleKey(row.tool, row.command))),
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
const allCommandRulesSelected = computed(() =>
  areAllSelected(
    visibleCommandRules.value.map((row) => commandRuleKey(row.tool, row.command)),
    selectedCommandRuleKeys.value,
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

function commandRuleKey(tool: string, command: string): string {
  return `${tool}\u0000${command}`
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
function toggleCommandRule(tool: string, command: string, event: Event) {
  toggleSelection(selectedCommandRuleKeys, commandRuleKey(tool, command), event)
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
function toggleAllCommandRules() {
  toggleAllSelection(
    selectedCommandRuleKeys,
    visibleCommandRules.value.map((row) => commandRuleKey(row.tool, row.command)),
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
  [visiblePathRules, visibleToolNameRules, visibleCommandRules],
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
      visibleCommandRules.value.map((row) => commandRuleKey(row.tool, row.command)),
    )
  },
  { flush: 'sync' },
)

// A first class edit copies the complete inherited tool group before overriding it.
function writeRule(tool: string, command: string, value: string) {
  mutate((next) => {
    if (tool !== '*' && isCommandClass(tool, command)) {
      const group = hasToolEntry(next.tools?.rules, tool)
        ? toolEntry(next.tools?.rules, tool)
        : toolEntry(inheritedPermission.value?.tools?.rules, tool)
      const keys =
        group && typeof group === 'object'
          ? Object.keys(group).filter((key) => commandClassIdentity(key) === commandClassIdentity(command))
          : []
      if (isMode(value)) setCommandRuleInConfig(next, tool, keys.at(-1) || command, value)
      else for (const key of keys) setCommandRuleInConfig(next, tool, key, '')
    } else setCommandRuleInConfig(next, tool, command, value)
    if (isEmptyToolSection(next.tools)) delete next.tools
  })
}

function isMode(value: unknown): value is PermissionMode {
  return value === 'allow' || value === 'auto' || value === 'ask' || value === 'deny'
}

function isShellCapableTool(value: string): boolean {
  return (SHELL_CAPABLE_TOOLS as readonly string[]).includes(canonicalPermissionToolName(value))
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

function syncRaw() {
  rawJson.value = JSON.stringify(config.value, null, 2)
  rawJsonError.value = ''
  networkJson.value = JSON.stringify(config.value.network || {}, null, 2)
  networkJsonError.value = ''
}

function mutate(mutator: (next: PermissionConfig) => void): boolean {
  if (editingDisabled.value) return false
  if (rawPending.value && !applyRawJson()) return false
  const next = clone(config.value)
  mutator(next)
  try {
    config.value = parsePermissionConfig(next)
    syncRaw()
    error.value = ''
    return true
  } catch (reason) {
    // Invalid intermediate edits stay local so several legacy rules can be
    // repaired one by one. Save always validates the complete draft again.
    config.value = next
    syncRaw()
    error.value = permissionConfigErrorMessage(reason, st)
    rawJsonError.value = error.value
    return true
  }
}

function modeAt(scope: 'workspace' | 'external'): PathPermissionMode | '' {
  const mode = config.value.path?.[scope]
  return isPathPermissionMode(mode) ? mode : ''
}

function pathInheritanceLabel(scope: 'workspace' | 'external'): string {
  const mode = inheritedPermission.value?.path?.[scope]
  const label = pathModeOptions.value.find((option) => option.value === mode)?.label
  return label ? `${st('Inherit')} · ${label}` : st('Inherit')
}

function setPathDefault(scope: 'workspace' | 'external', value: string) {
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
  mutate((next) => {
    next.path ||= {}
    next.path.rules ||= {}
    if (isPathPermissionMode(value)) next.path.rules[path] = value
    else delete next.path.rules[path]
    if (isEmptyPathSection(next.path)) delete next.path
  })
}

function removePathRule(path: string) {
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

type NetworkDefault = 'default' | 'inbound' | 'hostLoopback'

function networkMode(field: NetworkDefault): NetworkAction {
  return displayedNetwork.value?.[field] || 'deny'
}

function setNetworkSource(value: string) {
  if (formDisabled.value || networkRuleDraft.value) return
  if (value === 'inherit') {
    mutate((next) => {
      delete next.network
    })
  } else if (value === 'custom' && !networkCustom.value) {
    if (!inheritedNetwork.value) {
      inheritedNetworkError.value = st(
        'The inherited network policy could not be loaded. Refresh before customizing it.',
      )
      return
    }
    mutate((next) => {
      next.network = clone(inheritedNetwork.value!)
    })
  }
}

function setNetworkMode(field: NetworkDefault, value: string) {
  if (formDisabled.value || !networkCustom.value || (value !== 'allow' && value !== 'deny')) return
  mutate((next) => {
    const network = (next.network ||= {})
    network[field] = value
  })
}

function useNetworkPreset(mode: NetworkAction) {
  if (formDisabled.value || networkRuleDraft.value) return
  mutate((next) => {
    next.network = { default: mode }
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
  if (formDisabled.value || !networkCustom.value || networkRuleDraft.value) return
  const rule = index === null ? undefined : config.value.network?.[action]?.[index]
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
  if (!networkRuleDraft.value || !['tcp', 'udp', 'icmp', 'any'].includes(value)) return
  networkRuleDraft.value.protocol = value as NetworkProtocol
  if (value === 'icmp') networkRuleDraft.value.ports = ''
  networkRuleError.value = ''
}

function applyNetworkRule(): boolean {
  const draft = networkRuleDraft.value
  if (!draft || editingDisabled.value || rawPending.value || networkPending.value) return false
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
    const network = (next.network ||= {})
    const rules = (network[draft.action] ||= [])
    if (draft.index === null) rules.push(rule)
    else rules[draft.index] = rule
    const checked = parsePermissionConfig(next)
    config.value = checked
    cancelNetworkRule()
    syncRaw()
    error.value = ''
    return true
  } catch (reason) {
    networkRuleError.value = permissionConfigErrorMessage(reason, st)
    return false
  }
}

function removeNetworkRule(action: NetworkRuleList, index: number) {
  if (formDisabled.value || networkRuleDraft.value || !networkCustom.value) return
  mutate((next) => {
    const rules = next.network?.[action]
    if (!rules) return
    rules.splice(index, 1)
    if (!rules.length) delete next.network![action]
  })
}

function applyNetworkJson(): boolean {
  if (editingDisabled.value || rawPending.value || networkRuleDraft.value) return false
  try {
    const next = clone(config.value)
    next.network = parseNetworkPermissionConfig(JSON.parse(networkJson.value))
    config.value = parsePermissionConfig(next)
    syncRaw()
    error.value = ''
    return true
  } catch (reason) {
    networkJsonError.value = permissionConfigErrorMessage(reason, st)
    return false
  }
}

function useNetworkExample() {
  if (editingDisabled.value || rawPending.value || networkRuleDraft.value) return
  networkJson.value = JSON.stringify(
    { default: 'deny', allow: [{ to: '192.0.2.10', protocol: 'tcp', ports: [443, '8000-8099'] }] },
    null,
    2,
  )
  networkJsonError.value = ''
}

function setToolDefault(value: string) {
  mutate((next) => {
    next.tools ||= {}
    if (isMode(value)) next.tools.default = value
    else delete next.tools.default
    if (isEmptyToolSection(next.tools)) delete next.tools
  })
}

function setToolNameMode(name: string, value: string) {
  mutate((next) => {
    next.tools ||= {}
    next.tools.names ||= {}
    writeToolEntry(next.tools.names, name, isMode(value) ? value : undefined)
  })
}

function removeToolName(name: string) {
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

function setCommandMode(tool: string, command: string, value: string) {
  if (!isShellCapableTool(tool) && tool !== '*') return
  mutate((next) => {
    setCommandRuleInConfig(next, tool, command, value)
  })
}

function removeCommandRule(tool: string, command: string) {
  mutate((next) => {
    setCommandRuleInConfig(next, tool, command, '')
    if (isEmptyToolSection(next.tools)) delete next.tools
  })
}

function setCommandRuleInConfig(next: PermissionConfig, tool: string, command: string, value: string) {
  if (!isShellCapableTool(tool) && !(tool === '*' && command === 'read-only')) return
  tool = canonicalPermissionToolName(tool)
  next.tools ||= {}
  next.tools.rules ||= {}
  // A tool's rule group replaces its parent as a whole. A first local edit
  // starts with that complete group so other inherited patterns stay active.
  const current = hasToolEntry(next.tools.rules, tool)
    ? toolEntry(next.tools.rules, tool)
    : isMode(value)
      ? toolEntry(inheritedPermission.value?.tools?.rules, tool)
      : undefined
  const rules: Record<string, PermissionMode> = isMode(current)
    ? tool === '*'
      ? {}
      : { '*': current }
    : current && typeof current === 'object' && !Array.isArray(current)
      ? ({ ...current } as Record<string, PermissionMode>)
      : {}
  if (tool === '*' && command === 'read-only') {
    for (const key of Object.keys(rules)) if (isReadOnlyClass(key)) delete rules[key]
  }
  if (isMode(value)) rules[command] = value
  else delete rules[command]
  writeToolEntry(next.tools.rules, tool, Object.keys(rules).length ? rules : undefined)
}

function setSelectedCommandMode(value: string) {
  const rows = selectedCommandRules.value.map(({ tool, command }) => ({ tool, command }))
  if (!rows.length || !isMode(value)) return
  mutate((next) => {
    for (const { tool, command } of rows) setCommandRuleInConfig(next, tool, command, value)
  })
}

function removeSelectedCommandRules() {
  const rows = selectedCommandRules.value.map(({ tool, command }) => ({ tool, command }))
  if (!rows.length) return
  mutate((next) => {
    for (const { tool, command } of rows) setCommandRuleInConfig(next, tool, command, '')
    if (isEmptyToolSection(next.tools)) delete next.tools
  })
  clearSelection(selectedCommandRuleKeys)
}

function renameCommandRule(tool: string, command: string, event: Event) {
  const input = event.target as HTMLInputElement
  if (tool === '*') {
    input.value = command
    return
  }
  const nextCommand = input.value.trim()
  if (!nextCommand || nextCommand === command) {
    input.value = command
    return
  }
  const existing = toolEntry(config.value.tools?.rules, tool)
  const rules = isMode(existing) ? { '*': existing } : existing
  if (rules && typeof rules === 'object' && Object.prototype.hasOwnProperty.call(rules, nextCommand)) {
    error.value = st('Command rule already exists for {tool}: {nextCommand}', { tool: tool, nextCommand: nextCommand })
    input.value = command
    return
  }
  mutate((next) => {
    const raw = toolEntry(next.tools?.rules, tool)
    const nextRules: Record<string, PermissionMode> = isMode(raw)
      ? { '*': raw }
      : raw && typeof raw === 'object' && !Array.isArray(raw)
        ? ({ ...raw } as Record<string, PermissionMode>)
        : {}
    if (!nextRules[command]) return
    writeToolEntry(next.tools!.rules!, tool, renamePermissionRule(nextRules, command, nextCommand))
  })
  error.value = ''
  input.value = nextCommand
}

function addPathRule() {
  let path: string
  try {
    path = canonicalPermissionPath(newPath.value)
  } catch (reason) {
    error.value = permissionConfigErrorMessage(reason, st)
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
  const name = canonicalPermissionToolName(newToolName.value.trim())
  if (!name) return
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
  const tool = canonicalPermissionToolName(newCommandTool.value.trim())
  const command = newCommandPattern.value.trim()
  if (!isShellCapableTool(tool) || !command) return
  const existing = toolEntry(config.value.tools?.rules, tool)
  if (
    (isMode(existing) && command === '*') ||
    (existing && typeof existing === 'object' && Object.prototype.hasOwnProperty.call(existing, command))
  ) {
    error.value = st('Command rule already exists for {tool}: {nextCommand}', { tool, nextCommand: command })
    return
  }
  setCommandMode(tool, command, newCommandMode.value)
  error.value = ''
  newCommandPattern.value = ''
}

function applyRawJson(): boolean {
  if (editingDisabled.value || networkPending.value || networkRuleDraft.value) return false
  try {
    const parsed = JSON.parse(rawJson.value) as JsonValue
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed))
      throw new Error(st('Permission config must be a JSON object.'))
    config.value = parsePermissionConfig(parsed)
    syncRaw()
    return true
  } catch (reason) {
    rawJsonError.value = permissionConfigErrorMessage(reason, st)
    rawEditorOpen.value = true
    return false
  }
}

const contextKey = computed(() => {
  const source = selectedSource.value
  // Layer endpoints belong to the server runtime. Only session-backed reads
  // change their target when the client opens another conversation.
  return JSON.stringify(source === 'session' || source === 'effective' ? [source, activeSessionId.value] : [source])
})
type PermissionDraft = {
  config: PermissionConfig
  saved: PermissionConfig
  raw: string
  network: string
  networkRule: NetworkRuleDraft | null
}
const drafts = new Map<string, PermissionDraft>()
let loadedContext = ''
let loadGeneration = 0
let loadController: AbortController | undefined

function rememberDraft() {
  if (!loadedContext || !loaded.value) return
  if (dirty.value)
    drafts.set(loadedContext, {
      config: clone(config.value),
      saved: clone(savedConfig.value),
      raw: rawJson.value,
      network: networkJson.value,
      networkRule: clone(networkRuleDraft.value),
    })
  else drafts.delete(loadedContext)
}

type PolicyRead = { policy: PermissionConfig; inherited?: unknown; configPath?: string }
async function readPolicy(
  source: PermissionSource,
  sessionId: number | null,
  signal: AbortSignal,
): Promise<PolicyRead> {
  if (source === 'global' || source === 'workspace') {
    const response = await apiJson<SettingsResponse>(`/api/v1/settings/layers/${source}?path=permission`, { signal })
    return { policy: asConfig(response.value), inherited: response.inherited_value, configPath: response.config_path }
  }
  if (sessionId) {
    const session = await apiJson<SessionStateResponse>(`/api/v1/sessions/${sessionId}/state`, { signal })
    const execution = session.execution
    const value =
      source === 'session'
        ? execution?.execution?.selected_permission ||
          execution?.selected_permission ||
          execution?.context?.selected_permission
        : execution?.execution?.effective_permission ||
          execution?.effective_permission ||
          execution?.context?.effective_permission
    const inherited =
      execution?.execution?.inherited_permission ||
      execution?.inherited_permission ||
      execution?.context?.inherited_permission
    const policy = asConfig(value)
    if (source === 'session' || Object.keys(policy).length) return { policy, inherited }
  }
  if (source === 'session') return { policy: {} }
  const response = await apiJson<SettingsResponse>('/api/v1/settings?source=effective&path=permission', { signal })
  return { policy: asConfig(response.value) }
}

async function load() {
  const key = contextKey.value
  const source = selectedSource.value
  const sessionId = activeSessionId.value
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
    configPath.value = ''
    inheritedNetworkError.value = ''
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
    const [policy, catalog] = await Promise.allSettled([
      readPolicy(source, sessionId, controller.signal),
      apiJson<ToolCatalogResponse>('/api/v1/plugins/surface', { signal: controller.signal }),
    ])
    if (generation !== loadGeneration || key !== contextKey.value) return
    if (catalog.status === 'fulfilled') {
      toolCatalog.value = (Array.isArray(catalog.value?.permission_tools) ? catalog.value.permission_tools : [])
        .map((item) => ({ name: String(item?.name || '').trim(), summary: String(item?.summary || '').trim() }))
        .filter((item) => item.name)
        .sort((a, b) => a.name.localeCompare(b.name))
    }
    if (policy.status === 'rejected') throw policy.reason
    configPath.value = String(policy.value.configPath || '')
    inheritedPermission.value = null
    inheritedNetworkError.value = ''
    if (source !== 'effective') {
      try {
        const parent = parsePermissionConfig(policy.value.inherited)
        inheritedPermission.value = parent
        if (parent.network === undefined)
          throw new Error(st('The inherited network policy could not be loaded. Refresh before customizing it.'))
      } catch {
        inheritedNetworkError.value = st(
          'The inherited network policy could not be loaded. Refresh before customizing it.',
        )
      }
    }
    // A refresh updates server data only when the editor has no pending work.
    if (!dirty.value || !loaded.value) {
      config.value = clone(policy.value.policy)
      savedConfig.value = clone(policy.value.policy)
      syncRaw()
    }
    loaded.value = true
    try {
      parsePermissionConfig(config.value)
    } catch (reason) {
      rawJsonError.value = permissionConfigErrorMessage(reason, st)
      rawEditorOpen.value = true
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
  cancelNetworkRule()
  config.value = clone(savedConfig.value)
  syncRaw()
  error.value = ''
  saveError.value = ''
  drafts.delete(loadedContext)
  clearAllRuleSelections()
}

async function save() {
  if (editingDisabled.value || !dirty.value) return
  if (networkRuleDraft.value && !applyNetworkRule()) return
  if (networkPending.value && !applyNetworkJson()) return
  if (rawPending.value && !applyRawJson()) return
  const key = contextKey.value
  const source = selectedSource.value
  const sessionId = activeSessionId.value
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
    if (source === 'session') {
      const response = await chat.setSessionPermission(String(sessionId), snapshot as JsonValue)
      const execution = response.execution as SessionStateResponse['execution']
      savedPermission = asConfig(
        execution?.execution?.selected_permission ||
          execution?.selected_permission ||
          execution?.context?.selected_permission,
      )
    } else {
      const response = !Object.keys(snapshot).length
        ? await deleteRuntimeSetting('permission', { reload: true }, source as 'global' | 'workspace')
        : await setRuntimeSetting(
            'permission',
            snapshot as JsonValue,
            { reload: true },
            source as 'global' | 'workspace',
          )
      // `current` is the persisted layer value even when runtime reload is
      // deferred. Do not replace it with a potentially older effective read.
      // Older response shapes must be confirmed by reading the same raw layer.
      savedPermission = parsePermissionConfig(
        response.current !== undefined
          ? (response.current ?? {})
          : (await readPolicy(source, sessionId, new AbortController().signal)).policy,
      )
    }
    drafts.delete(key)
    if (key === contextKey.value) {
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

async function clearSource() {
  if (editingDisabled.value) return
  const key = contextKey.value
  const confirmed = await confirmAction({
    title: st('Reset permissions in this scope?'),
    description: st(
      'This resets all permission overrides in this scope, including advanced fields. Save to apply the reset.',
    ),
    confirmText: st('Reset'),
    cancelText: st('Cancel'),
  })
  if (!confirmed || editingDisabled.value || key !== contextKey.value) return
  cancelNetworkRule()
  config.value = {}
  syncRaw()
  error.value = ''
  clearAllRuleSelections()
}

async function canLeave() {
  if (saving.value || rulesPanel.value?.busy) return false
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
  if (!ownsNavigation(to)) return true
  const sourceRoute =
    workspacePane && routeWindowId(from) && routeWindowId(from) !== workspacePane.windowId.value ? route : from
  const source = normalizePermissionSource(sourceRoute.query.scope || selectedSource.value)
  const recordedSessionId = lastSessionId.value || chat.selectedSessionId
  if (
    to.params.section === 'permissions' &&
    isPermissionAccessView(to.query.view || PERMISSION_ACCESS_VIEW) &&
    normalizePermissionSource(to.query.scope || source) === source &&
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
onBeforeRouteLeave((to) => !ownsNavigation(to) || canLeave())
function beforeUnload(event: BeforeUnloadEvent) {
  if (!dirty.value && !drafts.size) return
  event.preventDefault()
  event.returnValue = ''
}
async function refresh() {
  if (saving.value) return
  await Promise.allSettled([load(), rulesPanel.value?.refresh()])
}
defineExpose({ refresh })
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
  return scrollToPermissionElement(`[data-permission-section="${section}"]`)
}
function selectSection(section: PermissionAccessSection) {
  const hash = permissionAccessHash(section)
  if (route.hash === hash) void scrollToSection(section)
  else void router.replace({ path: route.path, query: route.query, hash })
}
function selectRuleGroup(kind: PermissionRuleKind) {
  const hash = '#permission-rules-' + kind
  if (route.hash === hash) void scrollToPermissionElement(`[data-permission-rule-table="${kind}"]`)
  else void router.replace({ path: route.path, query: route.query, hash })
}
watch(
  [loaded, () => route.hash],
  ([ready, hash]) => {
    if (!ready) return
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
onMounted(() => {
  void load()
  window.addEventListener('beforeunload', beforeUnload)
})
onBeforeUnmount(() => {
  loadGeneration += 1
  loadController?.abort()
  window.removeEventListener('beforeunload', beforeUnload)
})
</script>

<template>
  <section ref="editorElement" class="permission-studio min-w-0" :aria-busy="loading || saving">
    <PermissionAccessToolbar
      :scope="selectedSource"
      :session-id="sessionId"
      :config-path="configPath"
      :disabled="saving || Boolean(rulesPanel?.busy)"
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
          v-if="canEdit"
          variant="ghost"
          size="sm"
          class="h-8 w-8"
          :disabled="editingDisabled || !Object.keys(config).length"
          :tooltip="$st('Reset scope')"
          :aria-label="$st('Reset scope')"
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
    <div v-else-if="!loaded && loading" class="my-2 grid grid-cols-3 gap-2" aria-hidden="true">
      <div v-for="n in 3" :key="n" class="h-32 animate-pulse rounded border border-border bg-muted/20" />
    </div>
    <template v-else-if="loaded">
      <nav class="permission-mobile-summary" :aria-label="$st('Access permission sections')" data-permission-sections>
        <a
          v-for="section in sections"
          :key="section.id"
          :href="permissionAccessHash(section.id)"
          :data-permission-summary="section.id"
          @click.prevent="selectSection(section.id)"
        >
          <span class="flex items-center gap-1.5 font-medium"
            ><component :is="section.icon" class="h-4 w-4 shrink-0" />{{ section.label }}</span
          >
          <span class="text-muted-foreground">{{ sectionSummary(section.id) }}</span>
        </a>
      </nav>

      <div class="permission-default-grid">
        <section
          class="permission-domain"
          data-permission-section="filesystem"
          :aria-label="$st('File access')"
          tabindex="-1"
        >
          <header class="permission-domain-heading">
            <h2>
              <a :href="permissionAccessHash('filesystem')" @click.prevent="selectSection('filesystem')"
                ><RiFolderLine class="h-4 w-4" />{{ $st('File access') }}</a
              >
            </h2>
            <PermissionHelp
              class="ml-auto"
              :text="
                $st('Choose readonly, readwrite, or deny for workspace and external paths.') +
                ' ' +
                $st(
                  'Use concrete files or directories. Directories include all descendants; denied paths cannot be reopened.',
                )
              "
            />
          </header>
          <PermissionPolicyRow
            v-for="scope in pathScopes"
            :key="scope.id"
            :label="scope.label"
            :hint="scope.description"
            :value="pathModeLabel(displayedPathMode(scope.id))"
            :editable="canEdit"
            :origin="canEdit && !modeAt(scope.id) ? $st('Inherited') : ''"
            :data-permission-default="'path.' + scope.id"
          >
            <OptionPicker
              :model-value="modeAt(scope.id)"
              :options="pathModeOptions"
              :empty-label="pathInheritanceLabel(scope.id)"
              :display-label="pathModeLabel(displayedPathMode(scope.id))"
              :title="$st('Path access mode')"
              :disabled="formDisabled"
              size="sm"
              @update:model-value="setPathDefault(scope.id, $event)"
            />
          </PermissionPolicyRow>
        </section>
        <section
          class="permission-domain"
          data-permission-section="network"
          :aria-label="$st('Network access')"
          tabindex="-1"
        >
          <header class="permission-domain-heading">
            <h2>
              <a :href="permissionAccessHash('network')" @click.prevent="selectSection('network')"
                ><RiGlobalLine class="h-4 w-4" />{{ $st('Network access') }}</a
              >
            </h2>
            <div v-if="canEdit" class="ml-auto w-28 shrink-0" data-permission-default="network.source">
              <OptionPicker
                :model-value="networkCustom ? 'custom' : 'inherit'"
                :options="networkSourceOptions"
                :include-empty="false"
                :display-label="networkCustom ? $st('Custom') : $st('Inherited')"
                :title="$st('Network configuration')"
                :disabled="formDisabled || Boolean(networkRuleDraft)"
                size="sm"
                @update:model-value="setNetworkSource"
              />
            </div>
            <PermissionHelp
              :text="
                $st('Custom network settings replace the parent policy as a whole. Missing values deny access.') +
                ' ' +
                $st('Customizing starts with a copy of the inherited policy.') +
                ' ' +
                $st('Deny rules take precedence over every allow rule.')
              "
            />
          </header>
          <PermissionPolicyRow
            :label="$st('Outbound default')"
            :hint="$st('Action when no outbound rule matches.')"
            :value="modeLabel(networkMode('default'))"
            :editable="canEdit && networkCustom"
            data-permission-default="network.default"
            ><OptionPicker
              :model-value="networkMode('default')"
              :options="networkActionOptions"
              :include-empty="false"
              :title="$st('Outbound default')"
              :disabled="formDisabled"
              size="sm"
              @update:model-value="setNetworkMode('default', $event)"
          /></PermissionPolicyRow>
          <PermissionPolicyRow
            v-for="field in networkAdvancedDefaults"
            :key="field.id"
            :label="field.label"
            :hint="field.description"
            :value="modeLabel(networkMode(field.id))"
            :editable="canEdit && networkCustom"
            :data-permission-default="'network.' + field.id"
            ><OptionPicker
              :model-value="networkMode(field.id)"
              :options="networkActionOptions"
              :include-empty="false"
              :title="field.label"
              :disabled="formDisabled"
              size="sm"
              @update:model-value="setNetworkMode(field.id, $event)"
          /></PermissionPolicyRow>
          <p v-if="inheritedNetworkError && !networkCustom" role="alert" class="py-1 text-[13px] text-destructive">
            {{ inheritedNetworkError }}
          </p>
        </section>
        <section
          class="permission-domain"
          data-permission-section="tools"
          :aria-label="$st('Tool execution & approvals')"
          tabindex="-1"
        >
          <header class="permission-domain-heading">
            <h2>
              <a :href="permissionAccessHash('tools')" @click.prevent="selectSection('tools')"
                ><RiToolsLine class="h-4 w-4" />{{ $st('Tool execution & approvals') }}</a
              >
            </h2>
            <PermissionHelp
              class="ml-auto"
              :text="
                $st('Set the fallback policy and access for tools that only read data.') +
                ' ' +
                $st('Choose a tool to override its default access policy.') +
                ' ' +
                $st(
                  'Match shell commands with patterns such as git push *. The last matching command rule wins; a denied tool stays denied.',
                )
              "
            />
          </header>
          <PermissionPolicyRow
            :label="$st('Default tool mode')"
            :hint="$st('Used when no tool-specific rule matches.')"
            :value="modeLabel(displayedToolDefault)"
            :editable="canEdit"
            :origin="canEdit && !config.tools?.default ? $st('Inherited') : ''"
            data-permission-default="tools.default"
            ><OptionPicker
              :model-value="config.tools?.default || ''"
              :options="modeOptions"
              :empty-label="toolInheritanceLabel()"
              :display-label="modeLabel(displayedToolDefault)"
              :title="$st('Default tool mode')"
              :disabled="formDisabled"
              size="sm"
              @update:model-value="setToolDefault"
          /></PermissionPolicyRow>
          <PermissionPolicyRow
            :label="$st('Read-only tools')"
            :hint="$st('Read-only Auto follows the default tool policy; named tool rules take precedence.')"
            :value="modeLabel(displayedReadOnlyMode)"
            :editable="canEdit"
            :origin="
              canEdit && !Object.hasOwn(config.tools?.rules || {}, '*')
                ? $st('Inherited')
                : displayedReadOnlyPolicy === 'auto'
                  ? $st('Auto')
                  : ''
            "
            data-permission-default="tools.read-only"
            ><OptionPicker
              :model-value="readOnlyEditorMode"
              :options="readOnlyModeOptions"
              :empty-label="readOnlyInheritanceLabel()"
              :display-label="modeLabel(displayedReadOnlyMode)"
              :title="$st('Read-only tools')"
              :disabled="formDisabled"
              size="sm"
              @update:model-value="writeRule('*', 'read-only', $event)"
          /></PermissionPolicyRow>
        </section>
      </div>
      <nav class="permission-rule-navigation" :aria-label="$st('Access rules')">
        <a
          v-for="group in ruleGroups"
          :key="group.kind"
          :href="'#permission-rules-' + group.kind"
          :data-permission-rule-link="group.kind"
          @click.prevent="selectRuleGroup(group.kind)"
        >
          {{ group.label }}<span>{{ group.rows.length }}</span>
        </a>
      </nav>
      <PermissionRuleTable
        v-for="group in ruleGroups"
        :id="'permission-rules-' + group.kind"
        :key="group.kind"
        class="permission-rule-group"
        tabindex="-1"
        :kind="group.kind"
        :rows="group.rows"
        :selected-keys="group.selectedKeys"
        :disabled="formDisabled || Boolean(networkRuleDraft)"
        @select="selectRuleRow"
        @select-visible="(rows, selected) => selectVisibleRuleRows(group.kind, rows, selected)"
        @mode="setRuleRowMode"
        @rename-target="renameRuleTarget"
        @rename-condition="(row, event) => renameCommandRule(row.target, row.condition, event)"
        @edit="editRuleRow"
        @remove="removeRuleRow"
      >
        <template v-if="canEdit" #actions>
          <Button
            variant="outline"
            size="sm"
            class="h-8 px-2 text-[13px]"
            :disabled="formDisabled || Boolean(networkRuleDraft) || (group.kind === 'network' && !networkCustom)"
            :data-permission-create-toggle="group.kind"
            :aria-expanded="newRuleKind === group.kind"
            @click="beginRule(group.kind)"
            ><RiAddLine class="mr-1 h-3.5 w-3.5" />{{ group.addLabel }}</Button
          >
        </template>
        <template #bulk>
          <div v-if="group.kind === 'filesystem' && selectedPathRules.length" class="permission-bulk-group">
            <span>{{ $st('Path Rules') }} · {{ selectedPathRules.length }}</span
            ><OptionPicker
              v-model="bulkPathMode"
              :options="pathModeOptions"
              :include-empty="false"
              :title="$st('Path access mode')"
              size="sm"
              :disabled="formDisabled"
            /><Button variant="outline" size="sm" :disabled="formDisabled" @click="applySelectedPathMode">{{
              $st('Apply')
            }}</Button
            ><Button variant="ghost" size="sm" :disabled="formDisabled" @click="removeSelectedPathRules">{{
              $st('Remove selected')
            }}</Button
            ><IconButton
              size="sm"
              variant="ghost"
              :aria-label="$st('Select none')"
              :tooltip="$st('Select none')"
              @click="clearPathRuleSelection"
              ><RiCloseLine class="h-4 w-4"
            /></IconButton>
          </div>
          <div v-if="group.kind === 'tool-name' && selectedToolNameRules.length" class="permission-bulk-group">
            <span>{{ $st('Name Rules') }} · {{ selectedToolNameRules.length }}</span
            ><OptionPicker
              v-model="bulkToolNameMode"
              :options="modeOptions"
              :include-empty="false"
              :title="$st('Tool name mode')"
              size="sm"
              :disabled="formDisabled"
            /><Button
              variant="outline"
              size="sm"
              :disabled="formDisabled"
              @click="setSelectedToolNameMode(bulkToolNameMode)"
              >{{ $st('Apply') }}</Button
            ><Button variant="ghost" size="sm" :disabled="formDisabled" @click="removeSelectedToolNameRules">{{
              $st('Remove selected')
            }}</Button
            ><IconButton
              size="sm"
              variant="ghost"
              :aria-label="$st('Select none')"
              :tooltip="$st('Select none')"
              @click="clearToolNameRuleSelection"
              ><RiCloseLine class="h-4 w-4"
            /></IconButton>
          </div>
          <div v-if="group.kind === 'command' && selectedCommandRules.length" class="permission-bulk-group">
            <span>{{ $st('Command Rules') }} · {{ selectedCommandRules.length }}</span
            ><OptionPicker
              v-model="bulkCommandMode"
              :options="modeOptions"
              :include-empty="false"
              :title="$st('Command rule mode')"
              size="sm"
              :disabled="formDisabled"
            /><Button
              variant="outline"
              size="sm"
              :disabled="formDisabled"
              @click="setSelectedCommandMode(bulkCommandMode)"
              >{{ $st('Apply') }}</Button
            ><Button variant="ghost" size="sm" :disabled="formDisabled" @click="removeSelectedCommandRules">{{
              $st('Remove selected')
            }}</Button
            ><IconButton
              size="sm"
              variant="ghost"
              :aria-label="$st('Select none')"
              :tooltip="$st('Select none')"
              @click="clearCommandRuleSelection"
              ><RiCloseLine class="h-4 w-4"
            /></IconButton>
          </div>
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
                  /><Button type="submit" variant="outline" size="sm" :disabled="formDisabled || !newPath.trim()">{{
                    $st('Add path rule')
                  }}</Button>
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
                  /><Button type="submit" variant="outline" size="sm" :disabled="formDisabled || !newToolName.trim()">{{
                    $st('Add tool name')
                  }}</Button>
                </div>
              </form></template
            ><template v-else
              ><form class="permission-create-form" @submit.prevent="addCommandRule">
                <label class="permission-control"
                  ><span>{{ $st('Tool name') }}</span
                  ><OptionPicker
                    v-model="newCommandTool"
                    :options="shellToolOptions"
                    :include-empty="false"
                    :title="$st('Shell-capable tool')"
                    :disabled="formDisabled"
                    size="sm"
                    monospace /></label
                ><label class="permission-control"
                  ><span>{{ $st('Command pattern') }}</span
                  ><Input
                    v-model="newCommandPattern"
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
                    :disabled="formDisabled"
                    size="sm"
                  /><Button
                    type="submit"
                    variant="outline"
                    size="sm"
                    :disabled="formDisabled || !newCommandPattern.trim()"
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
                v-model="networkRuleDraft.action"
                :options="networkActionOptions"
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
            <p v-if="networkRuleError" role="alert" class="text-[13px] text-destructive">{{ networkRuleError }}</p>
            <div class="flex gap-1">
              <Button
                type="submit"
                variant="outline"
                size="sm"
                :disabled="editingDisabled"
                data-permission-network-apply
                >{{ $st('Apply rule') }}</Button
              ><Button type="button" variant="ghost" size="sm" :disabled="editingDisabled" @click="cancelNetworkRule">{{
                $st('Cancel')
              }}</Button>
            </div>
          </form>
        </template>
      </PermissionRuleTable>
      <div class="permission-secondary-grid">
        <PermissionsPanel :scope="selectedSource" :session-id="sessionId" compact ref="rulesPanel" />
        <section class="permission-advanced min-w-0">
          <div
            v-if="canEdit"
            class="permission-presets"
            :title="$st('Presets replace the current network draft. Save to apply.')"
          >
            <Button
              variant="ghost"
              size="sm"
              :disabled="formDisabled || Boolean(networkRuleDraft)"
              @click="useNetworkPreset('deny')"
              >{{ $st('Block external network') }}</Button
            >
            <Button
              variant="ghost"
              size="sm"
              :disabled="formDisabled || Boolean(networkRuleDraft)"
              @click="useNetworkPreset('allow')"
              >{{ $st('Allow outbound by default') }}</Button
            >
          </div>
          <details v-if="networkCustom" class="permission-details">
            <summary class="permission-summary">
              <RiCodeSSlashLine class="h-4 w-4" />{{ $st('Network policy JSON')
              }}<span v-if="networkPending" class="ml-auto h-1.5 w-1.5 rounded-full bg-primary" />
            </summary>
            <div class="permission-json-body">
              <p class="text-[13px] text-muted-foreground">
                {{
                  $st(
                    'This object defines the complete network policy. Empty lists and omitted fields do not inherit parent rules.',
                  )
                }}
              </p>
              <textarea
                v-model="networkJson"
                class="permission-json"
                rows="10"
                spellcheck="false"
                :readonly="editingDisabled || rawPending || Boolean(networkRuleDraft)"
                :aria-label="$st('Network policy JSON')"
                :aria-invalid="Boolean(networkJsonError)"
                @input="networkJsonError = ''"
              />
              <p v-if="networkJsonError" class="text-[13px] text-destructive" role="alert">{{ networkJsonError }}</p>
              <div v-if="canEdit" class="flex flex-wrap gap-1">
                <Button
                  variant="outline"
                  size="sm"
                  :disabled="editingDisabled || rawPending || Boolean(networkRuleDraft) || !networkPending"
                  @click="applyNetworkJson"
                  >{{ $st('Apply JSON') }}</Button
                ><Button
                  variant="ghost"
                  size="sm"
                  :disabled="editingDisabled || rawPending || Boolean(networkRuleDraft)"
                  @click="useNetworkExample"
                  >{{ $st('Load CIDR example') }}</Button
                >
              </div>
            </div>
          </details>
          <details class="permission-details" data-command-classes>
            <summary class="permission-summary">
              <RiTerminalBoxLine class="h-4 w-4" />{{ $st('Command categories')
              }}<PermissionHelp :text="$st('Review no-op, routine, and dangerous commands for each shell tool.')" />
            </summary>
            <div class="permission-json-body">
              <div v-for="tool in SHELL_CAPABLE_TOOLS" :key="tool" class="command-class-row">
                <code class="permission-target-text">{{ tool }}</code
                ><PermissionPolicyRow
                  v-for="category in commandClasses.filter(
                    (item) => tool !== 'agena.shell.write' || item.id === 'dangerous',
                  )"
                  :key="category.id"
                  :label="category.label"
                  :value="displayedClassLabel(tool, category.id)"
                  :editable="canEdit"
                  ><OptionPicker
                    :model-value="classMode(tool, category.id)"
                    :options="modeOptions"
                    :empty-label="classInheritanceLabel(tool, category.id)"
                    :title="category.label"
                    :disabled="formDisabled"
                    size="sm"
                    @update:model-value="writeRule(tool, category.id, $event)"
                /></PermissionPolicyRow>
              </div>
            </div>
          </details>
          <details
            class="permission-details permission-raw-editor"
            :open="rawEditorOpen"
            @toggle="rawEditorOpen = ($event.target as HTMLDetailsElement).open"
          >
            <summary class="permission-summary">
              <RiCodeSSlashLine class="h-4 w-4" />{{ $st('Advanced permission configuration')
              }}<span v-if="rawPending" class="h-1.5 w-1.5 rounded-full bg-primary" />
            </summary>
            <div class="permission-json-body">
              <p class="text-[13px] text-muted-foreground">
                {{ $st('Edit file, network, and tool permissions together as JSON for this scope.') }}
              </p>
              <textarea
                v-model="rawJson"
                rows="12"
                spellcheck="false"
                class="permission-json"
                :aria-label="$st('Raw PermissionConfig')"
                :readonly="editingDisabled || networkPending || Boolean(networkRuleDraft)"
                :aria-invalid="Boolean(rawJsonError)"
              />
              <p v-if="rawJsonError" class="text-[13px] text-destructive" role="alert">{{ rawJsonError }}</p>
              <div v-if="canEdit" class="flex justify-end">
                <Button variant="outline" size="sm" :disabled="editingDisabled || !rawPending" @click="applyRawJson">{{
                  $st('Apply JSON')
                }}</Button>
              </div>
            </div>
          </details>
          <div class="flex items-center justify-end">
            <PermissionHelp
              :text="
                $st(
                  'macOS does not support per-destination outbound rules. Linux requires inbound and host-loopback access to remain denied.',
                )
              "
            />
          </div>
        </section>
      </div>
    </template>
  </section>
</template>

<style scoped>
@reference '../../style.css';
.permission-studio {
  container: permissions / inline-size;
  font-size: 13px;
}
.permission-status {
  @apply text-muted-foreground;
  display: inline-flex;
  align-items: center;
  gap: 0.25rem;
  font-size: 13px;
}
.permission-default-grid {
  display: grid;
  gap: 0.75rem;
  align-items: start;
  margin: 0.5rem 0;
}
.permission-rule-group {
  @apply border-t border-border/60;
  margin-top: 0.75rem;
  min-width: 0;
  scroll-margin-top: 5rem;
  outline: none;
}
.permission-rule-navigation {
  display: flex;
  flex-wrap: wrap;
  gap: 0.25rem 0.75rem;
  padding-block: 0.25rem;
}
.permission-rule-navigation a {
  @apply text-muted-foreground hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring;
  display: inline-flex;
  align-items: center;
  gap: 0.375rem;
  min-height: 1.75rem;
  font-size: 13px;
}
.permission-rule-navigation span {
  @apply text-foreground;
  font-variant-numeric: tabular-nums;
}
.permission-domain {
  min-width: 0;
  padding: 0 0.5rem 0.375rem;
  scroll-margin-top: 5rem;
  outline: none;
}
.permission-domain-heading {
  @apply border-b border-border/60;
  display: flex;
  min-width: 0;
  min-height: 40px;
  align-items: center;
  gap: 0.375rem;
  margin-bottom: 0.25rem;
}
.permission-domain-heading h2 {
  min-width: 0;
  font-size: 14px;
  font-weight: 600;
}
.permission-domain-heading h2 a {
  display: flex;
  align-items: center;
  gap: 0.375rem;
}
.permission-presets {
  display: flex;
  flex-wrap: wrap;
  gap: 0 0.25rem;
  padding-bottom: 0.25rem;
}
.permission-presets :deep(button) {
  padding-inline: 0.375rem;
  font-size: 13px;
}
.permission-control {
  display: grid;
  flex: 1 1 14rem;
  min-width: 0;
  gap: 0.25rem;
}
.permission-control > span {
  @apply text-muted-foreground;
  font-size: 13px;
}
.permission-rule-form {
  @apply rounded border border-primary/20 bg-primary/5;
  margin: 0.375rem 0;
  padding: 0.5rem;
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
  flex: 1 1 20rem;
  grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
  align-items: end;
  gap: 0.5rem;
}
.permission-create-actions :deep(button) {
  font-size: 13px;
}
.permission-create-form > details,
.permission-create-form > p {
  flex: 1 1 100%;
}
.permission-bulk-group {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 0.375rem;
  font-size: 13px;
}
.permission-bulk-group > :deep(.relative) {
  width: 10rem;
}
.permission-secondary-grid {
  display: grid;
  gap: 0.75rem;
  align-items: start;
  margin-top: 0.75rem;
}
.permission-details {
  @apply border-t border-border/60;
  min-width: 0;
}
.permission-summary {
  @apply focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring;
  display: flex;
  min-height: 34px;
  cursor: pointer;
  list-style: none;
  align-items: center;
  gap: 0.375rem;
  font-size: 13px;
  font-weight: 500;
}
.permission-summary::-webkit-details-marker {
  display: none;
}
.permission-json-body {
  display: grid;
  gap: 0.5rem;
  min-width: 0;
  padding: 0.5rem 0;
}
.permission-json {
  @apply rounded border border-input bg-background p-2 outline-none focus-visible:ring-1 focus-visible:ring-ring;
  width: 100%;
  min-width: 0;
  resize: vertical;
  font-family: var(--font-mono);
  font-size: 13px;
  line-height: 1.5;
}
.command-class-row {
  @apply border-t border-border/40;
  padding: 0.375rem 0;
}
.permission-target-text {
  display: block;
  overflow-wrap: anywhere;
  font-size: 13px;
  line-height: 1.4;
}
.permission-mobile-summary {
  display: grid;
  margin-top: 0.5rem;
}
.permission-mobile-summary a {
  @apply border-b border-border/50;
  display: grid;
  grid-template-columns: minmax(6.5rem, 35%) minmax(0, 1fr);
  gap: 0.375rem;
  align-items: center;
  padding: 0.375rem 0;
  font-size: 13px;
  line-height: 1.4;
}
.permission-studio :deep(button[aria-haspopup='dialog']) {
  min-height: 32px;
  height: auto;
  font-size: 13px;
  padding: 0.25rem 0.375rem;
}
.permission-domain :deep(button[aria-haspopup='dialog'] .truncate) {
  overflow: visible;
  text-overflow: clip;
  white-space: normal;
}
@container permissions (min-width:56rem) {
  .permission-default-grid {
    grid-template-columns: repeat(3, minmax(0, 1fr));
  }
  .permission-domain + .permission-domain {
    @apply border-l border-border/60;
  }
  .permission-secondary-grid {
    grid-template-columns: minmax(0, 1.2fr) minmax(0, 1fr);
  }
  .permission-mobile-summary {
    display: none;
  }
}
@media (prefers-reduced-motion: reduce) {
  .permission-studio :deep(*) {
    animation: none;
    transition: none;
  }
}
</style>
