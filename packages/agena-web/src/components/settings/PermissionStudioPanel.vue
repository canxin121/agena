<script setup lang="ts">
import { computed, defineAsyncComponent, onMounted, ref, watch, type Ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiAddLine, RiDeleteBinLine, RiRefreshLine, RiSave3Line } from '@remixicon/vue'

import Button from '@/components/ui/Button.vue'
import { findSettingsScope } from '@/components/settings/settingsScopeCatalog'
import IconButton from '@/components/ui/IconButton.vue'
import Input from '@/components/ui/Input.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import { apiJson } from '@/lib/api'
import { deleteRuntimeSetting, setRuntimeSetting } from '@/lib/runtimeSettings'
import { useChatStore } from '@/stores/chat'
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
const { t } = useI18n()
const chat = useChatStore()
const toasts = useToastsStore()
const PermissionsPanel = defineAsyncComponent(() => import('@/components/settings/PermissionsPanel.vue'))

const loading = ref(false)
const saving = ref(false)
const error = ref('')
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
const newCommandTool = ref('shell')
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
const selectedPathRules = computed(() => pathRules.value.filter(([path]) => selectedPathRuleKeys.value.has(path)))
const selectedNetworkRules = computed(() =>
  networkRules.value.filter(([target]) => selectedNetworkRuleKeys.value.has(target)),
)
const selectedToolNameRules = computed(() =>
  toolNameRules.value.filter(([name]) => selectedToolNameRuleKeys.value.has(name)),
)
const selectedCommandRules = computed(() =>
  commandRules.value.filter((row) => selectedCommandRuleKeys.value.has(commandRuleKey(row.tool, row.command))),
)
const allPathRulesSelected = computed(() =>
  areAllSelected(
    pathRules.value.map(([path]) => path),
    selectedPathRuleKeys.value,
  ),
)
const allNetworkRulesSelected = computed(() =>
  areAllSelected(
    networkRules.value.map(([target]) => target),
    selectedNetworkRuleKeys.value,
  ),
)
const allToolNameRulesSelected = computed(() =>
  areAllSelected(
    toolNameRules.value.map(([name]) => name),
    selectedToolNameRuleKeys.value,
  ),
)
const allCommandRulesSelected = computed(() =>
  areAllSelected(
    commandRules.value.map((row) => commandRuleKey(row.tool, row.command)),
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
    pathRules.value.map(([path]) => path),
  )
}
function toggleAllNetworkRules() {
  toggleAllSelection(
    selectedNetworkRuleKeys,
    networkRules.value.map(([target]) => target),
  )
}
function toggleAllToolNameRules() {
  toggleAllSelection(
    selectedToolNameRuleKeys,
    toolNameRules.value.map(([name]) => name),
  )
}
function toggleAllCommandRules() {
  toggleAllSelection(
    selectedCommandRuleKeys,
    commandRules.value.map((row) => commandRuleKey(row.tool, row.command)),
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
  [pathRules, networkRules, toolNameRules, commandRules],
  () => {
    pruneSelection(
      selectedPathRuleKeys,
      pathRules.value.map(([path]) => path),
    )
    pruneSelection(
      selectedNetworkRuleKeys,
      networkRules.value.map(([target]) => target),
    )
    pruneSelection(
      selectedToolNameRuleKeys,
      toolNameRules.value.map(([name]) => name),
    )
    pruneSelection(
      selectedCommandRuleKeys,
      commandRules.value.map((row) => commandRuleKey(row.tool, row.command)),
    )
  },
  { flush: 'sync' },
)

// The built-in entries, offered as one-click rows so the shipped behaviour is
// visible and adjustable without typing the keywords by hand. Each one is an
// ordinary rule row: writing it creates an override, clearing it restores the
// built-in.
const BUILT_IN_RULES: ReadonlyArray<{ tool: string; command: string }> = [
  ...SHELL_LAUNCH_TOOLS.flatMap((tool) => [
    { tool, command: 'no-op' },
    { tool, command: 'routine' },
    { tool, command: 'dangerous' },
  ]),
  { tool: 'agena.shell.write', command: 'dangerous' },
  { tool: '*', command: 'read-only' },
]
const writtenRules = computed(() => new Set(commandRules.value.map((row) => `${row.tool}\u0000${row.command}`)))
const builtInRuleRows = computed(() =>
  BUILT_IN_RULES.filter((rule) => !writtenRules.value.has(`${rule.tool}\u0000${rule.command}`)).map((rule) => ({
    ...rule,
    mode: builtInRuleMode(rule.tool, rule.command),
  })),
)

// The shipped mode of a built-in rule, shown while the user has not written one
// of their own. Mirrors `PermissionConfig::global_default()`.
function builtInRuleMode(tool: string, command: string): PermissionMode {
  if (tool === '*' && command === 'read-only') return 'allow'
  if (isShellCapableTool(tool)) {
    if (command === 'dangerous') return 'deny'
    return 'allow'
  }
  return 'allow'
}

function writeRule(tool: string, command: string, value: string) {
  mutate((next) => {
    next.tools ||= {}
    const current = next.tools.rules?.[tool]
    const entries: Record<string, PermissionMode> =
      current && typeof current === 'object' && !Array.isArray(current) ? { ...current } : {}
    if (isMode(value)) entries[command] = value
    else delete entries[command]
    if (Object.keys(entries).length) next.tools.rules = { ...(next.tools.rules || {}), [tool]: entries }
    else if (next.tools.rules) {
      const rules = { ...next.tools.rules }
      delete rules[tool]
      if (Object.keys(rules).length) next.tools.rules = rules
      else delete next.tools.rules
    }
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
  setCommandMode(tool, command, newCommandMode.value)
  newCommandPattern.value = ''
}

function applyRawJson() {
  try {
    const parsed = JSON.parse(rawJson.value) as JsonValue
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed))
      throw new Error(st('Permission config must be a JSON object.'))
    config.value = asConfig(parsed)
    rawJsonError.value = ''
  } catch (reason) {
    rawJsonError.value = reason instanceof Error ? reason.message : String(reason)
  }
}

async function load() {
  loading.value = true
  error.value = ''
  clearAllRuleSelections()
  try {
    const [effective, global, workspace, toolResponse] = await Promise.all([
      apiJson<SettingsResponse>('/api/v1/settings?source=effective&path=permission'),
      apiJson<SettingsResponse>('/api/v1/settings/layers/global?path=permission'),
      apiJson<SettingsResponse>('/api/v1/settings/layers/workspace?path=permission'),
      apiJson<ToolCatalogResponse>('/api/v1/plugins/surface'),
    ])
    toolCatalog.value = (Array.isArray(toolResponse?.permission_tools) ? toolResponse.permission_tools : [])
      .map((item) => ({ name: String(item?.name || '').trim(), summary: String(item?.summary || '').trim() }))
      .filter((item) => item.name)
      .sort((a, b) => a.name.localeCompare(b.name))

    let sessionConfig: PermissionConfig = {}
    let sessionEffective: PermissionConfig = {}
    if (activeSessionId.value) {
      const session = await apiJson<SessionStateResponse>(`/api/v1/sessions/${activeSessionId.value}/state`)
      const nestedExecution = session.execution?.execution
      sessionConfig = asConfig(
        nestedExecution?.selected_permission ||
          session.execution?.selected_permission ||
          session.execution?.context?.selected_permission,
      )
      sessionEffective = asConfig(
        nestedExecution?.effective_permission ||
          session.execution?.effective_permission ||
          session.execution?.context?.effective_permission,
      )
    }
    const sourceValue =
      selectedSource.value === 'global'
        ? global?.value
        : selectedSource.value === 'workspace'
          ? workspace?.value
          : selectedSource.value === 'session'
            ? sessionConfig
            : Object.keys(sessionEffective).length
              ? sessionEffective
              : effective?.value
    config.value = asConfig(sourceValue)
    syncRaw()
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    loading.value = false
  }
}

async function save() {
  if (!canEdit.value || saving.value) return
  saving.value = true
  error.value = ''
  try {
    if (selectedSource.value === 'session') {
      await apiJson(`/api/v1/sessions/${activeSessionId.value}/permission`, {
        method: 'PUT',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ permission: config.value }),
      })
    } else {
      await setRuntimeSetting(
        'permission',
        config.value as JsonValue,
        { reload: true },
        selectedSource.value as 'global' | 'workspace',
      )
    }
    toasts.push(
      'success',
      st('{scope} updated', {
        scope: findSettingsScope('permissions', props.section, selectedSource.value)?.label || st('Permission'),
      }),
    )
    await load()
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    saving.value = false
  }
}

