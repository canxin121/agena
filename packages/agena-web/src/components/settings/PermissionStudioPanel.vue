<script setup lang="ts">
import { computed, defineAsyncComponent, onBeforeUnmount, onMounted, ref, watch, type Ref } from 'vue'
import { onBeforeRouteLeave, onBeforeRouteUpdate } from 'vue-router'
import {
  RiAddLine,
  RiArrowDownSLine,
  RiCheckLine,
  RiCodeSSlashLine,
  RiDeleteBinLine,
  RiFolderLine,
  RiGlobalLine,
  RiLockLine,
  RiRefreshLine,
  RiResetLeftLine,
  RiSave3Line,
  RiShieldCheckLine,
  RiTerminalBoxLine,
  RiToolsLine,
} from '@remixicon/vue'

import Button from '@/components/ui/Button.vue'
import { findSettingsScope } from '@/components/settings/settingsScopeCatalog'
import IconButton from '@/components/ui/IconButton.vue'
import Input from '@/components/ui/Input.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import PermissionSettingsCard from './permissions/PermissionSettingsCard.vue'
import PermissionRuleCard from './permissions/PermissionRuleCard.vue'
import { apiJson } from '@/lib/api'
import { confirmAction } from '@/lib/appConfirm'
import { deleteRuntimeSetting, setRuntimeSetting } from '@/lib/runtimeSettings'
import { useChatStore } from '@/stores/chat'
import { useDirectoryStore } from '@/stores/directory'
import { useToastsStore } from '@/stores/toasts'
import type { JsonValue } from '@/types/json'
import { settingsText as st } from '@/i18n/settingsText'

type PermissionMode = 'allow' | 'auto' | 'ask' | 'deny'
type PermissionSource = 'global' | 'workspace' | 'session' | 'effective'
type PermissionSection = 'path' | 'network' | 'tools'
type AccessModes = { read?: PermissionMode; write?: PermissionMode }
type PermissionConfig = {
  path?: {
    workspace?: AccessModes
    external?: AccessModes
    rules?: Record<string, AccessModes>
  }
  network?: {
    internet?: PermissionMode
    private?: PermissionMode
    loopback?: PermissionMode
    rules?: Record<string, PermissionMode>
  }
  tools?: {
    default?: PermissionMode
    names?: Record<string, PermissionMode>
    rules?: Record<string, PermissionMode | Record<string, PermissionMode>>
  }
  approval_model?: JsonValue
}

type SettingsResponse = { value?: JsonValue; config_found?: boolean; config_path?: string }
type SessionStateResponse = {
  execution?: {
    selected_permission?: PermissionConfig
    effective_permission?: PermissionConfig
    context?: { selected_permission?: PermissionConfig; effective_permission?: PermissionConfig }
    execution?: { selected_permission?: PermissionConfig; effective_permission?: PermissionConfig }
  }
}
type ToolCatalogResponse = { permission_tools?: Array<{ name?: string; summary?: string }> }

const SHELL_LAUNCH_TOOLS = ['agena.shell.exec', 'agena.shell.spawn', 'agena.shell.watch', 'agena.shell.open'] as const
const SHELL_CAPABLE_TOOLS = [...SHELL_LAUNCH_TOOLS, 'agena.shell.write'] as const

const props = defineProps<{ section: string; scope: string }>()
const chat = useChatStore()
const directory = useDirectoryStore()
const toasts = useToastsStore()
const PermissionsPanel = defineAsyncComponent(() => import('@/components/settings/PermissionsPanel.vue'))

const loading = ref(false)
const saving = ref(false)
const error = ref('')
const saveError = ref('')
const loaded = ref(false)
const savedConfig = ref<PermissionConfig>({})
const rawEditorOpen = ref(false)
const rulesPanel = ref<{ refresh: () => Promise<void> } | null>(null)
const rawPending = computed(() => rawJson.value !== JSON.stringify(config.value, null, 2))
const dirty = computed(() => JSON.stringify(config.value) !== JSON.stringify(savedConfig.value) || rawPending.value)
const editingDisabled = computed(() => !canEdit.value || !loaded.value || loading.value || saving.value)
const formDisabled = computed(() => editingDisabled.value || rawPending.value)
// The workbench owns source selection; this editor only normalizes the layer.
const selectedSource = computed<PermissionSource>(() => normalizePermissionSource(props.scope))

function normalizePermissionSource(value: unknown): PermissionSource {
  return value === 'global' || value === 'workspace' || value === 'session' ? value : 'effective'
}
const selectedSection = computed<PermissionSection>(() => {
  if (props.section === 'network') return 'network'
  if (props.section === 'tools') return 'tools'
  return 'path'
})
const config = ref<PermissionConfig>({})
const rawJson = ref('{}')
const rawJsonError = ref('')
const toolCatalog = ref<Array<{ name: string; summary: string }>>([])
const newPath = ref('')
const newPathReadMode = ref<PermissionMode>('auto')
const newPathWriteMode = ref<PermissionMode>('auto')
const newNetworkRule = ref('')
const newNetworkRuleMode = ref<PermissionMode>('auto')
const newToolName = ref('')
const newToolNameMode = ref<PermissionMode>('ask')
const newCommandTool = ref<string>(SHELL_LAUNCH_TOOLS[0])
const newCommandPattern = ref('')
const newCommandMode = ref<PermissionMode>('ask')
const selectedPathRuleKeys = ref<Set<string>>(new Set())
const selectedNetworkRuleKeys = ref<Set<string>>(new Set())
const selectedToolNameRuleKeys = ref<Set<string>>(new Set())
const selectedCommandRuleKeys = ref<Set<string>>(new Set())
const bulkPathReadMode = ref<PermissionMode | ''>('auto')
const bulkPathWriteMode = ref<PermissionMode | ''>('auto')
const bulkNetworkMode = ref<PermissionMode>('auto')
const bulkToolNameMode = ref<PermissionMode>('auto')
const bulkCommandMode = ref<PermissionMode>('ask')
const pathQuery = ref('')
const networkQuery = ref('')
const toolQuery = ref('')
const commandQuery = ref('')

