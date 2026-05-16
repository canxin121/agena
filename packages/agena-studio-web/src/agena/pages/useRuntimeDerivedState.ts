import { computed, type Ref } from 'vue'

import type {
  GlobalEventRecord,
  MarketplaceInstalledPluginResource,
  MarketplacePluginResource,
  PermissionMode,
  PermissionRuleResource,
  PluginInspect,
  RuntimeSkill,
  RuntimeStatus,
  SessionExecutionResource,
  TimelineEventRecord,
} from '../lib/agenaApi'
import { isDesktopRuntime, type DesktopBackendStatus, type DesktopConfig, type DesktopRuntimeInfo, type DesktopUpdateProgress } from '../../lib/desktopConfig'
import { buildDesktopConfigFacts, buildDesktopRuntimeFacts, buildDesktopStatusFacts, buildDesktopUpdateFacts } from './runtimeDesktopModel'
import { buildExecutionFacts, buildOperatorCards, buildRuntimeSnapshotFacts, buildSessionCacheFacts, buildTimelineSummary, type OperatorCard, type SessionExecutionFact } from './runtimePageModel'
import {
  filterMarketplacePluginsByQuery,
  filterRuntimeSkillsByQuery,
  queryMatchesText,
  resolveRuntimeRouteSection,
  sectionDescriptions,
  sectionTitles,
  type RuntimeRouteSection,
  type SectionTabOption,
} from './runtimePageStateModel'

export type RuntimeDerivedStateInput = {
  desktopConfig: Ref<DesktopConfig | null>
  desktopRuntimeState: Ref<DesktopRuntimeInfo | null>
  desktopStatus: Ref<DesktopBackendStatus | null>
  desktopUpdate: Ref<DesktopUpdateProgress | null>
  lspQuery: Ref<string>
  marketplaceInstalled: Ref<MarketplaceInstalledPluginResource[]>
  marketplacePlugins: Ref<MarketplacePluginResource[]>
  marketplaceQuery: Ref<string>
  mcpQuery: Ref<string>
  permissionModeFilter: Ref<'all' | PermissionMode>
  permissionRules: Ref<PermissionRuleResource[]>
  permissionScopeFilter: Ref<'all' | 'session' | 'workspace' | 'global'>
  permissionStatusFilter: Ref<'all' | 'active' | 'revoked'>
  permissionSubjectFilter: Ref<'all' | 'tool' | 'path_access'>
  routePath: Ref<string>
  runtime: Ref<RuntimeStatus | null>
  runtimeSkillQuery: Ref<string>
  section?: RuntimeRouteSection
  selectedPlugin: Ref<PluginInspect | null>
  globalEvents: Ref<GlobalEventRecord[]>
  sessionExecution: Ref<SessionExecutionResource | null>
  sessionTimeline: Ref<TimelineEventRecord[]>
  tabs: SectionTabOption[]
}

