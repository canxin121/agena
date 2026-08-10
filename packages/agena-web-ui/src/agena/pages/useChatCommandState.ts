import { computed, type ComputedRef, type Ref } from 'vue'
import type { Router } from 'vue-router'

import {
  invokePluginUiTool,
  runPluginUiAction,
  type RuntimeStatus,
  type SessionResource,
  type WorkspaceResource,
} from '../lib/agenaApi'
import { createChatCommandCatalog, type ChatCommandCatalogActions } from '../lib/chatCommandCatalog'
import {
  buildPluginCommandPayload,
  commandMatchesSlash,
  commandSearchText,
  createCommandPalette,
  createCommandPaletteItems,
  type CommandItem,
  type CommandPaletteCatalogInput,
  type CommandRunResult,
} from '../lib/commandPalette'
import { useRegisteredCommandPaletteItems } from '../lib/commandPaletteRegistry'
import { isPluginUiToolInvokeResponse, resolvePluginCommandOutput } from '../lib/pluginUiActionRuntime'
import type { NotificationsHandle } from '../lib/notifications/types'
import type { ChatUsageSummary } from './chatUsageModel'
import type { ComposerQueueItem } from './chatQueueModel'

export type ChatCommandStateInput = {
  routeRouter: Router
  runtime: Ref<RuntimeStatus | null>
  selectedWorkspaceId: Ref<number | null>
  selectedSessionId: Ref<number | null>
  sessions: Ref<SessionResource[]>
  messages: Ref<import('../lib/agenaApi').MessageResource[]>
  composerQueue: Ref<ComposerQueueItem[]>
  workspaces: Ref<WorkspaceResource[]>
  sessionImportJsonl: Ref<string>
  sessionTreeRows: ComputedRef<Array<{ session: SessionResource; depth: number }>>
  rewindCheckpoints: Ref<Array<unknown>>
  ancestorSessions: ComputedRef<SessionResource[]>
  childSessions: ComputedRef<SessionResource[]>
  parentSession: ComputedRef<SessionResource | null>
  sessionState: Ref<import('../lib/agenaApi').SessionExecutionResource | null>
  sessionUsageSummary: ComputedRef<ChatUsageSummary>
  composer: Ref<string>
  notify: NotificationsHandle
  newSessionTitle: Ref<string>
  workspacePath: Ref<string>
  sessionSearch: Ref<string>
  actions: Omit<
    ChatCommandCatalogActions,
    | 'setLocalCommandNotice'
    | 'setNewSessionTitle'
    | 'setWorkspacePath'
    | 'setSessionSearch'
    | 'runRuntimeEntry'
    | 'invokeRuntimeTool'
  >
}

function slashQuery(value: string): { slashNeedle: string; textNeedle: string } | null {
  const trimmed = value.trimStart()
  if (!trimmed.startsWith('/')) return null
  const parts = trimmed.split(/\s+/).filter(Boolean)
  const slashNeedle = (parts[0] || '').toLowerCase()
  const textNeedle = trimmed.slice(1).trim().toLowerCase()
  return { slashNeedle, textNeedle }
}

function sourcePriority(item: CommandItem): number {
  switch (item.source) {
    case 'chat-action':
      return 0
    case 'workspace-action':
      return 1
    case 'navigation':
      return 2
    case 'runtime-command':
      return 3
    case 'runtime-skill':
      return 4
    case 'plugin-studio':
      return 5
  }
}

function slashSuggestionScore(item: CommandItem, query: { slashNeedle: string; textNeedle: string }): number | null {
  const slashes = [item.slash, ...(item.slashAliases || [])].filter((value): value is string => Boolean(value))
  if (!slashes.length) return null
  if (commandMatchesSlash(item, query.slashNeedle)) return 0
  if (slashes.some((slash) => slash.toLowerCase().startsWith(query.slashNeedle))) return 1
  if (commandSearchText(item).includes(query.textNeedle)) return 2
  return null
}

export function buildSlashSuggestions(items: CommandItem[], composer: string, limit = 10): CommandItem[] {
  const query = slashQuery(composer)
  if (!query) return []
  return items
    .map((item) => ({
      item,
      score: slashSuggestionScore(item, query),
      featured: query.slashNeedle === '/' && item.id === 'chat.attach-skill' ? 0 : 1,
    }))
    .filter((entry): entry is { item: CommandItem; score: number; featured: number } => entry.score !== null)
    .sort((left, right) => {
      if (left.score !== right.score) return left.score - right.score
      if (left.featured !== right.featured) return left.featured - right.featured
      const sourceDelta = sourcePriority(left.item) - sourcePriority(right.item)
      if (sourceDelta !== 0) return sourceDelta
      return left.item.title.localeCompare(right.item.title)
    })
    .map((entry) => entry.item)
    .slice(0, Math.max(0, limit))
}

