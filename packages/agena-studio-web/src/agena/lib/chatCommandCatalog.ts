import type { ComputedRef } from 'vue'

import type { SessionResource, WorkspaceResource } from './agenaApi'
import type { CommandContext, CommandItem } from './commandPalette'
import {
  sectionTabNavigationItems,
  type PluginsTab,
  type RuntimeRouteSection,
  type RuntimeTab,
  type SettingsTab,
} from '../pages/runtimePageStateModel'
import { workspaceShortcuts, type WorkspaceShortcut } from './runtimeWorkspaceShortcuts'
import { chatUsageFacts, formatUsageCount } from '../pages/chatUsageModel'

export type ChatCommandCatalogState = {
  selectedWorkspaceId: ComputedRef<number | null>
  selectedSessionId: ComputedRef<number | null>
  sessions: ComputedRef<SessionResource[]>
  workspaces: ComputedRef<WorkspaceResource[]>
  sessionImportJsonl: ComputedRef<string>
  sessionTreeRows: ComputedRef<Array<{ session: SessionResource; depth: number }>>
  rewindCheckpoints: ComputedRef<Array<unknown>>
  ancestorSessions: ComputedRef<SessionResource[]>
  sessionUsageSummary: ComputedRef<ReturnType<typeof import('../pages/chatUsageModel').summarizeChatUsage>>
}

export type ChatCommandCatalogActions = {
  openWorkspaceBrowser: (relativePath?: string) => void
  openRuntimeSection: (section: RuntimeRouteSection, tab: RuntimeTab | SettingsTab | PluginsTab) => void
  openSessionById: (sessionId: number) => Promise<boolean>
  setNewSessionTitle: (value: string) => void
  createSessionAction: () => void | Promise<void>
  continueCurrentSession: () => void | Promise<void>
  forkCurrentSession: () => void | Promise<void>
  exportCurrentSession: () => void | Promise<void>
  importSessionFromJsonl: () => void | Promise<void>
  selectWorkspace: (workspaceId: number) => void | Promise<void>
  resolveWorkspaceAction: (createIfMissing: boolean) => void | Promise<void>
  setWorkspacePath: (value: string) => void
  showSessionGoalAction: () => void | Promise<void>
  setSessionGoalAction: (objective: string) => void | Promise<void>
  completeSessionGoalAction: () => void | Promise<void>
  clearSessionGoalAction: () => void | Promise<void>
  loadSessionTree: (rootId: number) => void | Promise<void>
  loadRewindCheckpoints: (sessionId: number) => void | Promise<void>
  setLocalCommandNotice: (value: string) => void
}

function readCommandArgument(context: CommandContext | undefined): string {
  return context?.args.join(' ').trim() || ''
}

type GoalCommandPlan =
  | { kind: 'show' }
  | { kind: 'set'; objective: string }
  | { kind: 'complete' }
  | { kind: 'clear' }
  | { kind: 'error'; message: string }

function readGoalCommandPlan(context: CommandContext | undefined): GoalCommandPlan {
  const args = context?.args ?? []
  if (!args.length) return { kind: 'show' }

  const first = (args[0] || '').toLowerCase()
  if (first === 'show' || first === 'status') return { kind: 'show' }
  if (first === 'done' || first === 'complete' || first === 'completed') return { kind: 'complete' }
  if (first === 'clear' || first === 'unset' || first === 'remove') return { kind: 'clear' }

  const objective = args.join(' ').trim()
  if (!objective) {
    return { kind: 'error', message: 'Usage: /goal <objective>' }
  }
  return { kind: 'set', objective }
}

function createWorkspaceShortcutCommand(shortcut: WorkspaceShortcut, actions: ChatCommandCatalogActions): CommandItem {
  return {
    id: `workspace-shortcut.${shortcut.id}`,
    title: `Open ${shortcut.title}`,
    description: shortcut.description,
    category: 'Workspace Actions',
    source: 'workspace-action',
    slash: `/open-${shortcut.id}`,
    usage: `/open-${shortcut.id}`,
    aliases: [shortcut.id, shortcut.relativePath],
    run: () => {
      actions.openWorkspaceBrowser(shortcut.relativePath)
      actions.setLocalCommandNotice(`Opened workspace path ${shortcut.relativePath}.`)
    },
  }
}