export function useRuntimeDerivedState(input: RuntimeDerivedStateInput) {
  const operatorCards = computed<OperatorCard[]>(() => buildOperatorCards(input.runtime.value))
  const runtimeSnapshotFacts = computed<SessionExecutionFact[]>(() => buildRuntimeSnapshotFacts(input.runtime.value))
  const sessionCacheFacts = computed<SessionExecutionFact[]>(() => buildSessionCacheFacts(input.runtime.value))
  const executionFacts = computed<SessionExecutionFact[]>(() => buildExecutionFacts(input.sessionExecution.value))
  const timelineSummaries = computed(() => buildTimelineSummary(input.sessionTimeline.value))
  const globalEventSummaries = computed(() => buildTimelineSummary(input.globalEvents.value))
  const desktopEnabled = computed(() => isDesktopRuntime())
  const desktopConfigFacts = computed(() => buildDesktopConfigFacts(input.desktopConfig.value))
  const desktopStatusFacts = computed(() => buildDesktopStatusFacts(input.desktopStatus.value))
  const desktopRuntimeFacts = computed(() => buildDesktopRuntimeFacts(input.desktopRuntimeState.value))
  const desktopUpdateFacts = computed(() => buildDesktopUpdateFacts(input.desktopUpdate.value))
  const desktopBackendUrl = computed(() => input.desktopStatus.value?.url?.trim() || '')
  const desktopBackendErrorFacts = computed(() => {
    const info = input.desktopStatus.value?.last_error_info
    if (!info) return [] as Array<{ label: string; value: string; mono?: boolean }>
    return [
      { label: 'Code', value: info.code, mono: true },
      { label: 'Summary', value: info.summary },
      { label: 'Detail', value: info.detail || 'n/a' },
      { label: 'Hint', value: info.hint || 'n/a' },
      { label: 'Exit Code', value: info.exitCode != null ? String(info.exitCode) : 'n/a', mono: true },
      { label: 'Signal', value: info.signal != null ? String(info.signal) : 'n/a', mono: true },
    ]
  })
  const desktopUpdateProgressPercent = computed(() => {
    const total = input.desktopUpdate.value?.totalBytes ?? null
    const downloaded = input.desktopUpdate.value?.downloadedBytes ?? 0
    if (!total || total <= 0) return ''
    return `${Math.max(0, Math.min(100, Math.round((downloaded / total) * 100)))}%`
  })
  const routeSection = computed<RuntimeRouteSection>(() => resolveRuntimeRouteSection(input.routePath.value, input.section))
  const pageTitle = computed(() => sectionTitles[routeSection.value])
  const pageDescription = computed(() => sectionDescriptions[routeSection.value])
  const visibleTabs = computed(() => {
    if (routeSection.value === 'runtime') return input.tabs
    return [] as SectionTabOption[]
  })
  const skillCommands = computed<RuntimeSkill[]>(() => input.runtime.value?.operator.skills.commands ?? [])
  const discoveredSkills = computed<RuntimeSkill[]>(() => input.runtime.value?.operator.skills.skills ?? [])
  const filteredSkillCommands = computed(() => filterRuntimeSkillsByQuery(skillCommands.value, input.runtimeSkillQuery.value))
  const filteredDiscoveredSkills = computed(() => filterRuntimeSkillsByQuery(discoveredSkills.value, input.runtimeSkillQuery.value))
  const filteredMcpServers = computed(() =>
    (input.runtime.value?.operator.mcp.servers ?? []).filter((server) =>
      queryMatchesText([server.name, String(server.tool_count)], input.mcpQuery.value),
    ),
  )
  const filteredLspServers = computed(() =>
    (input.runtime.value?.operator.lsp.servers ?? []).filter((server) =>
      queryMatchesText([server.name, server.command, ...server.file_extensions, ...server.root_markers], input.lspQuery.value),
    ),
  )
  const filteredPermissionRules = computed(() => {
    return input.permissionRules.value.filter((rule) => {
      if (input.permissionModeFilter.value !== 'all' && rule.mode !== input.permissionModeFilter.value) return false
      if (input.permissionScopeFilter.value !== 'all' && rule.scope !== input.permissionScopeFilter.value) return false
      if (input.permissionSubjectFilter.value !== 'all' && rule.subject_kind !== input.permissionSubjectFilter.value) return false
      if (input.permissionStatusFilter.value === 'active' && rule.revoked_at) return false
      if (input.permissionStatusFilter.value === 'revoked' && !rule.revoked_at) return false
      return true
    })
  })
  const filteredMarketplacePlugins = computed(() =>
    filterMarketplacePluginsByQuery(
      input.marketplacePlugins.value,
      input.marketplaceInstalled.value,
      input.marketplaceQuery.value,
    ),
  )
  const installedMarketplacePluginIds = computed(() => new Set(input.marketplaceInstalled.value.map((plugin) => plugin.plugin_id)))
  const selectedPluginManifest = computed(() => input.selectedPlugin.value?.manifest ?? null)

  return {
    desktopBackendErrorFacts,
    desktopBackendUrl,
    desktopConfigFacts,
    desktopEnabled,
    desktopRuntimeFacts,
    desktopStatusFacts,
    desktopUpdateFacts,
    desktopUpdateProgressPercent,
    discoveredSkills,
    executionFacts,
    filteredDiscoveredSkills,
    filteredLspServers,
    filteredMarketplacePlugins,
    filteredMcpServers,
    filteredPermissionRules,
    filteredSkillCommands,
    globalEventSummaries,
    installedMarketplacePluginIds,
    operatorCards,
    pageDescription,
    pageTitle,
    routeSection,
    runtimeSnapshotFacts,
    selectedPluginManifest,
    sessionCacheFacts,
    skillCommands,
    timelineSummaries,
    visibleTabs,
  }
}
