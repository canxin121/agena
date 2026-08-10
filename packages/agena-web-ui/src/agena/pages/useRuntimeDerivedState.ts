import { computed, type Ref } from 'vue'

import type {
  MarketplaceInstalledPluginResource,
  MarketplacePluginResource,
  PermissionMode,
  PermissionRuleResource,
  PermissionSubjectKind,
  PluginInspect,
  RuntimeSkill,
  RuntimeStatus,
  SessionExecutionResource,
} from '../lib/agenaApi'
import {
  buildExecutionFacts,
  buildOperatorCards,
  buildRuntimeSnapshotFacts,
  buildSessionCacheFacts,
  type OperatorCard,
  type SessionExecutionFact,
} from './runtimePageModel'
import {
  filterMarketplacePluginsByQuery,
  filterRuntimeSkillsByQuery,
  queryMatchesText,
  pluginsTabs,
  resolveRuntimeRouteSection,
  sectionDescriptions,
  settingsTabs,
  sectionTitles,
  type RuntimeRouteSection,
  type SectionTabOption,
} from './runtimePageStateModel'

export type RuntimeDerivedStateInput = {
  lspQuery: Ref<string>
  marketplaceInstalled: Ref<MarketplaceInstalledPluginResource[]>
  marketplacePlugins: Ref<MarketplacePluginResource[]>
  marketplaceQuery: Ref<string>
  mcpQuery: Ref<string>
  permissionModeFilter: Ref<'all' | PermissionMode>
  permissionRules: Ref<PermissionRuleResource[]>
  permissionSearch: Ref<string>
  permissionScopeFilter: Ref<'all' | 'session' | 'workspace' | 'global'>
  permissionStatusFilter: Ref<'all' | 'active' | 'revoked'>
  permissionSubjectFilter: Ref<'all' | PermissionSubjectKind>
  routePath: Ref<string>
  runtime: Ref<RuntimeStatus | null>
  runtimeSkillQuery: Ref<string>
  section?: RuntimeRouteSection
  selectedPlugin: Ref<PluginInspect | null>
  sessionExecution: Ref<SessionExecutionResource | null>
  tabs: SectionTabOption[]
}

export function useRuntimeDerivedState(input: RuntimeDerivedStateInput) {
  const operatorCards = computed<OperatorCard[]>(() => buildOperatorCards(input.runtime.value))
  const runtimeSnapshotFacts = computed<SessionExecutionFact[]>(() => buildRuntimeSnapshotFacts(input.runtime.value))
  const sessionCacheFacts = computed<SessionExecutionFact[]>(() => buildSessionCacheFacts(input.runtime.value))
  const executionFacts = computed<SessionExecutionFact[]>(() => buildExecutionFacts(input.sessionExecution.value))
  const routeSection = computed<RuntimeRouteSection>(() =>
    resolveRuntimeRouteSection(input.routePath.value, input.section),
  )
  const pageTitle = computed(() => sectionTitles[routeSection.value])
  const pageDescription = computed(() => sectionDescriptions[routeSection.value])
  const visibleTabs = computed(() => {
    if (routeSection.value === 'runtime') return input.tabs
    if (routeSection.value === 'settings') return settingsTabs
    if (routeSection.value === 'plugins') return pluginsTabs
    return [] as SectionTabOption[]
  })
  const skillCommands = computed<RuntimeSkill[]>(() => input.runtime.value?.operator.skills.commands ?? [])
  const discoveredSkills = computed<RuntimeSkill[]>(() => input.runtime.value?.operator.skills.skills ?? [])
  const filteredSkillCommands = computed(() =>
    filterRuntimeSkillsByQuery(skillCommands.value, input.runtimeSkillQuery.value),
  )
  const filteredDiscoveredSkills = computed(() =>
    filterRuntimeSkillsByQuery(discoveredSkills.value, input.runtimeSkillQuery.value),
  )
  const filteredMcpServers = computed(() =>
    (input.runtime.value?.operator.mcp.servers ?? []).filter((server) =>
      queryMatchesText([server.name, String(server.tool_count)], input.mcpQuery.value),
    ),
  )
  const filteredLspServers = computed(() =>
    (input.runtime.value?.operator.lsp.servers ?? []).filter((server) =>
      queryMatchesText(
        [server.name, server.command, ...server.file_extensions, ...server.root_markers],
        input.lspQuery.value,
      ),
    ),
  )
  const filteredPermissionRules = computed(() => {
    return input.permissionRules.value.filter((rule) => {
      if (input.permissionModeFilter.value !== 'all' && rule.mode !== input.permissionModeFilter.value) return false
      if (input.permissionScopeFilter.value !== 'all' && rule.scope !== input.permissionScopeFilter.value) return false
      if (input.permissionSubjectFilter.value !== 'all' && rule.subject_kind !== input.permissionSubjectFilter.value)
        return false
      if (input.permissionStatusFilter.value === 'active' && rule.revoked_at) return false
      if (input.permissionStatusFilter.value === 'revoked' && !rule.revoked_at) return false
      const query = input.permissionSearch.value.trim().toLowerCase()
      if (
        query &&
        ![
          rule.action_key,
          rule.subject_kind,
          rule.tool_name || '',
          rule.qualifier || '',
          rule.path_access_kind || '',
          rule.workspace_root || '',
          rule.target_path || '',
          rule.network_target || '',
          rule.network_host || '',
          rule.scope,
          rule.mode,
          rule.source,
        ]
          .join(' ')
          .toLowerCase()
          .includes(query)
      )
        return false
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
  const installedMarketplacePluginIds = computed(
    () => new Set(input.marketplaceInstalled.value.map((plugin) => plugin.plugin_id)),
  )
  const selectedPluginManifest = computed(() => input.selectedPlugin.value?.manifest ?? null)

  return {
    discoveredSkills,
    executionFacts,
    filteredDiscoveredSkills,
    filteredLspServers,
    filteredMarketplacePlugins,
    filteredMcpServers,
    filteredPermissionRules,
    filteredSkillCommands,
    installedMarketplacePluginIds,
    operatorCards,
    pageDescription,
    pageTitle,
    routeSection,
    runtimeSnapshotFacts,
    selectedPluginManifest,
    sessionCacheFacts,
    skillCommands,
    visibleTabs,
  }
}