export function useChatCommandState(input: ChatCommandStateInput) {
  async function invokeRuntimeEntry(name: string, args: string): Promise<CommandRunResult> {
    const response = await invokePluginUiTool({
      tool: name,
      payload: { args: args.trim() || null },
      sessionId: input.selectedSessionId.value,
    })
    const prompt = response.output_text.trim()
    return prompt ? { submitText: prompt } : { notice: `Runtime entry /${name} returned an empty prompt.` }
  }

  async function invokeRuntimeTool(name: string, payload: Record<string, unknown>): Promise<CommandRunResult> {
    const response = await invokePluginUiTool({
      tool: name,
      payload,
      sessionId: input.selectedSessionId.value,
    })
    return { notice: response.output_text.trim() || `Ran runtime tool ${name}.` }
  }

  const localCommands = computed(() =>
    createChatCommandCatalog(
      {
        selectedWorkspaceId: computed(() => input.selectedWorkspaceId.value),
        selectedSessionId: computed(() => input.selectedSessionId.value),
        sessions: computed(() => input.sessions.value),
        messages: computed(() => input.messages.value),
        composerQueue: computed(() => input.composerQueue.value),
        workspaces: computed(() => input.workspaces.value),
        sessionImportJsonl: computed(() => input.sessionImportJsonl.value),
        sessionTreeRows: input.sessionTreeRows,
        rewindCheckpoints: computed(() => input.rewindCheckpoints.value),
        ancestorSessions: input.ancestorSessions,
        childSessions: input.childSessions,
        parentSession: input.parentSession,
        sessionState: computed(() => input.sessionState.value),
        sessionUsageSummary: input.sessionUsageSummary,
      },
      {
        ...input.actions,
        setLocalCommandNotice: (value) => input.notify.notice(value),
        setNewSessionTitle: (value) => {
          input.newSessionTitle.value = value
        },
        setWorkspacePath: (value) => {
          input.workspacePath.value = value
        },
        setSessionSearch: (value) => {
          input.sessionSearch.value = value
        },
        runRuntimeEntry: invokeRuntimeEntry,
        invokeRuntimeTool,
      },
    ),
  )

  const paletteCatalogInput: CommandPaletteCatalogInput = {
    router: input.routeRouter,
    runtimeSkills: computed(() => input.runtime.value?.operator.skills.skills ?? []),
    runtimeCommands: computed(() => input.runtime.value?.operator.skills.commands ?? []),
    pluginCommands: computed(() => input.runtime.value?.operator.ui?.catalog.studio.commands ?? []),
    localCommands,
    onSelectRuntimeEntry: async ({ context, item }) => {
      const response = await invokePluginUiTool({
        tool: item.name,
        payload: { args: context?.args.join(' ').trim() || null },
        sessionId: input.selectedSessionId.value,
      })
      const prompt = response.output_text.trim()
      if (!prompt) {
        input.notify.notice(`Runtime entry /${item.name} returned an empty prompt.`)
        return
      }
      return { submitText: prompt }
    },
    onRunPluginAction: async ({ command, context }) => {
      const action = command.action
      if (action.kind === 'submit_prompt') {
        return { submitText: action.prompt }
      }
      if (action.kind === 'invoke_tool') {
        const response = await runPluginUiAction({
          pluginId: command.plugin_id,
          actionId: command.id,
          payload: buildPluginCommandPayload(command, context),
          sessionId: input.selectedSessionId.value,
        })
        const output = isPluginUiToolInvokeResponse(response.result) ? response.result.output_text.trim() : ''
        if (action.submit_output_as_prompt && output) {
          return { submitText: output }
        }
        return { notice: output || `Ran plugin command ${command.title}.` }
      }
      if (action.kind === 'invoke_command') {
        const response = await runPluginUiAction({
          pluginId: command.plugin_id,
          actionId: command.id,
          payload: buildPluginCommandPayload(command, context),
          sessionId: input.selectedSessionId.value,
        })
        return await applyResolvedPluginCommandEffect(
          await resolvePluginCommandOutput({
            pluginId: command.plugin_id,
            result: response.result,
            sessionId: input.selectedSessionId.value,
            fallbackNotice: `Ran plugin command ${command.title}.`,
          }),
        )
      }
      if (action.kind === 'open_route') {
        await input.routeRouter.push(action.route)
        return
      }
      if (action.kind === 'open_url') {
        if (typeof window !== 'undefined') window.open(action.url, '_blank', 'noopener,noreferrer')
        return
      }
    },
  }

  async function applyResolvedPluginCommandEffect(effect: Awaited<ReturnType<typeof resolvePluginCommandOutput>>) {
    if (effect.kind === 'notice') {
      return { notice: effect.message }
    }
    if (effect.kind === 'submit_prompt') {
      return { submitText: effect.prompt }
    }
    if (effect.kind === 'open_route') {
      await input.routeRouter.push(effect.route)
      return
    }
    if (effect.kind === 'open_url') {
      if (typeof window !== 'undefined') window.open(effect.url, '_blank', 'noopener,noreferrer')
      return
    }
  }

  const commandItems = createCommandPaletteItems(paletteCatalogInput)
  const commandPalette = createCommandPalette(paletteCatalogInput)

  useRegisteredCommandPaletteItems(localCommands)

  const slashSuggestions = computed(() => {
    return buildSlashSuggestions(commandItems.value, input.composer.value)
  })

  return {
    commandItems,
    commandPalette,
    localCommands,
    slashSuggestions,
  }
}