function createParameterizedChatCommands(
  state: ChatCommandCatalogState,
  actions: ChatCommandCatalogActions,
): CommandItem[] {
  return [
    {
      id: 'chat.create-session',
      title: 'Create Session',
      description: 'Create a new session in the currently selected workspace.',
      category: 'Chat Actions',
      source: 'chat-action',
      slash: '/new-session',
      usage: '/new-session [title]',
      aliases: ['new session', 'create session'],
      run: async (context) => {
        if (!state.selectedWorkspaceId.value) {
          actions.setLocalCommandNotice('Select a workspace before running /new-session.')
          return
        }
        const title = readCommandArgument(context)
        if (title) actions.setNewSessionTitle(title)
        await actions.createSessionAction()
      },
    },
    {
      id: 'chat.new-session',
      title: 'New Session',
      description: 'Create a new session in the currently selected workspace.',
      category: 'Chat Actions',
      source: 'chat-action',
      slash: '/new',
      usage: '/new [title]',
      aliases: ['new session', 'create session', 'fresh chat'],
      run: async (context) => {
        if (!state.selectedWorkspaceId.value) {
          actions.setLocalCommandNotice('Select a workspace before running /new.')
          return
        }
        const title = readCommandArgument(context)
        if (title) actions.setNewSessionTitle(title)
        await actions.createSessionAction()
      },
    },
    {
      id: 'chat.session-goal',
      title: 'Session Goal',
      description: 'Show, set, complete, or clear the active goal for the selected session.',
      category: 'Chat Actions',
      source: 'chat-action',
      slash: '/goal',
      usage: '/goal [objective]',
      aliases: ['objective', 'active goal'],
      run: async (context) => {
        if (!state.selectedSessionId.value) {
          actions.setLocalCommandNotice('Select a session before running /goal.')
          return
        }
        const plan = readGoalCommandPlan(context)
        if (plan.kind === 'error') {
          actions.setLocalCommandNotice(plan.message)
          return
        }
        if (plan.kind === 'show') {
          await actions.showSessionGoalAction()
          return
        }
        if (plan.kind === 'complete') {
          await actions.completeSessionGoalAction()
          return
        }
        if (plan.kind === 'clear') {
          await actions.clearSessionGoalAction()
          return
        }
        await actions.setSessionGoalAction(plan.objective)
      },
    },
    {
      id: 'chat.complete-session-goal',
      title: 'Complete Goal',
      description: 'Mark the active session goal as completed.',
      category: 'Chat Actions',
      source: 'chat-action',
      slash: '/goal-done',
      usage: '/goal-done',
      aliases: ['complete goal', 'finish goal'],
      run: async () => {
        if (!state.selectedSessionId.value) {
          actions.setLocalCommandNotice('Select a session before running /goal-done.')
          return
        }
        await actions.completeSessionGoalAction()
      },
    },
    {
      id: 'chat.clear-session-goal',
      title: 'Clear Goal',
      description: 'Remove the active goal from the selected session.',
      category: 'Chat Actions',
      source: 'chat-action',
      slash: '/goal-clear',
      usage: '/goal-clear',
      aliases: ['unset goal', 'remove goal'],
      run: async () => {
        if (!state.selectedSessionId.value) {
          actions.setLocalCommandNotice('Select a session before running /goal-clear.')
          return
        }
        await actions.clearSessionGoalAction()
      },
    },
    {
      id: 'chat.continue-run',
      title: 'Continue Run',
      description: 'Continue the selected session using the current provider and model selection.',
      category: 'Chat Actions',
      source: 'chat-action',
      slash: '/continue',
      usage: '/continue',
      aliases: ['resume run'],
      run: async () => {
        if (!state.selectedSessionId.value) return
        await actions.continueCurrentSession()
      },
    },
    {
      id: 'chat.fork-session',
      title: 'Fork Current Session',
      description: 'Fork the selected session at the latest message.',
      category: 'Chat Actions',
      source: 'chat-action',
      slash: '/fork',
      usage: '/fork [title]',
      aliases: ['branch session'],
      run: async (context) => {
        if (!state.selectedSessionId.value) return
        const title = readCommandArgument(context)
        if (title) actions.setNewSessionTitle(title)
        await actions.forkCurrentSession()
      },
    },
    {
      id: 'chat.export-session',
      title: 'Export Session',
      description: 'Export the active session into JSONL and place it in the transfer textarea.',
      category: 'Chat Actions',
      source: 'chat-action',
      slash: '/export-session',
      usage: '/export-session',
      aliases: ['session export'],
      run: async () => {
        if (!state.selectedSessionId.value) return
        await actions.exportCurrentSession()
      },
    },
    {
      id: 'chat.import-session',
      title: 'Import Session',
      description: 'Import the JSONL currently pasted into the session transfer textarea.',
      category: 'Chat Actions',
      source: 'chat-action',
      slash: '/import-session',
      usage: '/import-session',
      aliases: ['session import'],
      run: async () => {
        if (!state.sessionImportJsonl.value.trim()) {
          actions.setLocalCommandNotice('Paste session JSONL before running /import-session.')
          return
        }
        await actions.importSessionFromJsonl()
      },
    },
    {
      id: 'chat.open-session',
      title: 'Open Session by ID',
      description: 'Switch the active Chat session to a known session id.',
      category: 'Chat Actions',
      source: 'chat-action',
      slash: '/open-session',
      usage: '/open-session <session-id>',
      aliases: ['session id', 'jump session'],
      run: async (context) => {
        const value = readCommandArgument(context)
        const sessionId = Number(value)
        if (!Number.isFinite(sessionId)) {
          actions.setLocalCommandNotice('Usage: /open-session <session-id>')
          return
        }
        const matched = await actions.openSessionById(sessionId)
        actions.setLocalCommandNotice(
          matched ? `Opened session #${sessionId}.` : `Session #${sessionId} was not found.`,
        )
      },
    },
    {
      id: 'chat.open-workspace',
      title: 'Open Workspace by ID',
      description: 'Switch the active Chat sidebar workspace to a known workspace id.',
      category: 'Workspace Actions',
      source: 'workspace-action',
      slash: '/open-workspace',
      usage: '/open-workspace <workspace-id>',
      aliases: ['workspace id', 'switch workspace'],
      run: async (context) => {
        const value = readCommandArgument(context)
        const workspaceId = Number(value)
        if (!Number.isFinite(workspaceId)) {
          actions.setLocalCommandNotice('Usage: /open-workspace <workspace-id>')
          return
        }
        if (!state.workspaces.value.some((workspace) => workspace.id === workspaceId)) {
          actions.setLocalCommandNotice(`Workspace #${workspaceId} was not found.`)
          return
        }
        await actions.selectWorkspace(workspaceId)
        actions.setLocalCommandNotice(`Opened workspace #${workspaceId}.`)
      },
    },
    {
      id: 'workspace.resolve-workspace',
      title: 'Resolve Workspace Path',
      description: 'Resolve or create a workspace from a filesystem path.',
      category: 'Workspace Actions',
      source: 'workspace-action',
      slash: '/resolve-workspace',
      usage: '/resolve-workspace <path>',
      aliases: ['attach workspace', 'open repo'],
      run: async (context) => {
        const path = readCommandArgument(context)
        if (!path) {
          actions.setLocalCommandNotice('Usage: /resolve-workspace <path>')
          return
        }
        actions.setWorkspacePath(path)
        await actions.resolveWorkspaceAction(true)
      },
    },
    {
      id: 'workspace.open-path',
      title: 'Open Workspace Path',
      description: 'Jump from Chat into the Workspace page at a specific relative path.',
      category: 'Workspace Actions',
      source: 'workspace-action',
      slash: '/open-path',
      usage: '/open-path <relative-path>',
      aliases: ['browse path', 'workspace path'],
      run: (context) => {
        const relativePath = readCommandArgument(context)
        if (!relativePath) {
          actions.setLocalCommandNotice('Usage: /open-path <relative-path>')
          return
        }
        actions.openWorkspaceBrowser(relativePath)
        actions.setLocalCommandNotice(`Opened workspace path ${relativePath}.`)
      },
    },
    {
      id: 'chat.show-cost',
      title: 'Show Session Cost',
      description: 'Summarize token and cost usage for the active session.',
      category: 'Chat Actions',
      source: 'chat-action',
      slash: '/cost',
      usage: '/cost',
      aliases: ['usage', 'tokens', 'session cost'],
      run: () => {
        if (!state.selectedSessionId.value || !state.sessionUsageSummary.value.runs) {
          actions.setLocalCommandNotice('No assistant usage has been recorded for the active session yet.')
          return
        }
        const topModel = state.sessionUsageSummary.value.byModel[0]
        const modelLabel = topModel ? ` · top_model=${topModel.providerId}/${topModel.modelId}` : ''
        actions.setLocalCommandNotice(
          `Session usage: ${chatUsageFacts(state.sessionUsageSummary.value).join(' · ')}${modelLabel}`,
        )
      },
    },
    {
      id: 'chat.show-tree',
      title: 'Show Session Tree',
      description: 'Summarize branch and lineage coverage for the active root session.',
      category: 'Chat Actions',
      source: 'chat-action',
      slash: '/tree',
      usage: '/tree',
      aliases: ['session tree', 'branches'],
      run: async () => {
        const rootId = state.ancestorSessions.value[0]?.id ?? state.selectedSessionId.value
        if (!rootId) return
        await actions.loadSessionTree(rootId)
        actions.setLocalCommandNotice(
          state.sessionTreeRows.value.length
            ? `Loaded ${formatUsageCount(state.sessionTreeRows.value.length)} session tree nodes from root #${rootId}.`
            : `No session tree nodes were returned for root #${rootId}.`,
        )
      },
    },
    {
      id: 'chat.show-rewind-checkpoints',
      title: 'Show Rewind Checkpoints',
      description: 'Load rewind checkpoints for the active session.',
      category: 'Chat Actions',
      source: 'chat-action',
      slash: '/checkpoints',
      usage: '/checkpoints',
      aliases: ['rewind checkpoints', 'rewind history'],
      run: async () => {
        if (!state.selectedSessionId.value) return
        await actions.loadRewindCheckpoints(state.selectedSessionId.value)
        actions.setLocalCommandNotice(
          state.rewindCheckpoints.value.length
            ? `Loaded ${formatUsageCount(state.rewindCheckpoints.value.length)} rewind checkpoints for session #${state.selectedSessionId.value}.`
            : `No rewind checkpoints are available for session #${state.selectedSessionId.value}.`,
        )
      },
    },
    ...sectionTabNavigationItems
      .filter((item) => Boolean(item.shortcutSlash))
      .map((item) => ({
        id: item.id.replace('nav.', 'chat.'),
        title: item.title,
        description: item.description,
        category: 'Navigation',
        source: 'navigation' as const,
        slash: item.shortcutSlash,
        usage: item.shortcutSlash,
        aliases: item.aliases,
        run: () => actions.openRuntimeSection(item.section, item.tab),
      })),
  ]
}

export function createChatCommandCatalog(
  state: ChatCommandCatalogState,
  actions: ChatCommandCatalogActions,
): CommandItem[] {
  const shortcutCommands = workspaceShortcuts.map((shortcut) => createWorkspaceShortcutCommand(shortcut, actions))
  return [...createParameterizedChatCommands(state, actions), ...shortcutCommands]
}
