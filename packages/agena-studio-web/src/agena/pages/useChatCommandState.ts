import { computed, type ComputedRef, type Ref } from 'vue'
import type { Router } from 'vue-router'

import type { RuntimeStatus, SessionResource, WorkspaceResource } from '../lib/agenaApi'
import { createChatCommandCatalog, type ChatCommandCatalogActions } from '../lib/chatCommandCatalog'
import { createCommandPalette, createCommandPaletteItems, type CommandItem } from '../lib/commandPalette'
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
      input.localCommandNotice.value =
        `Runtime ${descriptor} /${item.name} is available in the runtime catalog, but direct execution is not wired in Agena Web yet.`
    },
  }

  const commandItems = createCommandPaletteItems(paletteCatalogInput)
  const commandPalette = createCommandPalette(paletteCatalogInput)

  useRegisteredCommandPaletteItems(localCommands)

  const slashSuggestions = computed(() => {
    const value = input.composer.value.trim()
    if (!value.startsWith('/')) return [] as CommandItem[]
    const prefix = value.toLowerCase()
    return commandItems.value.filter((item) => item.slash && item.slash.toLowerCase().startsWith(prefix)).slice(0, 8)
  })

  return {
    commandItems,
    commandPalette,
    localCommands,
    slashSuggestions,
  }
}
