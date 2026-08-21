<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiAddLine, RiDeleteBinLine, RiRefreshLine, RiSave3Line } from '@remixicon/vue'

import Button from '@/components/ui/Button.vue'
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
type PathRule = AccessModes | string
type PermissionConfig = {
  path?: { workspace?: AccessModes; external?: AccessModes; rules?: Record<string, PathRule> }
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

const SHELL_CAPABLE_TOOLS = ['agena.shell.run'] as const

const { t } = useI18n()
const chat = useChatStore()
const toasts = useToastsStore()

const loading = ref(false)
const saving = ref(false)
const error = ref('')
const selectedSource = ref<PermissionSource>('global')
const selectedSection = ref<PermissionSection>('path')
const config = ref<PermissionConfig>({})
const globalConfig = ref<PermissionConfig>({})
const workspaceConfig = ref<PermissionConfig>({})
const sessionConfigSnapshot = ref<PermissionConfig>({})
const effectiveConfig = ref<PermissionConfig>({})
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

const activeSessionId = computed(() => {
  const value = Number(chat.selectedSessionId)
  return Number.isSafeInteger(value) && value > 0 ? value : null
})
const canEdit = computed(
  () => selectedSource.value !== 'effective' && (selectedSource.value !== 'session' || Boolean(activeSessionId.value)),
)
function permissionSummary(value: PermissionConfig): string {
  const pathDefaults = ['workspace', 'external'].reduce((total, scope) => {
    const modes = value.path?.[scope as 'workspace' | 'external']
    return total + Number(Boolean(modes?.read)) + Number(Boolean(modes?.write))
  }, 0)
  const pathRules = Object.keys(value.path?.rules || {}).length
  const networkDefaults = ['internet', 'private', 'loopback'].filter((zone) =>
    Boolean(value.network?.[zone as 'internet' | 'private' | 'loopback']),
  ).length
  const networkRules = Object.keys(value.network?.rules || {}).length
  const names = Object.keys(value.tools?.names || {}).length
  const commands = Object.values(value.tools?.rules || {}).reduce((total, rules) => {
    if (isMode(rules)) return total + 1
    return total + (rules && typeof rules === 'object' && !Array.isArray(rules) ? Object.keys(rules).length : 0)
  }, 0)
  const parts = [
    pathDefaults || pathRules
      ? st('filesystem {pathDefaults} defaults / {pathRules} rules', {
          pathDefaults: pathDefaults,
          pathRules: pathRules,
        })
      : '',
    networkDefaults || networkRules
      ? st('network {networkDefaults} defaults / {networkRules} rules', {
          networkDefaults: networkDefaults,
          networkRules: networkRules,
        })
      : '',
    value.tools?.default || names || commands
      ? value.tools?.default
        ? st('tools default + {names} names / {commands} commands', { names, commands })
        : st('tools {names} names / {commands} commands', { names, commands })
      : '',
    value.approval_model ? st('approval model') : '',
  ].filter(Boolean)
  return parts.join(' · ') || 'No overrides'
}

const sourceOptions = computed(() => [
  {
    value: 'global',
    label: st('Global Permission'),
    description: st('Baseline for all sessions.'),
    summary: permissionSummary(globalConfig.value),
  },
  {
    value: 'workspace',
    label: st('Workspace Permission'),
    description: st('Overrides for this workspace.'),
    summary: permissionSummary(workspaceConfig.value),
  },
  {
    value: 'session',
    label: st('Current Session Permission'),
    description: st('Applies only to the selected session.'),
    summary: activeSessionId.value ? permissionSummary(sessionConfigSnapshot.value) : st('No active session'),
  },
  {
    value: 'effective',
    label: st('Effective Permission'),
    description: st('Read-only merged policy.'),
    summary: permissionSummary(effectiveConfig.value),
  },
])
const sectionOptions = [
  { value: 'path', label: st('Filesystem'), description: st('Path defaults and path rules.') },
  { value: 'network', label: st('Network'), description: st('Network zones and domain rules.') },
  { value: 'tools', label: st('Tool Access'), description: st('Tool names and command rules.') },
]
const modeOptions = [
  { value: 'allow', label: st('Allow'), description: st('Always permit matching access.') },
  { value: 'auto', label: st('Auto'), description: st('Let the approval model decide.') },
  { value: 'ask', label: st('Ask'), description: st('Ask before matching access.') },
  { value: 'deny', label: st('Deny'), description: st('Always block matching access.') },
]
const shellToolOptions = SHELL_CAPABLE_TOOLS.map((value) => ({ value, label: value }))

const pathRules = computed(() => Object.entries(config.value.path?.rules || {}))
const networkRules = computed(() => Object.entries(config.value.network?.rules || {}))
const toolNameRules = computed(() => Object.entries(config.value.tools?.names || {}))
const commandRules = computed(() => {
  const rows: Array<{ tool: string; command: string; mode: PermissionMode }> = []
  for (const [tool, rawRules] of Object.entries(config.value.tools?.rules || {})) {
    if (!isShellCapableTool(tool)) continue
    if (isMode(rawRules)) {
      rows.push({ tool, command: '*', mode: rawRules })
      continue
    }
    if (!rawRules || typeof rawRules !== 'object' || Array.isArray(rawRules)) continue
    for (const [command, mode] of Object.entries(rawRules)) {
      if (isMode(mode)) rows.push({ tool, command, mode })
    }
  }
  return rows
})

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
    if (!next.path.workspace && !next.path.external && !Object.keys(next.path.rules || {}).length) delete next.path
  })
}

