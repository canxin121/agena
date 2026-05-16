import { computed, type ComputedRef, type Ref } from 'vue'
import type { Router } from 'vue-router'

import type { RuntimeStatus, SessionResource, WorkspaceResource } from '../lib/agenaApi'
import { createChatCommandCatalog, type ChatCommandCatalogActions } from '../lib/chatCommandCatalog'
import {
  commandSearchText,
  createCommandPalette,
  createCommandPaletteItems,
  type CommandItem,
} from '../lib/commandPalette'
import { useRegisteredCommandPaletteItems } from '../lib/commandPaletteRegistry'
import type { ChatUsageSummary } from './chatUsageModel'

export type ChatCommandStateInput = {
  routeRouter: Router
  runtime: Ref<RuntimeStatus | null>
  selectedWorkspaceId: Ref<number | null>
  selectedSessionId: Ref<number | null>
  sessions: Ref<SessionResource[]>
  workspaces: Ref<WorkspaceResource[]>
  sessionImportJsonl: Ref<string>
  sessionTreeRows: ComputedRef<Array<{ session: SessionResource; depth: number }>>
  rewindCheckpoints: Ref<Array<unknown>>
  ancestorSessions: ComputedRef<SessionResource[]>
  sessionUsageSummary: ComputedRef<ChatUsageSummary>
  composer: Ref<string>
  localCommandNotice: Ref<string>
  newSessionTitle: Ref<string>
  workspacePath: Ref<string>
  actions: Omit<ChatCommandCatalogActions, 'setLocalCommandNotice' | 'setNewSessionTitle' | 'setWorkspacePath'>
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
  }
}

function slashSuggestionScore(item: CommandItem, query: { slashNeedle: string; textNeedle: string }): number | null {
  const slash = (item.slash || '').toLowerCase()
  if (!slash) return null
  if (slash === query.slashNeedle) return 0
  if (slash.startsWith(query.slashNeedle)) return 1
  if (commandSearchText(item).includes(query.textNeedle)) return 2
  return null
}

export function useChatCommandState(input: ChatCommandStateInput) {
  const localCommands = computed(() =>
    createChatCommandCatalog(
      {
        selectedWorkspaceId: computed(() => input.selectedWorkspaceId.value),
        selectedSessionId: computed(() => input.selectedSessionId.value),
        sessions: computed(() => input.sessions.value),
        workspaces: computed(() => input.workspaces.value),
        sessionImportJsonl: computed(() => input.sessionImportJsonl.value),
        sessionTreeRows: input.sessionTreeRows,
        rewindCheckpoints: computed(() => input.rewindCheckpoints.value),
        ancestorSessions: input.ancestorSessions,
        sessionUsageSummary: input.sessionUsageSummary,
      },
      {
        ...input.actions,
        setLocalCommandNotice: (value) => {
          input.localCommandNotice.value = value
        },
        setNewSessionTitle: (value) => {
          input.newSessionTitle.value = value
        },
        setWorkspacePath: (value) => {
          input.workspacePath.value = value
        },
      },
    ),
  )

  const paletteCatalogInput = {
    router: input.routeRouter,
    runtimeSkills: computed(() => input.runtime.value?.operator.skills.skills ?? []),
    runtimeCommands: computed(() => input.runtime.value?.operator.skills.commands ?? []),
    localCommands,
    onSelectRuntimeEntry: async ({ kind, item }: { kind: 'skill' | 'command'; item: { name: string } }) => {
      const descriptor = kind === 'command' ? 'command' : 'skill'
      input.localCommandNotice.value = `Runtime ${descriptor} /${item.name} is available in the runtime catalog, but direct execution is not wired in Agena Web yet.`
    },
  }

  const commandItems = createCommandPaletteItems(paletteCatalogInput)
  const commandPalette = createCommandPalette(paletteCatalogInput)

  useRegisteredCommandPaletteItems(localCommands)

  const slashSuggestions = computed(() => {
    const query = slashQuery(input.composer.value)
    if (!query) return [] as CommandItem[]
    return commandItems.value
      .map((item) => ({ item, score: slashSuggestionScore(item, query) }))
      .filter((entry): entry is { item: CommandItem; score: number } => entry.score !== null)
      .sort((left, right) => {
        if (left.score !== right.score) return left.score - right.score
        const sourceDelta = sourcePriority(left.item) - sourcePriority(right.item)
        if (sourceDelta !== 0) return sourceDelta
        return left.item.title.localeCompare(right.item.title)
      })
      .map((entry) => entry.item)
      .slice(0, 10)
  })

  return {
    commandItems,
    commandPalette,
    localCommands,
    slashSuggestions,
  }
}