const activeSessionId = computed(() => {
  const value = Number(chat.selectedSessionId)
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
const shellToolOptions = SHELL_CAPABLE_TOOLS.map((value) => ({ value, label: value }))
const toolOptions = computed(() =>
  toolCatalog.value.map((tool) => ({ value: tool.name, label: tool.name, description: tool.summary })),
)
const pathScopes = computed(() => [
  { id: 'workspace' as const, label: st('Workspace files'), description: st('Files inside the current project.') },
  { id: 'external' as const, label: st('External files'), description: st('Files outside the current project.') },
])
const networkZones = computed(() => [
  { id: 'internet' as const, label: st('Internet'), description: st('Public websites and services.') },
  {
    id: 'private' as const,
    label: st('Private network'),
    description: st('Devices and services on your local network.'),
  },
  {
    id: 'loopback' as const,
    label: st('This device'),
    description: st('Local services on localhost or loopback addresses.'),
  },
])
const commandClasses = computed(() => [
  { id: 'no-op', label: st('No-op commands') },
  { id: 'routine', label: st('Routine commands') },
  { id: 'dangerous', label: st('Dangerous commands') },
])

const pathRules = computed(() => Object.entries(config.value.path?.rules || {}))
const networkRules = computed(() => Object.entries(config.value.network?.rules || {}))
const toolNameRules = computed(() => Object.entries(config.value.tools?.names || {}))
const commandRules = computed(() => {
  const rows: Array<{ tool: string; command: string; mode: PermissionMode }> = []
  for (const [tool, rawRules] of Object.entries(config.value.tools?.rules || {})) {
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
      if (tool === '*' && command !== 'read-only') continue
      if (isMode(mode)) rows.push({ tool, command, mode })
    }
  }
  return rows
})
const customCommandRules = computed(() =>
  commandRules.value.filter((row) => row.tool !== '*' && !isCommandClass(row.tool, row.command)),
)
const visiblePathRules = computed(() => pathRules.value.filter(([path]) => matchesQuery(path, pathQuery.value)))
const visibleNetworkRules = computed(() =>
  networkRules.value.filter(([target]) => matchesQuery(target, networkQuery.value)),
)
const visibleToolNameRules = computed(() => toolNameRules.value.filter(([name]) => matchesQuery(name, toolQuery.value)))
const visibleCommandRules = computed(() =>
  customCommandRules.value.filter((row) => matchesQuery(`${row.tool} ${row.command}`, commandQuery.value)),
)

function matchesQuery(value: string, query: string) {
  return value.toLowerCase().includes(query.trim().toLowerCase())
}
function isCommandClass(tool: string, command: string) {
  return tool === 'agena.shell.write' ? command === 'dangerous' : ['no-op', 'routine', 'dangerous'].includes(command)
}
function classMode(tool: string, command: string): PermissionMode | '' {
  const rules = config.value.tools?.rules?.[tool]
  return rules && typeof rules === 'object' && isMode(rules[command]) ? rules[command] : ''
}
const selectedPathRules = computed(() => pathRules.value.filter(([path]) => selectedPathRuleKeys.value.has(path)))
const selectedNetworkRules = computed(() =>
  networkRules.value.filter(([target]) => selectedNetworkRuleKeys.value.has(target)),
)
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
const allNetworkRulesSelected = computed(() =>
  areAllSelected(
    visibleNetworkRules.value.map(([target]) => target),
    selectedNetworkRuleKeys.value,
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
function toggleNetworkRule(target: string, event: Event) {
  toggleSelection(selectedNetworkRuleKeys, target, event)
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
function toggleAllNetworkRules() {
  toggleAllSelection(
    selectedNetworkRuleKeys,
    visibleNetworkRules.value.map(([target]) => target),
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
function clearNetworkRuleSelection() {
  clearSelection(selectedNetworkRuleKeys)
}
function clearToolNameRuleSelection() {
  clearSelection(selectedToolNameRuleKeys)
}
function clearCommandRuleSelection() {
  clearSelection(selectedCommandRuleKeys)
}
function applySelectedPathReadMode() {
  setSelectedPathMode('read', bulkPathReadMode.value)
}
function applySelectedPathWriteMode() {
  setSelectedPathMode('write', bulkPathWriteMode.value)
}
function clearAllRuleSelections() {
  clearSelection(selectedPathRuleKeys)
  clearSelection(selectedNetworkRuleKeys)
  clearSelection(selectedToolNameRuleKeys)
  clearSelection(selectedCommandRuleKeys)
}

watch(
  [visiblePathRules, visibleNetworkRules, visibleToolNameRules, visibleCommandRules],
  () => {
    pruneSelection(
      selectedPathRuleKeys,
      visiblePathRules.value.map(([path]) => path),
    )
    pruneSelection(
      selectedNetworkRuleKeys,
      visibleNetworkRules.value.map(([target]) => target),
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

// Class policies use the same ordered rule map as custom command patterns.
// An absent entry inherits the lower layer; it must not be presented as Allow.
function writeRule(tool: string, command: string, value: string) {
  mutate((next) => {
    setCommandRuleInConfig(next, tool, command, value)
    if (isEmptyToolSection(next.tools)) delete next.tools
  })
}

function isMode(value: unknown): value is PermissionMode {
  return value === 'allow' || value === 'auto' || value === 'ask' || value === 'deny'
}

function isShellCapableTool(value: string): boolean {
  return (SHELL_CAPABLE_TOOLS as readonly string[]).includes(value)
}

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T
}

function asConfig(value: unknown): PermissionConfig {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return {}
  return clone(value as PermissionConfig)
}

function syncRaw() {
  rawJson.value = JSON.stringify(config.value, null, 2)
  rawJsonError.value = ''
}

function mutate(mutator: (next: PermissionConfig) => void) {
  if (editingDisabled.value) return
  if (rawPending.value && !applyRawJson()) return
  const next = clone(config.value)
  mutator(next)
  config.value = next
  syncRaw()
}

function modeAt(scope: 'workspace' | 'external', access: 'read' | 'write'): PermissionMode | '' {
  return config.value.path?.[scope]?.[access] || ''
}

function setPathDefault(scope: 'workspace' | 'external', access: 'read' | 'write', value: string) {
  mutate((next) => {
    next.path ||= {}
    next.path[scope] ||= {}
    if (isMode(value)) next.path[scope]![access] = value
    else delete next.path[scope]![access]
    if (!next.path[scope]?.read && !next.path[scope]?.write) delete next.path[scope]
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

// A path rule is always the explicit read/write object form; there is no
// shorthand string form, on the wire or in the editor.
function pathRuleMode(rule: AccessModes, access: 'read' | 'write'): PermissionMode | '' {
  return rule?.[access] || ''
}

function setPathRuleMode(path: string, access: 'read' | 'write', value: string) {
  mutate((next) => {
    next.path ||= {}
    next.path.rules ||= {}
    const modes: AccessModes = { ...(next.path.rules[path] || {}) }
    if (isMode(value)) modes[access] = value
    else delete modes[access]
    if (modes.read || modes.write) next.path.rules[path] = modes
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

function setSelectedPathMode(access: 'read' | 'write', value: string) {
  const paths = selectedPathRules.value.map(([path]) => path)
  if (!paths.length) return
  mutate((next) => {
    for (const path of paths) {
      const rule = next.path?.rules?.[path]
      if (!rule) continue
      if (isMode(value)) rule[access] = value
      else delete rule[access]
      if (!rule.read && !rule.write) delete next.path!.rules![path]
    }
    if (isEmptyPathSection(next.path)) delete next.path
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
  const nextPath = input.value.trim()
  if (!nextPath || nextPath === path) {
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
  error.value = ''
  input.value = nextPath
}

function networkMode(zone: 'internet' | 'private' | 'loopback'): PermissionMode | '' {
  return config.value.network?.[zone] || ''
}

function setNetworkMode(zone: 'internet' | 'private' | 'loopback', value: string) {
  mutate((next) => {
    next.network ||= {}
    if (isMode(value)) next.network[zone] = value
    else delete next.network[zone]
    if (
      !next.network.internet &&
      !next.network.private &&
      !next.network.loopback &&
      !Object.keys(next.network.rules || {}).length
    )
      delete next.network
  })
}

function setNetworkRule(target: string, value: string) {
  mutate((next) => {
    next.network ||= {}
    next.network.rules ||= {}
    if (isMode(value)) next.network.rules[target] = value
  })
}

function removeNetworkRule(target: string) {
  mutate((next) => {
    if (next.network?.rules) delete next.network.rules[target]
    if (
      next.network &&
      !next.network.internet &&
      !next.network.private &&
      !next.network.loopback &&
      !Object.keys(next.network.rules || {}).length
    ) {
      delete next.network
    }
  })
}

function setSelectedNetworkMode(value: string) {
  const targets = selectedNetworkRules.value.map(([target]) => target)
  if (!targets.length || !isMode(value)) return
  mutate((next) => {
    next.network ||= {}
    next.network.rules ||= {}
    for (const target of targets) next.network.rules[target] = value
  })
}

function removeSelectedNetworkRules() {
  const targets = selectedNetworkRules.value.map(([target]) => target)
  if (!targets.length) return
  mutate((next) => {
    for (const target of targets) {
      if (next.network?.rules) delete next.network.rules[target]
    }
    if (
      next.network &&
      !next.network.internet &&
      !next.network.private &&
      !next.network.loopback &&
      !Object.keys(next.network.rules || {}).length
    ) {
      delete next.network
    }
  })
  clearSelection(selectedNetworkRuleKeys)
}

function renameNetworkRule(target: string, event: Event) {
  const input = event.target as HTMLInputElement
  const nextTarget = input.value.trim()
  if (!nextTarget || nextTarget === target) {
    input.value = target
    return
  }
  if (Object.prototype.hasOwnProperty.call(config.value.network?.rules || {}, nextTarget)) {
    error.value = st('Network rule already exists: {nextTarget}', { nextTarget: nextTarget })
    input.value = target
    return
  }
  mutate((next) => {
    const rules = next.network?.rules
    if (!rules || !Object.prototype.hasOwnProperty.call(rules, target)) return
    const value = rules[target]
    delete rules[target]
    rules[nextTarget] = value
  })
  error.value = ''
  input.value = nextTarget
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
    if (isMode(value)) next.tools.names[name] = value
    else delete next.tools.names[name]
  })
}

function removeToolName(name: string) {
  mutate((next) => {
    if (next.tools?.names) delete next.tools.names[name]
    if (isEmptyToolSection(next.tools)) delete next.tools
  })
}

function setSelectedToolNameMode(value: string) {
  const names = selectedToolNameRules.value.map(([name]) => name)
  if (!names.length || !isMode(value)) return
  mutate((next) => {
    next.tools ||= {}
    next.tools.names ||= {}
    for (const name of names) next.tools.names[name] = value
  })
}

function removeSelectedToolNameRules() {
  const names = selectedToolNameRules.value.map(([name]) => name)
  if (!names.length) return
  mutate((next) => {
    for (const name of names) {
      if (next.tools?.names) delete next.tools.names[name]
    }
    if (isEmptyToolSection(next.tools)) delete next.tools
  })
  clearSelection(selectedToolNameRuleKeys)
}

function renameToolName(name: string, event: Event) {
  const input = event.target as HTMLInputElement
  const nextName = input.value.trim()
  if (!nextName || nextName === name) {
    input.value = name
    return
  }
  if (Object.prototype.hasOwnProperty.call(config.value.tools?.names || {}, nextName)) {
    error.value = st('Tool name rule already exists: {nextName}', { nextName: nextName })
    input.value = name
    return
  }
  mutate((next) => {
    const names = next.tools?.names
    if (!names || !Object.prototype.hasOwnProperty.call(names, name)) return
    const value = names[name]
    delete names[name]
    names[nextName] = value
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
  next.tools ||= {}
  next.tools.rules ||= {}
  const current = next.tools.rules[tool]
  const rules: Record<string, PermissionMode> = isMode(current)
    ? { '*': current }
    : current && typeof current === 'object' && !Array.isArray(current)
      ? ({ ...current } as Record<string, PermissionMode>)
      : {}
  if (isMode(value)) rules[command] = value
  else delete rules[command]
  if (Object.keys(rules).length) next.tools.rules[tool] = rules
  else delete next.tools.rules[tool]
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
  const existing = config.value.tools?.rules?.[tool]
  const rules = isMode(existing) ? { '*': existing } : existing
  if (rules && typeof rules === 'object' && Object.prototype.hasOwnProperty.call(rules, nextCommand)) {
    error.value = st('Command rule already exists for {tool}: {nextCommand}', { tool: tool, nextCommand: nextCommand })
    input.value = command
    return
  }
  mutate((next) => {
    const raw = next.tools?.rules?.[tool]
    const nextRules: Record<string, PermissionMode> = isMode(raw)
      ? { '*': raw }
      : raw && typeof raw === 'object' && !Array.isArray(raw)
        ? ({ ...raw } as Record<string, PermissionMode>)
        : {}
    const value = nextRules[command]
    if (!value) return
    delete nextRules[command]
    nextRules[nextCommand] = value
    next.tools!.rules![tool] = nextRules
  })
  error.value = ''
  input.value = nextCommand
}

function addPathRule() {
  const path = newPath.value.trim()
  if (!path) return
  if (Object.prototype.hasOwnProperty.call(config.value.path?.rules || {}, path)) {
    error.value = st('Path rule already exists: {path}', { path: path })
    return
  }
  mutate((next) => {
    next.path ||= {}
    next.path.rules ||= {}
    next.path.rules[path] = { read: newPathReadMode.value, write: newPathWriteMode.value }
  })
  error.value = ''
  newPath.value = ''
}

function addNetworkRule() {
  const target = newNetworkRule.value.trim()
  if (!target) return
  if (Object.prototype.hasOwnProperty.call(config.value.network?.rules || {}, target)) {
    error.value = st('Network rule already exists: {target}', { target: target })
    return
  }
  mutate((next) => {
    next.network ||= {}
    next.network.rules ||= {}
    next.network.rules[target] = newNetworkRuleMode.value
  })
  error.value = ''
  newNetworkRule.value = ''
}

function addToolName() {
  const name = newToolName.value.trim()
  if (!name) return
  if (Object.prototype.hasOwnProperty.call(config.value.tools?.names || {}, name)) {
    error.value = st('Tool name rule already exists: {name}', { name: name })
    return
  }
  mutate((next) => {
    next.tools ||= {}
    next.tools.names ||= {}
    next.tools.names[name] = newToolNameMode.value
  })
  error.value = ''
  newToolName.value = ''
}

function addCommandRule() {
  const tool = newCommandTool.value.trim()
  const command = newCommandPattern.value.trim()
  if (!isShellCapableTool(tool) || !command) return
  const existing = config.value.tools?.rules?.[tool]
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
  if (editingDisabled.value) return false
  try {
    const parsed = JSON.parse(rawJson.value) as JsonValue
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed))
      throw new Error(st('Permission config must be a JSON object.'))
    config.value = asConfig(parsed)
    syncRaw()
    return true
  } catch (reason) {
    rawJsonError.value = reason instanceof Error ? reason.message : String(reason)
    rawEditorOpen.value = true
    return false
  }
}

const contextKey = computed(() =>
  JSON.stringify([selectedSource.value, activeSessionId.value, directory.currentDirectory]),
)
type PermissionDraft = { config: PermissionConfig; saved: PermissionConfig; raw: string }
const drafts = new Map<string, PermissionDraft>()
let loadedContext = ''
let loadGeneration = 0
let loadController: AbortController | undefined

function rememberDraft() {
  if (!loadedContext || !loaded.value) return
  if (dirty.value)
    drafts.set(loadedContext, { config: clone(config.value), saved: clone(savedConfig.value), raw: rawJson.value })
  else drafts.delete(loadedContext)
}

async function readPolicy(source: PermissionSource, sessionId: number | null, signal: AbortSignal) {
  if (source === 'global' || source === 'workspace') {
    const response = await apiJson<SettingsResponse>(`/api/v1/settings/layers/${source}?path=permission`, { signal })
    return asConfig(response.value)
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
    const sessionPolicy = asConfig(value)
    if (source === 'session' || Object.keys(sessionPolicy).length) return sessionPolicy
  }
  if (source === 'session') return {}
  const response = await apiJson<SettingsResponse>('/api/v1/settings?source=effective&path=permission', { signal })
  return asConfig(response.value)
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
    config.value = clone(draft?.config || {})
    savedConfig.value = clone(draft?.saved || {})
    syncRaw()
    if (draft) rawJson.value = draft.raw
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
    // A refresh updates server data only when the editor has no pending work.
    if (!dirty.value || !loaded.value) {
      config.value = clone(policy.value)
      savedConfig.value = clone(policy.value)
      syncRaw()
    }
    loaded.value = true
  } catch (reason) {
    if (generation === loadGeneration && key === contextKey.value && !controller.signal.aborted)
      error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    if (generation === loadGeneration) loading.value = false
  }
}

function discard() {
  if (saving.value) return
  config.value = clone(savedConfig.value)
  syncRaw()
  error.value = ''
  saveError.value = ''
  drafts.delete(loadedContext)
  clearAllRuleSelections()
}

async function save() {
  if (editingDisabled.value || !dirty.value) return
  if (rawPending.value && !applyRawJson()) return
  const key = contextKey.value
  const source = selectedSource.value
  const sessionId = activeSessionId.value
  const snapshot = clone(config.value)
  const scopeLabel = findSettingsScope('permissions', props.section, selectedSource.value)?.label || st('Permission')
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
    } else if (!Object.keys(snapshot).length) {
      await deleteRuntimeSetting('permission', { reload: true }, source as 'global' | 'workspace')
    } else {
      await setRuntimeSetting('permission', snapshot as JsonValue, { reload: true }, source as 'global' | 'workspace')
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
  config.value = {}
  syncRaw()
  error.value = ''
  clearAllRuleSelections()
}

async function canLeave() {
  if (saving.value) return false
  if (!dirty.value && !drafts.size) return true
  return confirmAction({
    title: st('Discard unsaved permission changes?'),
    description: st('Your permission changes have not been saved.'),
    confirmText: st('Discard changes'),
    cancelText: st('Keep editing'),
  })
}
onBeforeRouteUpdate(async (to) => {
  if (
    to.params.section === 'permissions' &&
    normalizePermissionSource(to.query.scope || selectedSource.value) === selectedSource.value
  )
    return true
  if (!(await canLeave())) return false
  discard()
  drafts.clear()
  return true
})
onBeforeRouteLeave(canLeave)
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
  <section class="permission-studio grid min-w-0 gap-4" :aria-busy="loading || saving">
    <div class="flex min-w-0 flex-wrap items-center justify-between gap-2">
      <div class="flex min-w-0 items-center gap-2 text-xs text-muted-foreground" role="status" aria-live="polite">
        <RiLockLine v-if="selectedSource === 'effective'" class="h-4 w-4 shrink-0" />
        <RiRefreshLine v-else-if="loading || saving" class="h-4 w-4 shrink-0 animate-spin" />
        <span v-else-if="dirty" class="h-1.5 w-1.5 shrink-0 rounded-full bg-primary" />
        <RiCheckLine v-else-if="loaded && canEdit" class="h-4 w-4 shrink-0" />
        <span v-if="selectedSource === 'effective'">{{ $st('Read-only merged policy.') }}</span>
        <span v-else-if="saving">{{ $st('Saving…') }}</span>
        <span v-else-if="loading">{{ $st('Loading…') }}</span>
        <span v-else-if="loaded && canEdit">{{ dirty ? $st('Unsaved changes') : $st('All changes saved.') }}</span>
      </div>
      <div class="flex items-center gap-1">
        <Button
          v-if="canEdit"
          variant="ghost"
          size="sm"
          :disabled="editingDisabled || !Object.keys(config).length"
          @click="clearSource"
        >
          <RiResetLeftLine class="mr-1.5 h-3.5 w-3.5" />{{ $st('Reset scope') }}
        </Button>
        <IconButton
          variant="ghost"
          size="sm"
          :tooltip="$st('Refresh permissions')"
          :aria-label="$st('Refresh permissions')"
          :disabled="loading || saving"
          @click="refresh"
        >
          <RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''" />
        </IconButton>
      </div>
    </div>
    <div
      v-if="error"
      role="alert"
      class="rounded-lg border border-destructive/30 bg-destructive/5 px-4 py-3 text-sm text-destructive"
    >
      {{ error }}
    </div>
    <div
      v-if="selectedSource === 'session' && !activeSessionId"
      class="flex items-start gap-3 rounded-xl border border-border/60 bg-muted/20 p-4 text-sm text-muted-foreground"
    >
      <RiShieldCheckLine class="h-5 w-5 shrink-0" />{{ $st('Open a session to edit current-session permission.') }}
    </div>
    <div v-else-if="!loaded && loading" class="grid gap-4" aria-hidden="true">
      <div v-for="n in 2" :key="n" class="h-40 animate-pulse rounded-xl border border-border/60 bg-muted/20" />
    </div>
    <template v-else-if="loaded">
      <template v-if="selectedSection === 'path'">
        <PermissionSettingsCard
          :title="$st('Path Defaults')"
          :description="$st('Set separate read and write decisions for workspace and external paths.')"
          :icon="RiFolderLine"
        >
          <div class="policy-grid">
            <div v-for="scope in pathScopes" :key="scope.id" class="policy-item">
              <div>
                <h3 class="text-sm font-medium">{{ scope.label }}</h3>
                <p class="mt-1 text-xs leading-relaxed text-muted-foreground">{{ scope.description }}</p>
              </div>
              <div class="grid grid-cols-2 gap-3">
                <label class="permission-control"
                  ><span>{{ $st('Read') }}</span
                  ><OptionPicker
                    :model-value="modeAt(scope.id, 'read')"
                    :options="modeOptions"
                    :empty-label="$st('Inherit')"
                    :title="$st('Path read mode')"
                    :disabled="formDisabled"
                    @update:model-value="setPathDefault(scope.id, 'read', $event)"
                /></label>
                <label class="permission-control"
                  ><span>{{ $st('Write') }}</span
                  ><OptionPicker
                    :model-value="modeAt(scope.id, 'write')"
                    :options="modeOptions"
                    :empty-label="$st('Inherit')"
                    :title="$st('Path write mode')"
                    :disabled="formDisabled"
                    @update:model-value="setPathDefault(scope.id, 'write', $event)"
                /></label>
              </div>
            </div>
          </div>
        </PermissionSettingsCard>
        <PermissionRuleCard
          :title="$st('Path Rules')"
          :description="$st('Override access for matching paths. The last matching rule wins.')"
          :icon="RiFolderLine"
          :count="pathRules.length"
          :visible-count="visiblePathRules.length"
          v-model:query="pathQuery"
          :selected-count="selectedPathRules.length"
          :all-selected="allPathRulesSelected"
          :editable="canEdit"
          :disabled="formDisabled"
          @toggle-all="toggleAllPathRules"
          @clear-selection="clearPathRuleSelection"
        >
          <template #bulk>
            <div class="flex flex-wrap items-center gap-1.5">
              <span class="text-xs text-muted-foreground">{{ $st('Read') }}</span>
              <div class="w-28">
                <OptionPicker
                  v-model="bulkPathReadMode"
                  :options="modeOptions"
                  :empty-label="$st('Inherit')"
                  :title="$st('Path rule read mode')"
                  size="sm"
                  :disabled="formDisabled"
                />
              </div>
              <Button variant="outline" size="sm" :disabled="formDisabled" @click="applySelectedPathReadMode">{{
                $st('Apply')
              }}</Button>
            </div>
            <div class="flex flex-wrap items-center gap-1.5">
              <span class="text-xs text-muted-foreground">{{ $st('Write') }}</span>
              <div class="w-28">
                <OptionPicker
                  v-model="bulkPathWriteMode"
                  :options="modeOptions"
                  :empty-label="$st('Inherit')"
                  :title="$st('Path rule write mode')"
                  size="sm"
                  :disabled="formDisabled"
                />
              </div>
              <Button variant="outline" size="sm" :disabled="formDisabled" @click="applySelectedPathWriteMode">{{
                $st('Apply')
              }}</Button>
            </div>
            <Button
              variant="ghost"
              size="sm"
              class="text-destructive"
              :disabled="formDisabled"
              @click="removeSelectedPathRules"
              >{{ $st('Remove selected') }}</Button
            >
          </template>
          <div class="permission-rule-labels" aria-hidden="true">
            <span class="rule-target">{{ $st('Path') }}</span>
            <div class="rule-modes">
              <span>{{ $st('Read') }}</span
              ><span>{{ $st('Write') }}</span>
            </div>
          </div>
          <fieldset
            v-for="[path, rule] in visiblePathRules"
            :key="path"
            :disabled="formDisabled"
            class="permission-rule"
          >
            <label v-if="canEdit" class="rule-select"
              ><input
                type="checkbox"
                class="accent-primary"
                :checked="selectedPathRuleKeys.has(path)"
                @change="togglePathRule(path, $event)"
              /><span class="sr-only">{{ path }}</span></label
            >
            <div class="rule-target">
              <input
                :value="path"
                type="text"
                class="permission-target-input"
                :aria-label="$st('Rename path rule {path}', { path })"
                :title="$st('Rename path rule {path}', { path })"
                :readonly="formDisabled"
                @change="renamePathRule(path, $event)"
              />
            </div>
            <div class="rule-modes">
              <label class="permission-control"
                ><span>{{ $st('Read') }}</span
                ><OptionPicker
                  :model-value="pathRuleMode(rule, 'read')"
                  :options="modeOptions"
                  :empty-label="$st('Inherit')"
                  :title="$st('Path rule read mode')"
                  :disabled="formDisabled"
                  @update:model-value="setPathRuleMode(path, 'read', $event)"
              /></label>
              <label class="permission-control"
                ><span>{{ $st('Write') }}</span
                ><OptionPicker
                  :model-value="pathRuleMode(rule, 'write')"
                  :options="modeOptions"
                  :empty-label="$st('Inherit')"
                  :title="$st('Path rule write mode')"
                  :disabled="formDisabled"
                  @update:model-value="setPathRuleMode(path, 'write', $event)"
              /></label>
            </div>
            <div v-if="canEdit" class="rule-remove">
              <IconButton
                class="text-muted-foreground hover:text-destructive"
                variant="ghost"
                size="sm"
                :tooltip="$st('Remove path rule')"
                :aria-label="$st('Remove path rule')"
                @click="removePathRule(path)"
                ><RiDeleteBinLine class="h-4 w-4"
              /></IconButton>
            </div>
          </fieldset>
          <template #create>
            <form class="rule-create" @submit.prevent="addPathRule">
              <label class="permission-control"
                ><span>{{ $st('Path') }}</span
                ><Input
                  v-model="newPath"
                  class="font-mono text-xs"
                  placeholder="/path or relative/path"
                  :aria-label="$st('Path')"
              /></label>
              <div class="grid grid-cols-2 gap-2">
                <label class="permission-control"
                  ><span>{{ $st('Read') }}</span
                  ><OptionPicker
                    v-model="newPathReadMode"
                    :options="modeOptions"
                    :include-empty="false"
                    :title="$st('New path read mode')"
                    :disabled="formDisabled" /></label
                ><label class="permission-control"
                  ><span>{{ $st('Write') }}</span
                  ><OptionPicker
                    v-model="newPathWriteMode"
                    :options="modeOptions"
                    :include-empty="false"
                    :title="$st('New path write mode')"
                    :disabled="formDisabled"
                /></label>
              </div>
              <Button type="submit" variant="outline" size="sm" class="h-9" :disabled="formDisabled || !newPath.trim()"
                ><RiAddLine class="mr-1.5 h-4 w-4" />{{ $st('Add path rule') }}</Button
              >
            </form>
          </template>
        </PermissionRuleCard>
      </template>
      <template v-else-if="selectedSection === 'network'">
        <PermissionSettingsCard
          :title="$st('Network Zones')"
          :description="$st('Control internet, private-network, and loopback access, then add domain/host rules.')"
          :icon="RiGlobalLine"
        >
          <div class="policy-grid network-policies">
            <div v-for="zone in networkZones" :key="zone.id" class="policy-item">
              <div>
                <h3 class="text-sm font-medium">{{ zone.label }}</h3>
                <p class="mt-1 text-xs leading-relaxed text-muted-foreground">{{ zone.description }}</p>
              </div>
              <OptionPicker
                :model-value="networkMode(zone.id)"
                :options="modeOptions"
                :empty-label="$st('Inherit')"
                :title="zone.label"
                :disabled="formDisabled"
                @update:model-value="setNetworkMode(zone.id, $event)"
              />
            </div>
          </div>
        </PermissionSettingsCard>
        <PermissionRuleCard
          :title="$st('Domain Rules')"
          :description="$st('Set exceptions for a domain, IP address, or host and port.')"
          :icon="RiGlobalLine"
          :count="networkRules.length"
          :visible-count="visibleNetworkRules.length"
          v-model:query="networkQuery"
          :selected-count="selectedNetworkRules.length"
          :all-selected="allNetworkRulesSelected"
          :editable="canEdit"
          :disabled="formDisabled"
          @toggle-all="toggleAllNetworkRules"
          @clear-selection="clearNetworkRuleSelection"
        >
          <template #bulk>
            <div class="w-32">
              <OptionPicker
                v-model="bulkNetworkMode"
                :options="modeOptions"
                :include-empty="false"
                :title="$st('Network rule mode')"
                size="sm"
                :disabled="formDisabled"
              />
            </div>
            <Button
              variant="outline"
              size="sm"
              :disabled="formDisabled"
              @click="setSelectedNetworkMode(bulkNetworkMode)"
              >{{ $st('Apply') }}</Button
            >
            <Button
              variant="ghost"
              size="sm"
              class="text-destructive"
              :disabled="formDisabled"
              @click="removeSelectedNetworkRules"
              >{{ $st('Remove selected') }}</Button
            >
          </template>
          <div class="permission-rule-labels" aria-hidden="true">
            <span class="rule-target">{{ $st('Domain or host') }}</span>
            <div class="rule-modes" data-single-mode>
              <span>{{ $st('Mode') }}</span>
            </div>
          </div>
          <fieldset
            v-for="[target, mode] in visibleNetworkRules"
            :key="target"
            :disabled="formDisabled"
            class="permission-rule"
          >
            <label v-if="canEdit" class="rule-select"
              ><input
                type="checkbox"
                class="accent-primary"
                :checked="selectedNetworkRuleKeys.has(target)"
                @change="toggleNetworkRule(target, $event)"
              /><span class="sr-only">{{ target }}</span></label
            >
            <div class="rule-target">
              <input
                :value="target"
                type="text"
                class="permission-target-input"
                :aria-label="$st('Rename network rule {target}', { target })"
                :title="$st('Rename network rule {target}', { target })"
                :readonly="formDisabled"
                @change="renameNetworkRule(target, $event)"
              />
            </div>
            <div class="rule-modes" data-single-mode>
              <label class="permission-control"
                ><span>{{ $st('Mode') }}</span
                ><OptionPicker
                  :model-value="mode"
                  :options="modeOptions"
                  :include-empty="false"
                  :title="$st('Network rule mode')"
                  :disabled="formDisabled"
                  @update:model-value="setNetworkRule(target, $event)"
              /></label>
            </div>
            <div v-if="canEdit" class="rule-remove">
              <IconButton
                class="text-muted-foreground hover:text-destructive"
                variant="ghost"
                size="sm"
                :tooltip="$st('Remove network rule')"
                :aria-label="$st('Remove network rule')"
                @click="removeNetworkRule(target)"
                ><RiDeleteBinLine class="h-4 w-4"
              /></IconButton>
            </div>
          </fieldset>
          <template #create>
            <form class="rule-create" @submit.prevent="addNetworkRule">
              <label class="permission-control"
                ><span>{{ $st('Domain or host') }}</span
                ><Input
                  v-model="newNetworkRule"
                  class="font-mono text-xs"
                  placeholder="example.com or 127.0.0.1:8080"
                  :aria-label="$st('Domain or host')"
              /></label>
              <label class="permission-control"
                ><span>{{ $st('Mode') }}</span
                ><OptionPicker
                  v-model="newNetworkRuleMode"
                  :options="modeOptions"
                  :include-empty="false"
                  :title="$st('Network rule mode')"
                  :disabled="formDisabled"
              /></label>
              <Button
                type="submit"
                variant="outline"
                size="sm"
                class="h-9"
                :disabled="formDisabled || !newNetworkRule.trim()"
                ><RiAddLine class="mr-1.5 h-4 w-4" />{{ $st('Add domain rule') }}</Button
              >
            </form>
          </template>
        </PermissionRuleCard>
      </template>
      <template v-else>
        <PermissionSettingsCard
          :title="$st('Tool Defaults')"
          :description="$st('Set the fallback policy and access for tools that only read data.')"
          :icon="RiShieldCheckLine"
        >
          <div class="policy-grid">
            <div class="policy-item">
              <div>
                <h3 class="text-sm font-medium">{{ $st('Default tool mode') }}</h3>
                <p class="mt-1 text-xs leading-relaxed text-muted-foreground">
                  {{ $st('Used when no tool-specific rule matches.') }}
                </p>
              </div>
              <OptionPicker
                :model-value="config.tools?.default || ''"
                :options="modeOptions"
                :empty-label="$st('Inherit')"
                :title="$st('Default tool mode')"
                :disabled="formDisabled"
                @update:model-value="setToolDefault"
              />
            </div>
            <div class="policy-item">
              <div>
                <h3 class="text-sm font-medium">{{ $st('Read-only tools') }}</h3>
                <p class="mt-1 text-xs leading-relaxed text-muted-foreground">
                  {{ $st('Tools whose permission contract is read-only.') }}
                </p>
              </div>
              <OptionPicker
                :model-value="classMode('*', 'read-only')"
                :options="modeOptions"
                :empty-label="$st('Inherit')"
                :title="$st('Read-only tools')"
                :disabled="formDisabled"
                @update:model-value="writeRule('*', 'read-only', $event)"
              />
            </div>
          </div>
        </PermissionSettingsCard>
        <PermissionRuleCard
          :title="$st('Name Rules')"
          :description="$st('Choose a tool to override its default access policy.')"
          :icon="RiToolsLine"
          :count="toolNameRules.length"
          :visible-count="visibleToolNameRules.length"
          v-model:query="toolQuery"
          :selected-count="selectedToolNameRules.length"
          :all-selected="allToolNameRulesSelected"
          :editable="canEdit"
          :disabled="formDisabled"
          @toggle-all="toggleAllToolNameRules"
          @clear-selection="clearToolNameRuleSelection"
        >
          <template #bulk>
            <div class="w-32">
              <OptionPicker
                v-model="bulkToolNameMode"
                :options="modeOptions"
                :include-empty="false"
                :title="$st('Tool name mode')"
                size="sm"
                :disabled="formDisabled"
              />
            </div>
            <Button
              variant="outline"
              size="sm"
              :disabled="formDisabled"
              @click="setSelectedToolNameMode(bulkToolNameMode)"
              >{{ $st('Apply') }}</Button
            >
            <Button
              variant="ghost"
              size="sm"
              class="text-destructive"
              :disabled="formDisabled"
              @click="removeSelectedToolNameRules"
              >{{ $st('Remove selected') }}</Button
            >
          </template>
          <div class="permission-rule-labels" aria-hidden="true">
            <span class="rule-target">{{ $st('Tool name') }}</span>
            <div class="rule-modes" data-single-mode>
              <span>{{ $st('Mode') }}</span>
            </div>
          </div>
          <fieldset
            v-for="[name, mode] in visibleToolNameRules"
            :key="name"
            :disabled="formDisabled"
            class="permission-rule"
          >
            <label v-if="canEdit" class="rule-select"
              ><input
                type="checkbox"
                class="accent-primary"
                :checked="selectedToolNameRuleKeys.has(name)"
                @change="toggleToolNameRule(name, $event)"
              /><span class="sr-only">{{ name }}</span></label
            >
            <div class="rule-target">
              <input
                :value="name"
                type="text"
                class="permission-target-input"
                :aria-label="$st('Rename tool rule {name}', { name })"
                :title="$st('Rename tool rule {name}', { name })"
                :readonly="formDisabled"
                @change="renameToolName(name, $event)"
              />
            </div>
            <div class="rule-modes" data-single-mode>
              <label class="permission-control"
                ><span>{{ $st('Mode') }}</span
                ><OptionPicker
                  :model-value="mode"
                  :options="modeOptions"
                  :include-empty="false"
                  :title="$st('Tool name mode')"
                  :disabled="formDisabled"
                  @update:model-value="setToolNameMode(name, $event)"
              /></label>
            </div>
            <div v-if="canEdit" class="rule-remove">
              <IconButton
                class="text-muted-foreground hover:text-destructive"
                variant="ghost"
                size="sm"
                :tooltip="$st('Remove tool rule')"
                :aria-label="$st('Remove tool rule')"
                @click="removeToolName(name)"
                ><RiDeleteBinLine class="h-4 w-4"
              /></IconButton>
            </div>
          </fieldset>
          <template #create>
            <form class="rule-create" @submit.prevent="addToolName">
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
                  monospace
              /></label>
              <label class="permission-control"
                ><span>{{ $st('Mode') }}</span
                ><OptionPicker
                  v-model="newToolNameMode"
                  :options="modeOptions"
                  :include-empty="false"
                  :title="$st('Tool name mode')"
                  :disabled="formDisabled"
              /></label>
              <Button
                type="submit"
                variant="outline"
                size="sm"
                class="h-9"
                :disabled="formDisabled || !newToolName.trim()"
                ><RiAddLine class="mr-1.5 h-4 w-4" />{{ $st('Add tool name') }}</Button
              >
            </form>
          </template>
        </PermissionRuleCard>
        <PermissionRuleCard
          :title="$st('Command Rules')"
          :description="$st('Match shell commands with patterns such as git push *. The last matching rule wins.')"
          :icon="RiTerminalBoxLine"
          :count="customCommandRules.length"
          :visible-count="visibleCommandRules.length"
          v-model:query="commandQuery"
          :selected-count="selectedCommandRules.length"
          :all-selected="allCommandRulesSelected"
          :editable="canEdit"
          :disabled="formDisabled"
          @toggle-all="toggleAllCommandRules"
          @clear-selection="clearCommandRuleSelection"
        >
          <template #bulk>
            <div class="w-32">
              <OptionPicker
                v-model="bulkCommandMode"
                :options="modeOptions"
                :include-empty="false"
                :title="$st('Command rule mode')"
                size="sm"
                :disabled="formDisabled"
              />
            </div>
            <Button
              variant="outline"
              size="sm"
              :disabled="formDisabled"
              @click="setSelectedCommandMode(bulkCommandMode)"
              >{{ $st('Apply') }}</Button
            >
            <Button
              variant="ghost"
              size="sm"
              class="text-destructive"
              :disabled="formDisabled"
              @click="removeSelectedCommandRules"
              >{{ $st('Remove selected') }}</Button
            >
          </template>
          <div class="permission-rule-labels" aria-hidden="true">
            <span class="rule-target">{{ $st('Command pattern') }}</span>
            <div class="rule-modes" data-single-mode>
              <span>{{ $st('Mode') }}</span>
            </div>
          </div>
          <fieldset
            v-for="row in visibleCommandRules"
            :key="commandRuleKey(row.tool, row.command)"
            :disabled="formDisabled"
            class="permission-rule"
          >
            <label v-if="canEdit" class="rule-select"
              ><input
                type="checkbox"
                class="accent-primary"
                :checked="selectedCommandRuleKeys.has(commandRuleKey(row.tool, row.command))"
                @change="toggleCommandRule(row.tool, row.command, $event)"
              /><span class="sr-only">{{ row.command }}</span></label
            >
            <div class="rule-target">
              <code class="mb-1 block truncate text-[11px] text-muted-foreground" :title="row.tool">{{
                row.tool
              }}</code>
              <input
                :value="row.command"
                type="text"
                class="permission-target-input"
                :aria-label="$st('Rename command rule {command}', { command: row.command })"
                :title="$st('Rename command rule {command}', { command: row.command })"
                :readonly="formDisabled"
                @change="renameCommandRule(row.tool, row.command, $event)"
              />
            </div>
            <div class="rule-modes" data-single-mode>
              <label class="permission-control"
                ><span>{{ $st('Mode') }}</span
                ><OptionPicker
                  :model-value="row.mode"
                  :options="modeOptions"
                  :include-empty="false"
                  :title="$st('Command rule mode')"
                  :disabled="formDisabled"
                  @update:model-value="setCommandMode(row.tool, row.command, $event)"
              /></label>
            </div>
            <div v-if="canEdit" class="rule-remove">
              <IconButton
                class="text-muted-foreground hover:text-destructive"
                variant="ghost"
                size="sm"
                :tooltip="$st('Remove command rule')"
                :aria-label="$st('Remove command rule')"
                @click="removeCommandRule(row.tool, row.command)"
                ><RiDeleteBinLine class="h-4 w-4"
              /></IconButton>
            </div>
          </fieldset>
          <template #create>
            <form class="rule-create" @submit.prevent="addCommandRule">
              <div class="command-inputs">
                <label class="permission-control"
                  ><span>{{ $st('Tool name') }}</span
                  ><OptionPicker
                    v-model="newCommandTool"
                    :options="shellToolOptions"
                    :include-empty="false"
                    :title="$st('Shell-capable tool')"
                    :disabled="formDisabled"
                    monospace
                /></label>
                <label class="permission-control"
                  ><span>{{ $st('Command pattern') }}</span
                  ><Input
                    v-model="newCommandPattern"
                    class="font-mono text-xs"
                    placeholder="git push *"
                    :aria-label="$st('Command pattern')"
                /></label>
              </div>
              <label class="permission-control"
                ><span>{{ $st('Mode') }}</span
                ><OptionPicker
                  v-model="newCommandMode"
                  :options="modeOptions"
                  :include-empty="false"
                  :title="$st('Command rule mode')"
                  :disabled="formDisabled"
              /></label>
              <Button
                type="submit"
                variant="outline"
                size="sm"
                class="h-9"
                :disabled="formDisabled || !newCommandPattern.trim()"
                ><RiAddLine class="mr-1.5 h-4 w-4" />{{ $st('Add command rule') }}</Button
              >
            </form>
          </template>
        </PermissionRuleCard>
        <details class="permission-details group" data-command-classes>
          <summary class="permission-summary">
            <RiTerminalBoxLine class="h-4.5 w-4.5 shrink-0 text-muted-foreground" />
            <div class="min-w-0 flex-1">
              <h2 class="text-[15px] font-semibold">{{ $st('Command categories') }}</h2>
              <p class="mt-1 text-xs leading-relaxed text-muted-foreground">
                {{ $st('Review no-op, routine, and dangerous commands for each shell tool.') }}
              </p>
            </div>
            <RiArrowDownSLine class="h-4 w-4 shrink-0 transition-transform group-open:rotate-180" />
          </summary>
          <div class="grid gap-0 px-3 sm:px-4">
            <div v-for="tool in SHELL_CAPABLE_TOOLS" :key="tool" class="command-class-row">
              <code class="min-w-0 break-all text-xs font-medium">{{ tool }}</code>
              <div class="command-class-modes">
                <label
                  v-for="category in commandClasses.filter(
                    (item) => tool !== 'agena.shell.write' || item.id === 'dangerous',
                  )"
                  :key="category.id"
                  class="permission-control"
                >
                  <span>{{ category.label }}</span>
                  <OptionPicker
                    :model-value="classMode(tool, category.id)"
                    :options="modeOptions"
                    :empty-label="$st('Inherit')"
                    :title="category.label"
                    :disabled="formDisabled"
                    @update:model-value="writeRule(tool, category.id, $event)"
                  />
                </label>
              </div>
            </div>
          </div>
        </details>
      </template>

      <PermissionsPanel v-if="selectedSection === 'tools'" :scope="selectedSource" ref="rulesPanel" />

      <details
        class="permission-details group"
        :open="rawEditorOpen"
        @toggle="rawEditorOpen = ($event.target as HTMLDetailsElement).open"
      >
        <summary class="permission-summary">
          <RiCodeSSlashLine class="h-4.5 w-4.5 shrink-0 text-muted-foreground" />
          <div class="min-w-0 flex-1">
            <h2 class="text-[15px] font-semibold">{{ $st('Advanced permission configuration') }}</h2>
            <p class="mt-1 text-xs leading-relaxed text-muted-foreground">
              {{ $st('Edit the full permission JSON for this scope, including all three permission pages.') }}
            </p>
          </div>
          <span v-if="rawPending" class="h-1.5 w-1.5 shrink-0 rounded-full bg-primary" />
          <RiArrowDownSLine class="h-4 w-4 shrink-0 transition-transform group-open:rotate-180" />
        </summary>
        <div class="grid min-w-0 gap-3 p-3 sm:p-4">
          <textarea
            v-model="rawJson"
            rows="12"
            spellcheck="false"
            class="w-full min-w-0 resize-y rounded-lg border border-input bg-background/40 p-3 font-mono text-xs leading-relaxed outline-none focus-visible:ring-1 focus-visible:ring-ring"
            :aria-label="$st('Raw PermissionConfig')"
            :readonly="editingDisabled"
            :aria-invalid="Boolean(rawJsonError)"
          />
          <p v-if="rawJsonError" class="text-xs text-destructive" role="alert">{{ rawJsonError }}</p>
          <div v-if="canEdit" class="flex justify-end">
            <Button variant="outline" size="sm" :disabled="editingDisabled || !rawPending" @click="applyRawJson">{{
              $st('Apply JSON')
            }}</Button>
          </div>
        </div>
      </details>
    </template>
    <div
      v-if="canEdit && loaded && (dirty || saving)"
      class="permission-savebar sticky bottom-0 z-20 flex flex-wrap items-center justify-between gap-3 rounded-xl border border-border bg-background/95 p-3 shadow-lg backdrop-blur"
      data-permission-savebar
    >
      <div class="min-w-0 text-xs text-muted-foreground">
        <span class="font-medium text-foreground">{{ $st('Unsaved changes') }}</span>
        <p class="mt-0.5">{{ $st('Save applies changes across all permission pages in this scope.') }}</p>
        <p v-if="saveError" class="mt-1 break-words text-destructive" role="alert">{{ saveError }}</p>
      </div>
      <div class="flex flex-wrap items-center gap-2">
        <Button variant="ghost" size="sm" :disabled="saving" @click="discard">{{ $st('Discard changes') }}</Button>
        <Button size="sm" :disabled="editingDisabled" @click="save"
          ><RiSave3Line class="mr-1.5 h-4 w-4" />{{ saving ? $st('Saving…') : $st('Save permission') }}</Button
        >
      </div>
    </div>
  </section>
</template>

<style scoped>
@reference '../../style.css';
.permission-studio {
  container: permissions / inline-size;
}
.policy-grid {
  display: grid;
  gap: 1.25rem;
}
.policy-item {
  display: grid;
  min-width: 0;
  gap: 1rem;
  align-content: start;
}
.policy-item + .policy-item {
  @apply border-t border-border/50;
  padding-block-start: 1.25rem;
}
.permission-control {
  display: grid;
  min-width: 0;
  gap: 0.375rem;
}
.permission-control > span {
  @apply text-xs text-muted-foreground;
}
.permission-rule {
  @apply rounded-lg border border-border/50 bg-background/20;
  display: grid;
  grid-template-columns: 1.25rem minmax(0, 1fr) minmax(0, 1fr) 2rem;
  gap: 0.5rem;
  min-width: 0;
  padding: 0.75rem;
  align-items: center;
}
.permission-rule-labels {
  display: none;
}
.rule-select {
  grid-column: 1;
  grid-row: 1;
  display: flex;
  justify-content: center;
}
.rule-target {
  grid-column: 2 / 4;
  grid-row: 1;
  min-width: 0;
}
.rule-modes {
  grid-column: 2 / -1;
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 0.5rem;
}
.rule-modes[data-single-mode] {
  grid-template-columns: minmax(0, 1fr);
}
.rule-remove {
  grid-column: 4;
  grid-row: 1;
}
.permission-target-input {
  @apply h-9 w-full min-w-0 rounded-md border border-input bg-transparent px-2.5 font-mono text-xs outline-none focus-visible:ring-1 focus-visible:ring-ring;
}
.permission-target-input:read-only {
  @apply border-transparent;
}
.rule-create,
.command-inputs {
  display: grid;
  min-width: 0;
  gap: 0.75rem;
  align-items: end;
}
.permission-details {
  @apply min-w-0 rounded-xl border border-border/70 bg-card/30;
}
.permission-summary {
  @apply flex cursor-pointer list-none items-center gap-3 rounded-xl bg-muted/20 px-4 py-3.5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring;
}
.permission-summary::-webkit-details-marker {
  display: none;
}
.permission-details[open] > .permission-summary {
  @apply rounded-b-none border-b border-border/60;
}
.command-class-row {
  display: grid;
  min-width: 0;
  gap: 0.75rem;
  padding-block: 1rem;
}
.command-class-row + .command-class-row {
  @apply border-t border-border/50;
}
.command-class-modes {
  display: grid;
  gap: 0.75rem;
  grid-template-columns: repeat(auto-fit, minmax(min(100%, 8rem), 1fr));
}
@container permissions (min-width: 40rem) {
  .policy-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 2rem;
  }
  .policy-item + .policy-item {
    border-block-start: 0;
    padding-block-start: 0;
    border-inline-start: 1px solid oklch(var(--border) / 0.5);
    padding-inline-start: 2rem;
  }
  .network-policies {
    grid-template-columns: repeat(3, minmax(0, 1fr));
  }
  .permission-rule {
    grid-template-columns: 1.25rem minmax(0, 1fr) minmax(15rem, 32%) 2rem;
  }
  .permission-rule-labels {
    @apply text-xs text-muted-foreground;
    display: grid;
    grid-template-columns: 1.25rem minmax(0, 1fr) minmax(15rem, 32%) 2rem;
    column-gap: 0.5rem;
    padding-inline: 0.75rem;
  }
  .permission-rule .rule-modes .permission-control > span {
    display: none;
  }
  .rule-target {
    grid-column: 2;
    align-self: end;
  }
  .rule-modes {
    grid-column: 3;
    grid-row: 1;
  }
  .rule-remove {
    align-self: end;
    margin-block-end: 0.125rem;
  }
  .rule-select {
    align-self: end;
    padding-block-end: 0.625rem;
  }
  .rule-create {
    grid-template-columns: minmax(0, 1fr) minmax(12rem, 28%) auto;
  }
  .command-class-row {
    grid-template-columns: minmax(0, 1fr) minmax(24rem, 64%);
    align-items: center;
  }
}
@container permissions (min-width: 64rem) {
  .command-inputs {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}
@media (prefers-reduced-motion: reduce) {
  .permission-studio :deep(*) {
    animation: none;
    transition: none;
  }
}
</style>