function pathRuleMode(rule: PathRule, access: 'read' | 'write'): PermissionMode | '' {
  if (typeof rule === 'string') return pathRuleShorthandModes(rule)[access] || ''
  return rule?.[access] || ''
}

function pathRuleShorthandModes(value: string): AccessModes {
  const normalized = value.trim().toLowerCase().replaceAll('-', '_')
  if (normalized === 'allow' || normalized === 'read_write' || normalized === 'rw') {
    return { read: 'allow', write: 'allow' }
  }
  if (normalized === 'auto') return { read: 'auto', write: 'auto' }
  if (normalized === 'ask') return { read: 'ask', write: 'ask' }
  if (normalized === 'deny' || normalized === 'none') return { read: 'deny', write: 'deny' }
  if (normalized === 'read' || normalized === 'read_only' || normalized === 'ro') {
    return { read: 'allow', write: 'deny' }
  }
  if (normalized === 'write' || normalized === 'write_only' || normalized === 'wo') {
    return { read: 'deny', write: 'allow' }
  }
  return {}
}

function setPathRuleMode(path: string, access: 'read' | 'write', value: string) {
  mutate((next) => {
    next.path ||= {}
    next.path.rules ||= {}
    const current = next.path.rules[path]
    const modes: AccessModes =
      typeof current === 'object' && current ? { ...current } : pathRuleShorthandModes(String(current || ''))
    if (isMode(value)) modes[access] = value
    else delete modes[access]
    next.path.rules[path] = modes
  })
}

function removePathRule(path: string) {
  mutate((next) => {
    if (next.path?.rules) delete next.path.rules[path]
  })
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
  })
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
  })
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
  if (!isShellCapableTool(tool)) return
  mutate((next) => {
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
    next.tools.rules[tool] = rules
  })
}

function removeCommandRule(tool: string, command: string) {
  mutate((next) => {
    const raw = next.tools?.rules?.[tool]
    if (isMode(raw)) {
      if (command === '*') delete next.tools!.rules![tool]
      return
    }
    if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return
    const rules = { ...raw } as Record<string, PermissionMode>
    delete rules[command]
    if (Object.keys(rules).length) next.tools!.rules![tool] = rules
    else delete next.tools!.rules![tool]
  })
}