async function clearSource() {
  if (!canEdit.value || saving.value) return
  saving.value = true
  try {
    if (selectedSource.value === 'session') {
      await apiJson(`/api/v1/sessions/${activeSessionId.value}/permission`, {
        method: 'PUT',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ permission: {} }),
      })
    } else {
      await deleteRuntimeSetting('permission', { reload: true }, selectedSource.value as 'global' | 'workspace')
    }
    await load()
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    saving.value = false
  }
}

watch([selectedSource, activeSessionId], () => void load())
onMounted(() => void load())
</script>

<template>
  <section class="grid gap-4">
    <div class="flex justify-end">
      <Button variant="outline" size="sm" :disabled="loading" @click="load"
        ><RiRefreshLine class="mr-2 h-4 w-4" :class="loading ? 'animate-spin' : ''" /> {{ $st('Refresh') }}</Button
      >
    </div>

    <div
      v-if="error"
      class="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
    >
      {{ error }}
    </div>

    <div class="grid gap-4">
      <div
        v-if="selectedSource === 'session' && !activeSessionId"
        class="rounded-md border border-dashed border-border/60 px-3 py-3 text-xs text-muted-foreground"
      >
        {{ $st('Open a session to edit current-session permission.') }}
      </div>

      <div class="grid min-w-0 gap-4">
        <div class="flex flex-wrap items-center justify-end gap-2 border-b border-border/60 pb-3">
          <Button v-if="canEdit" variant="ghost" size="sm" :disabled="saving" @click="clearSource"
            ><RiDeleteBinLine class="mr-1.5 h-4 w-4" /> {{ $st('Clear source') }}</Button
          >
          <Button v-if="canEdit" size="sm" :disabled="saving" @click="save"
            ><RiSave3Line class="mr-1.5 h-4 w-4" /> {{ saving ? $st('Saving…') : $st('Save permission') }}</Button
          >
        </div>

        <fieldset :disabled="!canEdit" class="contents">
          <div v-if="selectedSection === 'path'" class="grid gap-4">
            <div>
              <h3 class="text-sm font-medium">{{ $st('Path Defaults') }}</h3>
              <p class="mt-1 text-xs text-muted-foreground">
                {{ $st('Set separate read and write decisions for workspace and external paths.') }}
              </p>
            </div>
            <div class="grid gap-3 sm:grid-cols-2">
              <div
                v-for="scope in ['workspace', 'external'] as const"
                :key="scope"
                class="rounded-md border border-border/60 p-3"
              >
                <div class="mb-2 text-xs font-medium capitalize">{{ scope }}</div>
                <div class="grid gap-2">
                  <label class="grid gap-1"
                    ><span class="text-[11px] text-muted-foreground">{{ $st('Read') }}</span
                    ><OptionPicker
                      :model-value="modeAt(scope, 'read')"
                      :options="modeOptions"
                      :include-empty="true"
                      :empty-label="$st('Default')"
                      :title="$st('Path read mode')"
                      @update:model-value="setPathDefault(scope, 'read', $event)"
                  /></label>
                  <label class="grid gap-1"
                    ><span class="text-[11px] text-muted-foreground">{{ $st('Write') }}</span
                    ><OptionPicker
                      :model-value="modeAt(scope, 'write')"
                      :options="modeOptions"
                      :include-empty="true"
                      :empty-label="$st('Default')"
                      :title="$st('Path write mode')"
                      @update:model-value="setPathDefault(scope, 'write', $event)"
                  /></label>
                </div>
              </div>
            </div>
            <div class="grid gap-2">
              <div class="text-sm font-medium">{{ $st('Path Rules') }}</div>
              <p class="text-xs text-muted-foreground">
                {{
                  $st(
                    'Per-pattern rules, last match wins. The temporary and managed project-state directories are built-in rules here; writing the same pattern replaces one, deleting your entry restores it.',
                  )
                }}
              </p>
              <div
                v-if="pathRules.length"
                class="mt-2 flex flex-wrap items-center gap-2 rounded-md border border-border/60 bg-muted/10 px-3 py-2"
              >
                <label class="flex items-center gap-2 text-xs font-medium">
                  <input
                    type="checkbox"
                    :checked="allPathRulesSelected"
                    :aria-label="$st('Select all permission rules')"
                    @change="toggleAllPathRules"
                  />
                  {{ $st('Select all') }} ({{ pathRules.length }})
                </label>
                <span class="text-xs text-muted-foreground">{{
                  $st('{count} selected', { count: selectedPathRules.length })
                }}</span>
                <div class="ml-auto flex flex-wrap items-center gap-2">
                  <Button
                    variant="ghost"
                    size="sm"
                    :disabled="!selectedPathRules.length"
                    @click="clearPathRuleSelection"
                  >
                    {{ $st('Select none') }}
                  </Button>
                  <div class="flex items-center gap-1.5">
                    <span class="text-xs text-muted-foreground">{{ $st('Read') }}</span>
                    <div class="w-32">
                      <OptionPicker
                        v-model="bulkPathReadMode"
                        :options="modeOptions"
                        :include-empty="true"
                        :empty-label="$st('Read default')"
                        :title="$st('Path rule read mode')"
                        :disabled="!selectedPathRules.length"
                        size="sm"
                      />
                    </div>
                    <Button
                      variant="outline"
                      size="sm"
                      :disabled="!selectedPathRules.length"
                      @click="applySelectedPathReadMode"
                    >
                      {{ $st('Set mode ({count})', { count: selectedPathRules.length }) }}
                    </Button>
                  </div>
                  <div class="flex items-center gap-1.5">
                    <span class="text-xs text-muted-foreground">{{ $st('Write') }}</span>
                    <div class="w-32">
                      <OptionPicker
                        v-model="bulkPathWriteMode"
                        :options="modeOptions"
                        :include-empty="true"
                        :empty-label="$st('Write default')"
                        :title="$st('Path rule write mode')"
                        :disabled="!selectedPathRules.length"
                        size="sm"
                      />
                    </div>
                    <Button
                      variant="outline"
                      size="sm"
                      :disabled="!selectedPathRules.length"
                      @click="applySelectedPathWriteMode"
                    >
                      {{ $st('Set mode ({count})', { count: selectedPathRules.length }) }}
                    </Button>
                  </div>
                  <Button
                    variant="destructive"
                    size="sm"
                    :disabled="!selectedPathRules.length"
                    @click="removeSelectedPathRules"
                  >
                    {{ $st('Remove selected ({count})', { count: selectedPathRules.length }) }}
                  </Button>
                </div>
              </div>
              <div
                v-for="[path, rule] in pathRules"
                :key="path"
                class="grid gap-2 rounded-md border border-border/60 p-3 sm:grid-cols-[auto_minmax(0,1.5fr)_minmax(8rem,1fr)_minmax(8rem,1fr)_auto] sm:items-end"
              >
                <label class="flex h-9 items-center justify-center">
                  <input
                    type="checkbox"
                    :checked="selectedPathRuleKeys.has(path)"
                    @change="togglePathRule(path, $event)"
                  />
                  <span class="sr-only">{{ path }}</span>
                </label>
                <input
                  :value="path"
                  type="text"
                  class="h-9 min-w-0 rounded-md border border-input bg-transparent px-2.5 font-mono text-xs outline-none focus:border-ring"
                  :title="$st('Rename path rule {path}', { path })"
                  @change="renamePathRule(path, $event)"
                />
                <OptionPicker
                  :model-value="pathRuleMode(rule, 'read')"
                  :options="modeOptions"
                  :include-empty="true"
                  :empty-label="$st('Read default')"
                  :title="$st('Path rule read mode')"
                  @update:model-value="setPathRuleMode(path, 'read', $event)"
                />
                <OptionPicker
                  :model-value="pathRuleMode(rule, 'write')"
                  :options="modeOptions"
                  :include-empty="true"
                  :empty-label="$st('Write default')"
                  :title="$st('Path rule write mode')"
                  @update:model-value="setPathRuleMode(path, 'write', $event)"
                />
                <IconButton
                  variant="ghost"
                  size="sm"
                  :tooltip="$st('Remove path rule')"
                  :aria-label="$st('Remove path rule')"
                  @click="removePathRule(path)"
                  ><RiDeleteBinLine class="h-4 w-4 text-destructive"
                /></IconButton>
              </div>
              <div
                class="grid gap-2 sm:grid-cols-[minmax(0,1.5fr)_minmax(8rem,1fr)_minmax(8rem,1fr)_auto] sm:items-end"
              >
                <Input
                  v-model="newPath"
                  class="font-mono"
                  placeholder="/path or relative/path"
                  @keydown.enter="addPathRule"
                />
                <OptionPicker
                  v-model="newPathReadMode"
                  :options="modeOptions"
                  :include-empty="false"
                  :title="$st('New path read mode')"
                />
                <OptionPicker
                  v-model="newPathWriteMode"
                  :options="modeOptions"
                  :include-empty="false"
                  :title="$st('New path write mode')"
                />
                <Button variant="outline" size="sm" :disabled="!newPath.trim()" @click="addPathRule">
                  <RiAddLine class="mr-1.5 h-4 w-4" /> {{ $st('Add path rule') }}
                </Button>
              </div>
            </div>
          </div>

          <div v-else-if="selectedSection === 'network'" class="grid gap-4">
            <div>
              <h3 class="text-sm font-medium">{{ $st('Network Zones') }}</h3>
              <p class="mt-1 text-xs text-muted-foreground">
                {{ $st('Control internet, private-network, and loopback access, then add domain/host rules.') }}
              </p>
            </div>
            <div class="grid gap-3 sm:grid-cols-3">
              <label v-for="zone in ['internet', 'private', 'loopback'] as const" :key="zone" class="grid gap-1.5"
                ><span class="text-xs capitalize text-muted-foreground">{{ zone }}</span
                ><OptionPicker
                  :model-value="networkMode(zone)"
                  :options="modeOptions"
                  :include-empty="true"
                  :empty-label="$st('Default')"
                  :title="$st('Network zone mode')"
                  @update:model-value="setNetworkMode(zone, $event)"
              /></label>
            </div>
            <div class="grid gap-2">
              <div class="text-sm font-medium">{{ $st('Domain Rules') }}</div>
              <div
                v-if="networkRules.length"
                class="flex flex-wrap items-center gap-2 rounded-md border border-border/60 bg-muted/10 px-3 py-2"
              >
                <label class="flex items-center gap-2 text-xs font-medium">
                  <input type="checkbox" :checked="allNetworkRulesSelected" @change="toggleAllNetworkRules" />
                  {{ $st('Select all') }} ({{ networkRules.length }})
                </label>
                <span class="text-xs text-muted-foreground">{{
                  $st('{count} selected', { count: selectedNetworkRules.length })
                }}</span>
                <div class="ml-auto flex flex-wrap items-center gap-2">
                  <Button
                    variant="ghost"
                    size="sm"
                    :disabled="!selectedNetworkRules.length"
                    @click="clearNetworkRuleSelection"
                  >
                    {{ $st('Select none') }}
                  </Button>
                  <div class="w-36">
                    <OptionPicker
                      v-model="bulkNetworkMode"
                      :options="modeOptions"
                      :include-empty="false"
                      :title="$st('Network rule mode')"
                      :disabled="!selectedNetworkRules.length"
                      size="sm"
                    />
                  </div>
                  <Button
                    variant="outline"
                    size="sm"
                    :disabled="!selectedNetworkRules.length"
                    @click="setSelectedNetworkMode(bulkNetworkMode)"
                    >{{ $st('Set mode ({count})', { count: selectedNetworkRules.length }) }}</Button
                  >
                  <Button
                    variant="destructive"
                    size="sm"
                    :disabled="!selectedNetworkRules.length"
                    @click="removeSelectedNetworkRules"
                    >{{ $st('Remove selected ({count})', { count: selectedNetworkRules.length }) }}</Button
                  >
                </div>
              </div>
              <div
                v-for="[target, mode] in networkRules"
                :key="target"
                class="grid gap-2 rounded-md border border-border/60 p-3 sm:grid-cols-[auto_minmax(0,1fr)_minmax(8rem,0.5fr)_auto] sm:items-center"
              >
                <label class="flex h-9 items-center justify-center">
                  <input
                    type="checkbox"
                    :checked="selectedNetworkRuleKeys.has(target)"
                    @change="toggleNetworkRule(target, $event)"
                  />
                  <span class="sr-only">{{ target }}</span>
                </label>
                <input
                  :value="target"
                  type="text"
                  class="h-9 min-w-0 rounded-md border border-input bg-transparent px-2.5 font-mono text-xs outline-none focus:border-ring"
                  :title="$st('Rename network rule {target}', { target })"
                  @change="renameNetworkRule(target, $event)"
                /><OptionPicker
                  :model-value="mode"
                  :options="modeOptions"
                  :include-empty="false"
                  :title="$st('Network rule mode')"
                  @update:model-value="setNetworkRule(target, $event)"
                /><IconButton
                  variant="ghost"
                  size="sm"
                  :tooltip="$st('Remove network rule')"
                  :aria-label="$st('Remove network rule')"
                  @click="removeNetworkRule(target)"
                  ><RiDeleteBinLine class="h-4 w-4 text-destructive"
                /></IconButton>
              </div>
              <div class="grid gap-2 sm:grid-cols-[minmax(0,1fr)_minmax(8rem,0.5fr)_auto] sm:items-end">
                <Input
                  v-model="newNetworkRule"
                  class="font-mono"
                  placeholder="example.com or 127.0.0.1:8080"
                  @keydown.enter="addNetworkRule"
                />
                <OptionPicker
                  v-model="newNetworkRuleMode"
                  :options="modeOptions"
                  :include-empty="false"
                  :title="$st('New network rule mode')"
                />
                <Button variant="outline" size="sm" :disabled="!newNetworkRule.trim()" @click="addNetworkRule">
                  <RiAddLine class="mr-1.5 h-4 w-4" /> {{ $st('Add domain rule') }}
                </Button>
              </div>
            </div>
          </div>

          <div v-else class="grid gap-4">
            <div>
              <h3 class="text-sm font-medium">{{ $st('Tool Access') }}</h3>
              <p class="mt-1 text-xs text-muted-foreground">
                {{ $st('Manage the default tool policy, individual tool names, and command patterns.') }}
              </p>
            </div>
            <div class="grid gap-2">
              <div class="text-sm font-medium">{{ $st('Tool Defaults') }}</div>
              <p class="text-xs text-muted-foreground">
                {{
                  $st(
                    'The fallback for any tool the Name Rules below do not name. The interaction tool is a Name Rule; write its name there to change it.',
                  )
                }}
              </p>
              <label class="grid max-w-sm gap-1.5"
                ><span class="text-xs text-muted-foreground">{{ $st('Default tool mode') }}</span
                ><OptionPicker
                  :model-value="config.tools?.default || ''"
                  :options="modeOptions"
                  :include-empty="true"
                  :empty-label="$st('Default')"
                  :title="$st('Default tool mode')"
                  @update:model-value="setToolDefault"
              /></label>
            </div>
            <div class="grid gap-2">
              <div class="text-sm font-medium">{{ $st('Name Rules') }}</div>
              <div
                v-if="toolNameRules.length"
                class="flex flex-wrap items-center gap-2 rounded-md border border-border/60 bg-muted/10 px-3 py-2"
              >
                <label class="flex items-center gap-2 text-xs font-medium">
                  <input type="checkbox" :checked="allToolNameRulesSelected" @change="toggleAllToolNameRules" />
                  {{ $st('Select all') }} ({{ toolNameRules.length }})
                </label>
                <span class="text-xs text-muted-foreground">{{
                  $st('{count} selected', { count: selectedToolNameRules.length })
                }}</span>
                <div class="ml-auto flex flex-wrap items-center gap-2">
                  <Button
                    variant="ghost"
                    size="sm"
                    :disabled="!selectedToolNameRules.length"
                    @click="clearToolNameRuleSelection"
                  >
                    {{ $st('Select none') }}
                  </Button>
                  <div class="w-36">
                    <OptionPicker
                      v-model="bulkToolNameMode"
                      :options="modeOptions"
                      :include-empty="false"
                      :title="$st('Tool name mode')"
                      :disabled="!selectedToolNameRules.length"
                      size="sm"
                    />
                  </div>
                  <Button
                    variant="outline"
                    size="sm"
                    :disabled="!selectedToolNameRules.length"
                    @click="setSelectedToolNameMode(bulkToolNameMode)"
                    >{{ $st('Set mode ({count})', { count: selectedToolNameRules.length }) }}</Button
                  >
                  <Button
                    variant="destructive"
                    size="sm"
                    :disabled="!selectedToolNameRules.length"
                    @click="removeSelectedToolNameRules"
                    >{{ $st('Remove selected ({count})', { count: selectedToolNameRules.length }) }}</Button
                  >
                </div>
              </div>
              <div
                v-for="[name, mode] in toolNameRules"
                :key="name"
                class="grid gap-2 rounded-md border border-border/60 p-3 sm:grid-cols-[auto_minmax(0,1fr)_minmax(8rem,0.5fr)_auto] sm:items-center"
              >
                <label class="flex h-9 items-center justify-center">
                  <input
                    type="checkbox"
                    :checked="selectedToolNameRuleKeys.has(name)"
                    @change="toggleToolNameRule(name, $event)"
                  />
                  <span class="sr-only">{{ name }}</span>
                </label>
                <input
                  :value="name"
                  type="text"
                  class="h-9 min-w-0 rounded-md border border-input bg-transparent px-2.5 font-mono text-xs outline-none focus:border-ring"
                  :title="$st('Rename tool rule {name}', { name })"
                  @change="renameToolName(name, $event)"
                /><OptionPicker
                  :model-value="mode"
                  :options="modeOptions"
                  :include-empty="false"
                  :title="$st('Tool name mode')"
                  @update:model-value="setToolNameMode(name, $event)"
                /><IconButton
                  variant="ghost"
                  size="sm"
                  :tooltip="$st('Remove tool rule')"
                  :aria-label="$st('Remove tool rule')"
                  @click="removeToolName(name)"
                  ><RiDeleteBinLine class="h-4 w-4 text-destructive"
                /></IconButton>
              </div>
              <div class="grid gap-2 sm:grid-cols-[minmax(0,1fr)_minmax(8rem,0.5fr)_auto] sm:items-end">
                <Input
                  v-model="newToolName"
                  class="font-mono"
                  placeholder="shell or agena.web.fetch"
                  @keydown.enter="addToolName"
                />
                <OptionPicker
                  v-model="newToolNameMode"
                  :options="modeOptions"
                  :include-empty="false"
                  :title="$st('New tool rule mode')"
                />
                <Button variant="outline" size="sm" :disabled="!newToolName.trim()" @click="addToolName">
                  <RiAddLine class="mr-1.5 h-4 w-4" /> {{ $st('Add tool name') }}
                </Button>
              </div>
              <div v-if="toolCatalog.length" class="flex flex-wrap gap-1.5">
                <Button
                  v-for="tool in toolCatalog"
                  :key="tool.name"
                  variant="ghost"
                  size="sm"
                  class="font-mono text-[11px]"
                  @click="newToolName = tool.name"
                  >{{ tool.name }}</Button
                >
              </div>
            </div>
            <div class="grid gap-2">
              <div class="text-sm font-medium">{{ $st('Command Rules') }}</div>
              <p class="text-xs text-muted-foreground">
                {{
                  $st(
                    'Per-command rules, last match wins. A pattern is either a command pattern or one of the class keywords no-op, routine, and dangerous. The built-in rules are listed below; writing one of the same names replaces it, deleting it restores the built-in.',
                  )
                }}
              </p>
              <div
                v-if="commandRules.length"
                class="flex flex-wrap items-center gap-2 rounded-md border border-border/60 bg-muted/10 px-3 py-2"
              >
                <label class="flex items-center gap-2 text-xs font-medium">
                  <input type="checkbox" :checked="allCommandRulesSelected" @change="toggleAllCommandRules" />
                  {{ $st('Select all') }} ({{ commandRules.length }})
                </label>
                <span class="text-xs text-muted-foreground">{{
                  $st('{count} selected', { count: selectedCommandRules.length })
                }}</span>
                <div class="ml-auto flex flex-wrap items-center gap-2">
                  <Button
                    variant="ghost"
                    size="sm"
                    :disabled="!selectedCommandRules.length"
                    @click="clearCommandRuleSelection"
                  >
                    {{ $st('Select none') }}
                  </Button>
                  <div class="w-36">
                    <OptionPicker
                      v-model="bulkCommandMode"
                      :options="modeOptions"
                      :include-empty="false"
                      :title="$st('Command rule mode')"
                      :disabled="!selectedCommandRules.length"
                      size="sm"
                    />
                  </div>
                  <Button
                    variant="outline"
                    size="sm"
                    :disabled="!selectedCommandRules.length"
                    @click="setSelectedCommandMode(bulkCommandMode)"
                    >{{ $st('Set mode ({count})', { count: selectedCommandRules.length }) }}</Button
                  >
                  <Button
                    variant="destructive"
                    size="sm"
                    :disabled="!selectedCommandRules.length"
                    @click="removeSelectedCommandRules"
                    >{{ $st('Remove selected ({count})', { count: selectedCommandRules.length }) }}</Button
                  >
                </div>
              </div>
              <div
                v-for="row in commandRules"
                :key="`${row.tool}:${row.command}`"
                class="grid gap-2 rounded-md border border-border/60 p-3 sm:grid-cols-[auto_minmax(8rem,0.6fr)_minmax(0,1.4fr)_minmax(8rem,0.5fr)_auto] sm:items-center"
              >
                <label class="flex h-9 items-center justify-center">
                  <input
                    type="checkbox"
                    :checked="selectedCommandRuleKeys.has(commandRuleKey(row.tool, row.command))"
                    @change="toggleCommandRule(row.tool, row.command, $event)"
                  />
                  <span class="sr-only">{{ row.tool }} {{ row.command }}</span>
                </label>
                <code class="break-all text-xs">{{ row.tool }}</code>
                <input
                  :value="row.command"
                  type="text"
                  class="h-9 min-w-0 rounded-md border border-input bg-transparent px-2.5 font-mono text-xs outline-none focus:border-ring"
                  :title="$st('Rename command rule {command}', { command: row.command })"
                  @change="renameCommandRule(row.tool, row.command, $event)"
                /><OptionPicker
                  :model-value="row.mode"
                  :options="modeOptions"
                  :include-empty="false"
                  :title="$st('Command rule mode')"
                  @update:model-value="setCommandMode(row.tool, row.command, $event)"
                /><IconButton
                  variant="ghost"
                  size="sm"
                  :tooltip="$st('Remove command rule')"
                  :aria-label="$st('Remove command rule')"
                  @click="removeCommandRule(row.tool, row.command)"
                  ><RiDeleteBinLine class="h-4 w-4 text-destructive"
                /></IconButton>
              </div>
              <div
                v-for="row in builtInRuleRows"
                :key="`builtin:${row.tool}:${row.command}`"
                class="grid gap-2 rounded-md border border-dashed border-border/60 p-3 sm:grid-cols-[minmax(8rem,0.6fr)_minmax(0,1.4fr)_minmax(8rem,0.5fr)_auto] sm:items-center"
              >
                <code class="break-all text-xs">{{ row.tool }}</code>
                <code class="text-xs">{{ row.command }}</code>
                <OptionPicker
                  :model-value="row.mode"
                  :options="modeOptions"
                  :include-empty="true"
                  :empty-label="$st('Built-in')"
                  :title="$st('Built-in rule mode')"
                  @update:model-value="writeRule(row.tool, row.command, $event)"
                />
                <span class="text-[11px] text-muted-foreground">{{ $st('Built-in') }}</span>
              </div>
              <div
                class="grid gap-2 sm:grid-cols-[minmax(8rem,0.6fr)_minmax(0,1.4fr)_minmax(8rem,0.5fr)_auto] sm:items-end"
              >
                <OptionPicker
                  v-model="newCommandTool"
                  :options="shellToolOptions"
                  :include-empty="false"
                  :title="$st('Shell-capable tool')"
                  monospace
                /><Input v-model="newCommandPattern" class="font-mono" placeholder="git push *" /><OptionPicker
                  v-model="newCommandMode"
                  :options="modeOptions"
                  :include-empty="false"
                  :title="$st('New command mode')"
                /><Button variant="outline" size="sm" :disabled="!newCommandPattern.trim()" @click="addCommandRule"
                  ><RiAddLine class="mr-1.5 h-4 w-4" /> {{ $st('Add command rule') }}</Button
                >
              </div>
            </div>
          </div>
        </fieldset>

        <section class="grid gap-2 border-t border-border/60 pt-4">
          <div class="flex flex-wrap items-center justify-between gap-2">
            <div>
              <div class="text-sm font-medium">{{ $st('Raw PermissionConfig') }}</div>
              <div class="mt-1 text-xs text-muted-foreground">
                {{ $st('Use this escape hatch for any TUI-supported field not currently expanded above.') }}
              </div>
            </div>
            <Button variant="outline" size="sm" :disabled="!canEdit" @click="applyRawJson">{{
              $st('Apply JSON')
            }}</Button>
          </div>
          <textarea
            v-model="rawJson"
            rows="13"
            spellcheck="false"
            class="w-full rounded-md border border-input bg-transparent p-3 font-mono text-xs outline-none focus:border-ring"
            :readonly="!canEdit"
          />
          <div v-if="rawJsonError" class="text-xs text-destructive">{{ rawJsonError }}</div>
          <div v-if="selectedSource === 'effective'" class="text-xs text-muted-foreground">
            {{ t('settings.tui.effectiveReadOnly') }}
          </div>
        </section>
      </div>

      <PermissionsPanel v-if="selectedSection === 'tools'" :scope="selectedSource" />
    </div>
  </section>
</template>