function renameCommandRule(tool: string, command: string, event: Event) {
  const input = event.target as HTMLInputElement
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
  try {
    const [effective, global, workspace, toolResponse] = await Promise.all([
      apiJson<SettingsResponse>('/api/v1/settings?source=effective&path=permission'),
      apiJson<SettingsResponse>('/api/v1/settings/layers/global?path=permission'),
      apiJson<SettingsResponse>('/api/v1/settings/layers/workspace?path=permission'),
      apiJson<ToolCatalogResponse>('/api/v1/plugins/surface'),
    ])
    globalConfig.value = asConfig(global?.value)
    workspaceConfig.value = asConfig(workspace?.value)
    effectiveConfig.value = asConfig(effective?.value)
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
    sessionConfigSnapshot.value = sessionConfig
    if (Object.keys(sessionEffective).length) effectiveConfig.value = sessionEffective
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
    if (selectedSource.value === 'effective') effectiveConfig.value = asConfig(sourceValue)
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
        scope: sourceOptions.value.find((item) => item.value === selectedSource.value)?.label || st('Permission'),
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
  <section class="grid gap-4 rounded-lg border border-border/60 bg-background/30 p-4 lg:p-5">
    <div class="flex flex-wrap items-start justify-between gap-3">
      <div>
        <h2 class="text-base font-medium">{{ $st('Permission Studio') }}</h2>
        <p class="mt-1 text-xs text-muted-foreground">
          {{
            $st(
              'The web editor mirrors the TUI hierarchy: source scope first, then Filesystem, Network, and Tool Access.',
            )
          }}
        </p>
      </div>
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

    <div class="grid gap-4 lg:grid-cols-[minmax(13rem,0.7fr)_minmax(0,2fr)]">
      <div class="grid content-start gap-2">
        <div class="text-xs font-semibold uppercase tracking-[0.14em] text-muted-foreground">
          {{ $st('Permission source') }}
        </div>
        <button
          v-for="option in sourceOptions"
          :key="option.value"
          type="button"
          class="rounded-md border px-3 py-2 text-left text-xs"
          :class="
            selectedSource === option.value ? 'border-primary bg-primary/10' : 'border-border/60 hover:bg-muted/40'
          "
          @click="selectedSource = option.value as PermissionSource"
        >
          <span class="block font-medium">{{ option.label }}</span>
          <span class="mt-1 block text-[10px] text-muted-foreground">{{ option.description }}</span>
          <span class="mt-1.5 block text-[10px] leading-4 text-foreground/70">{{ option.summary }}</span>
        </button>
        <div
          v-if="selectedSource === 'session' && !activeSessionId"
          class="rounded-md border border-dashed border-border/60 px-3 py-3 text-xs text-muted-foreground"
        >
          {{ $st('Open a session to edit current-session permission.') }}
        </div>
      </div>

      <div class="grid min-w-0 gap-4">
        <div class="flex flex-wrap items-center justify-between gap-2 border-b border-border/60 pb-3">
          <div class="flex flex-wrap gap-1">
            <Button
              v-for="section in sectionOptions"
              :key="section.value"
              :variant="selectedSection === section.value ? 'default' : 'ghost'"
              size="sm"
              @click="selectedSection = section.value as PermissionSection"
              >{{ section.label }}</Button
            >
          </div>
          <div class="flex gap-2">
            <Button v-if="canEdit" variant="ghost" size="sm" :disabled="saving" @click="clearSource"
              ><RiDeleteBinLine class="mr-1.5 h-4 w-4" /> {{ $st('Clear source') }}</Button
            >
            <Button v-if="canEdit" size="sm" :disabled="saving" @click="save"
              ><RiSave3Line class="mr-1.5 h-4 w-4" /> {{ saving ? $st('Saving…') : $st('Save permission') }}</Button
            >
          </div>
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
              <div
                v-for="[path, rule] in pathRules"
                :key="path"
                class="grid gap-2 rounded-md border border-border/60 p-3 sm:grid-cols-[minmax(0,1.5fr)_minmax(8rem,1fr)_minmax(8rem,1fr)_auto] sm:items-end"
              >
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
                v-for="[target, mode] in networkRules"
                :key="target"
                class="grid gap-2 rounded-md border border-border/60 p-3 sm:grid-cols-[minmax(0,1fr)_minmax(8rem,0.5fr)_auto] sm:items-center"
              >
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
            <div class="grid gap-2">
              <div class="text-sm font-medium">{{ $st('Name Rules') }}</div>
              <div
                v-for="[name, mode] in toolNameRules"
                :key="name"
                class="grid gap-2 rounded-md border border-border/60 p-3 sm:grid-cols-[minmax(0,1fr)_minmax(8rem,0.5fr)_auto] sm:items-center"
              >
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
              <div
                v-for="row in commandRules"
                :key="`${row.tool}:${row.command}`"
                class="grid gap-2 rounded-md border border-border/60 p-3 sm:grid-cols-[minmax(8rem,0.6fr)_minmax(0,1.4fr)_minmax(8rem,0.5fr)_auto] sm:items-center"
              >
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
    </div>
  </section>
</template>
