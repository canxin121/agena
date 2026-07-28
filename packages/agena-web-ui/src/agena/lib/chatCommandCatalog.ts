import type { ComputedRef } from 'vue'

import type { DomainEventRecord, SessionExecutionResource, SessionResource, WorkspaceResource } from './agenaApi'
import type { CommandContext, CommandItem, CommandRunResult } from './commandPalette'
import {
  sectionTabNavigationItems,
  type PluginsTab,
  type RuntimeRouteSection,
  type RuntimeTab,
  type SettingsTab,
} from '../pages/runtimePageStateModel'
import { workspaceShortcuts, type WorkspaceShortcut } from './runtimeWorkspaceShortcuts'
import { chatUsageFacts, formatUsageCount } from '../pages/chatUsageModel'
import { messageBlocks } from '../pages/chatRenderModel'
import { composerQueuePreview, type ComposerQueueItem } from '../pages/chatQueueModel'
import { pendingPermissionRequests, pendingUserInputRequests } from './agenaApi'

export type ChatCommandCatalogState = {
  selectedWorkspaceId: ComputedRef<number | null>
  selectedSessionId: ComputedRef<number | null>
  sessions: ComputedRef<SessionResource[]>
  messages: ComputedRef<import('./agenaApi').MessageResource[]>
  composerQueue: ComputedRef<ComposerQueueItem[]>
  timelineEvents: ComputedRef<DomainEventRecord[]>
  workspaces: ComputedRef<WorkspaceResource[]>
  sessionImportJsonl: ComputedRef<string>
  sessionTreeRows: ComputedRef<Array<{ session: SessionResource; depth: number }>>
  rewindCheckpoints: ComputedRef<Array<unknown>>
  ancestorSessions: ComputedRef<SessionResource[]>
  childSessions: ComputedRef<SessionResource[]>
  parentSession: ComputedRef<SessionResource | null>
  sessionState: ComputedRef<SessionExecutionResource | null>
  sessionUsageSummary: ComputedRef<ReturnType<typeof import('../pages/chatUsageModel').summarizeChatUsage>>
}

export type ChatCommandCatalogActions = {
  openWorkspaceBrowser: (relativePath?: string) => void
  openRuntimeSection: (section: RuntimeRouteSection, tab: RuntimeTab | SettingsTab | PluginsTab) => void
  openSessionById: (sessionId: number) => Promise<boolean>
  setNewSessionTitle: (value: string) => void
  createSessionAction: () => void | Promise<void>
  continueCurrentSession: () => void | Promise<void>
  compactCurrentSession: () => void | Promise<void>
  forkCurrentSession: () => void | Promise<void>
  exportCurrentSession: (requestedPath?: string) => void | Promise<void>
  importSessionFromJsonl: () => void | Promise<void>
  selectWorkspace: (workspaceId: number) => void | Promise<void>
  resolveWorkspaceAction: (createIfMissing: boolean) => void | Promise<void>
  setWorkspacePath: (value: string) => void
  setSessionSearch: (value: string) => void
  setSessionViewMode: (mode: 'all' | 'roots' | 'subtree', query?: string) => void | Promise<void>
  openCommandPalette: () => void
  openAttachmentPicker: (imageOnly?: boolean) => void
  openMemorySettings: (name?: string) => void
  openPermissionSettings: (mode?: string) => void
  openSnapshotInspector: () => void
  forgetMemory: (name: string) => void | Promise<void>
  focusComposer: () => void
  focusTranscript: () => void
  focusRunOptions: () => void
  runRuntimeEntry: (name: string, args: string) => void | CommandRunResult | Promise<void | CommandRunResult>
  invokeRuntimeTool: (
    name: string,
    payload: Record<string, unknown>,
  ) => void | CommandRunResult | Promise<void | CommandRunResult>
  showSessionGoalAction: () => void | Promise<void>
  setSessionGoalAction: (objective: string) => void | Promise<void>
  completeSessionGoalAction: () => void | Promise<void>
  clearSessionGoalAction: () => void | Promise<void>
  clearComposerQueue: () => void
  loadSessionTree: (rootId: number) => void | Promise<void>
  loadRewindCheckpoints: (sessionId: number) => void | Promise<void>
  refreshConversation: (foreground: boolean) => void | Promise<void>
  loadSessionTimeline: (limit: number) => void | Promise<void>
  renameCurrentSession: (title?: string) => void | Promise<void>
  selectSession: (sessionId: number) => void | Promise<void>
  popComposerQueue: () => void
  approvePermission: (
    requestId: string,
    kind: 'allow_once' | 'allow_always' | 'deny_once' | 'deny_always',
    scope?: 'session' | 'workspace' | 'global',
  ) => void | Promise<void>
  askAside: (question: string) => void | Promise<void>
  copyText: (text: string, successMessage: string) => void | Promise<void>
  downloadWorkspaceFile: (path: string) => void | Promise<void>
  createCommit: (message: string) => void | Promise<void>
  createPullRequest: (input: { title: string; body?: string; base?: string; head?: string }) => void | Promise<void>
  setLocalCommandNotice: (value: string) => void
}

function readCommandArgument(context: CommandContext | undefined): string {
  return context?.args.join(' ').trim() || ''
}

export type PullRequestCommandPlan =
  { kind: 'create'; title: string; body?: string; base?: string; head?: string } | { kind: 'error'; message: string }

function unquoteCommandValue(value: string): string {
  const normalized = value.trim()
  if (normalized.length >= 2) {
    const first = normalized[0]
    const last = normalized.at(-1)
    if ((first === '"' && last === '"') || (first === "'" && last === "'")) {
      return normalized.slice(1, -1)
    }
  }
  return normalized
}

export function parsePullRequestCommandArgs(args: string[]): PullRequestCommandPlan {
  const optionNames = new Set(['--body', '--base', '--head'])
  const firstOption = args.findIndex((token) => token.startsWith('--'))
  const titleTokens = firstOption < 0 ? args : args.slice(0, firstOption)
  const title = unquoteCommandValue(titleTokens.join(' '))
  if (!title)
    return { kind: 'error', message: 'Usage: /pr <title> [--body <text>] [--base <branch>] [--head <branch>]' }

  const values: { body?: string; base?: string; head?: string } = {}
  let index = firstOption < 0 ? args.length : firstOption
  while (index < args.length) {
    const option = args[index]
    if (!optionNames.has(option)) return { kind: 'error', message: `Unknown /pr option: ${option}` }
    index += 1
    const start = index
    while (index < args.length && !args[index]?.startsWith('--')) index += 1
    const value = unquoteCommandValue(args.slice(start, index).join(' '))
    if (!value) return { kind: 'error', message: `Missing value for ${option}` }
    if (option === '--body') values.body = value
    if (option === '--base') values.base = value
    if (option === '--head') values.head = value
  }
  return { kind: 'create', title, ...values }
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
      id: 'chat.help',
      title: 'Command Help',
      description: 'Open the searchable command palette with local, runtime, skill, and plugin commands.',
      category: 'Chat Actions',
      source: 'chat-action',
      slash: '/help',
      slashAliases: ['/?', '/commands', '/palette'],
      usage: '/help',
      aliases: ['command list', 'keyboard help'],
      run: () => actions.openCommandPalette(),
    },
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
      slashAliases: ['/clear'],
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
      id: 'chat.sessions',
      title: 'Find Sessions',
      description: 'Filter the session sidebar, or report how many sessions are currently loaded.',
      category: 'Chat Actions',
      source: 'chat-action',
      slash: '/sessions',
      usage: '/sessions [query|all|roots|subtree]',
      aliases: ['session search', 'session list'],
      run: async (context) => {
        const query = readCommandArgument(context)
        const normalized = query.toLowerCase()
        if (['all', 'recent'].includes(normalized)) {
          await actions.setSessionViewMode('all')
          actions.setLocalCommandNotice('Showing all sessions in the current workspace.')
          return
        }
        if (['roots', 'root'].includes(normalized)) {
          await actions.setSessionViewMode('roots')
          actions.setLocalCommandNotice('Showing root sessions in the current workspace.')
          return
        }
        if (['subtree', 'tree', 'branch'].includes(normalized)) {
          await actions.setSessionViewMode('subtree')
          return
        }
        actions.setSessionSearch(query)
        await actions.setSessionViewMode('all', query)
        actions.setLocalCommandNotice(
          query
            ? `Filtering ${formatUsageCount(state.sessions.value.length)} loaded sessions by “${query}”.`
            : `${formatUsageCount(state.sessions.value.length)} sessions are loaded in the current workspace.`,
        )
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
      slashAliases: ['/resume-run'],
      usage: '/continue',
      aliases: ['resume run'],
      run: async () => {
        if (!state.selectedSessionId.value) return
        await actions.continueCurrentSession()
      },
    },
    {
      id: 'chat.compact-session',
      title: 'Compact Current Session',
      description: 'Summarize older context and continue the active session with a smaller prompt.',
      category: 'Chat Actions',
      source: 'chat-action',
      slash: '/compact',
      slashAliases: ['/compress', '/summarize'],
      usage: '/compact',
      aliases: ['session compaction'],
      run: async () => {
        if (!state.selectedSessionId.value) return
        await actions.compactCurrentSession()
      },
    },
    {
      id: 'chat.fork-session',
      title: 'Fork Current Session',
      description: 'Fork the selected session at the latest message.',
      category: 'Chat Actions',
      source: 'chat-action',
      slash: '/fork',
      slashAliases: ['/branch'],
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
      slashAliases: ['/export', '/save'],
      usage: '/export [filename]',
      aliases: ['session export'],
      run: async (context) => {
        if (!state.selectedSessionId.value) return
        await actions.exportCurrentSession(readCommandArgument(context) || undefined)
      },
    },
    {
      id: 'chat.rename-session',
      title: 'Rename Current Session',
      description: 'Rename the active session, using an argument or a browser prompt.',
      category: 'Chat Actions',
      source: 'chat-action',
      slash: '/rename',
      slashAliases: ['/title'],
      usage: '/rename [title]',
      aliases: ['session title'],
      run: (context) => actions.renameCurrentSession(readCommandArgument(context) || undefined),
    },
    {
      id: 'chat.timeline',
      title: 'Refresh Session Timeline',
      description: 'Reload the active conversation and its domain-event timeline.',
      category: 'Chat Actions',
      source: 'chat-action',
      slash: '/timeline',
      slashAliases: ['/events'],
      usage: '/timeline [limit]',
      aliases: ['session events'],
      run: async (context) => {
        if (!state.selectedSessionId.value) return
        const rawLimit = context?.args[0] || '100'
        const limit = Number(rawLimit)
        if (!Number.isInteger(limit) || limit <= 0) {
          actions.setLocalCommandNotice('Usage: /timeline [limit]')
          return
        }
        await actions.loadSessionTimeline(limit)
        actions.setLocalCommandNotice(
          `Loaded ${formatUsageCount(state.timelineEvents.value.length)} timeline events for session #${state.selectedSessionId.value}.`,
        )
      },
    },
    {
      id: 'chat.model-status',
      title: 'Show Active Model',
      description: 'Show the active execution model and point to the Chat run-option controls.',
      category: 'Chat Actions',
      source: 'chat-action',
      slash: '/model',
      usage: '/model',
      aliases: ['provider model', 'run options'],
      run: () => {
        const execution = state.sessionState.value?.execution
        const provider = execution?.model_provider_id || 'automatic provider'
        const model = execution?.model_id || 'automatic model'
        actions.setLocalCommandNotice(`Active model: ${provider}/${model}. Change it in the Chat Run Options panel.`)
        actions.focusRunOptions()
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
        if (!state.selectedSessionId.value || !state.sessionUsageSummary.value.requests) {
          actions.setLocalCommandNotice('No provider usage has been recorded for the active session yet.')
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
      slashAliases: ['/lineage', '/branch-history', '/branches'],
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
      slashAliases: ['/rewind', '/backtrack'],
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
    {
      id: 'chat.parent-session',
      title: 'Open Parent Session',
      description: 'Switch to the parent of the active session branch.',
      category: 'Chat Actions',
      source: 'chat-action',
      slash: '/parent',
      usage: '/parent',
      aliases: ['parent branch'],
      run: async () => {
        const parent = state.parentSession.value
        if (!parent) {
          actions.setLocalCommandNotice('The active session has no loaded parent.')
          return
        }
        await actions.selectSession(parent.id)
      },
    },
    {
      id: 'chat.child-session',
      title: 'Open Child Session',
      description: 'Switch to a child branch by one-based index, defaulting to the first child.',
      category: 'Chat Actions',
      source: 'chat-action',
      slash: '/children',
      slashAliases: ['/child'],
      usage: '/children [index]',
      aliases: ['child branch'],
      run: async (context) => {
        const requested = Number(context?.args[0] || '1')
        const index = Number.isInteger(requested) && requested > 0 ? requested - 1 : 0
        const child = state.childSessions.value[index]
        if (!child) {
          actions.setLocalCommandNotice(`Child session ${index + 1} is not available.`)
          return
        }
        await actions.selectSession(child.id)
      },
    },
    {
      id: 'chat.status',
      title: 'Show Session Status',
      description: 'Summarize the selected session, execution, workflow, and pending requests.',
      category: 'Chat Actions',
      source: 'chat-action',
      slash: '/status',
      usage: '/status',
      aliases: ['run status'],
      run: () => {
        const snapshot = state.sessionState.value
        if (!snapshot) {
          actions.setLocalCommandNotice('No active session status is loaded.')
          return
        }
        const pending = snapshot.pending_interactive_requests?.length || 0
        actions.setLocalCommandNotice(
          `Session #${snapshot.session.id}: execution=${snapshot.active_execution?.phase || 'idle'} · workflow=${snapshot.workflow_state} · pending=${pending}.`,
        )
      },
    },
    {
      id: 'chat.ask-aside',
      title: 'Ask in an Aside Session',
      description: 'Fork a child session, submit a side question there, and keep the current session selected.',
      category: 'Chat Actions',
      source: 'chat-action',
      slash: '/btw',
      slashAliases: ['/aside', '/side'],
      usage: '/btw <question>',
      aliases: ['side question', 'background question'],
      run: async (context) => {
        const question = readCommandArgument(context)
        if (!question) {
          actions.setLocalCommandNotice('Usage: /btw <question>')
          return
        }
        await actions.askAside(question)
      },
    },
    {
      id: 'chat.copy-last-assistant',
      title: 'Copy Last Assistant Message',
      description: 'Copy the rendered text from the latest assistant message to the browser clipboard.',
      category: 'Chat Actions',
      source: 'chat-action',
      slash: '/copy-message',
      slashAliases: ['/copy-last', '/copy-assistant'],
      usage: '/copy-message',
      aliases: ['clipboard', 'last answer'],
      run: async () => {
        const message = [...state.messages.value].reverse().find((item) => item.role === 'assistant')
        const text = message
          ? messageBlocks(message)
              .map((block) => block.body)
              .join('\n\n')
          : ''
        await actions.copyText(text, `Copied assistant message #${message?.id || ''}.`)
      },
    },
    {
      id: 'chat.attach-file',
      title: 'Attach File',
      description: 'Open the browser file picker and stage one or more files for the next message.',
      category: 'Chat Actions',
      source: 'chat-action',
      slash: '/attach',
      slashAliases: ['/file'],
      usage: '/attach',
      aliases: ['upload file', 'composer attachment'],
      run: () => actions.openAttachmentPicker(false),
    },
    {
      id: 'chat.attach-image',
      title: 'Attach Image',
      description: 'Open the browser image picker and stage images for the next message.',
      category: 'Chat Actions',
      source: 'chat-action',
      slash: '/image',
      slashAliases: ['/paste-image'],
      usage: '/image',
      aliases: ['upload image', 'vision input'],
      run: () => actions.openAttachmentPicker(true),
    },
    {
      id: 'chat.copy-conversation',
      title: 'Copy Visible Conversation',
      description: 'Copy all currently loaded user and assistant message text to the browser clipboard.',
      category: 'Chat Actions',
      source: 'chat-action',
      slash: '/copy-visible',
      slashAliases: ['/copy', '/yank'],
      usage: '/copy-visible',
      aliases: ['clipboard', 'visible transcript'],
      run: async () => {
        const text = state.messages.value
          .map((message) => {
            const body = messageBlocks(message)
              .map((block) => block.body)
              .join('\n\n')
            return body ? `${message.role.toUpperCase()}\n${body}` : ''
          })
          .filter(Boolean)
          .join('\n\n')
        await actions.copyText(text, `Copied ${state.messages.value.length} loaded messages.`)
      },
    },
    {
      id: 'chat.download-workspace-file',
      title: 'Download Workspace File',
      description: 'Download a regular file from the selected workspace to this browser.',
      category: 'Workspace Actions',
      source: 'workspace-action',
      slash: '/download',
      slashAliases: ['/dl'],
      usage: '/download <workspace-path>',
      aliases: ['save workspace file', 'browser download'],
      run: async (context) => {
        const path = readCommandArgument(context)
        if (!path) {
          actions.setLocalCommandNotice('Usage: /download <workspace-path>')
          return
        }
        await actions.downloadWorkspaceFile(path)
      },
    },
    {
      id: 'chat.create-commit',
      title: 'Create Git Commit',
      description: 'Commit the currently staged changes in the runtime workspace.',
      category: 'Git Actions',
      source: 'workspace-action',
      slash: '/commit',
      usage: '/commit <message>',
      aliases: ['git commit', 'staged changes'],
      run: async (context) => {
        const message = readCommandArgument(context)
        if (!message) {
          actions.setLocalCommandNotice('Usage: /commit <message>')
          return
        }
        await actions.createCommit(message)
      },
    },
    {
      id: 'chat.create-pull-request',
      title: 'Create GitHub Pull Request',
      description: 'Create a pull request from the runtime workspace using GitHub CLI.',
      category: 'Git Actions',
      source: 'workspace-action',
      slash: '/pr',
      usage: '/pr <title> [--body <text>] [--base <branch>] [--head <branch>]',
      aliases: ['pull request', 'github pr'],
      run: async (context) => {
        const plan = parsePullRequestCommandArgs(context?.args || [])
        if (plan.kind === 'error') {
          actions.setLocalCommandNotice(plan.message)
          return
        }
        await actions.createPullRequest(plan)
      },
    },
    ...(
      [
        { slash: '/allow', title: 'Allow Pending Permission Once', kind: 'allow_once' },
        { slash: '/allow-always', title: 'Always Allow Pending Permission', kind: 'allow_always' },
        { slash: '/deny', title: 'Deny Pending Permission Once', kind: 'deny_once' },
        { slash: '/deny-always', title: 'Always Deny Pending Permission', kind: 'deny_always' },
      ] as const
    ).map((permissionCommand) => ({
      id: `chat.permission.${permissionCommand.kind}`,
      title: permissionCommand.title,
      description: 'Reply to the oldest pending permission request for the active session.',
      category: 'Permission Actions',
      source: 'chat-action' as const,
      slash: permissionCommand.slash,
      usage: permissionCommand.slash,
      aliases: ['pending permission', permissionCommand.kind.replaceAll('_', ' ')],
      run: async () => {
        const request = pendingPermissionRequests(state.sessionState.value)[0]
        if (!request) {
          actions.setLocalCommandNotice('There is no pending permission request.')
          return
        }
        const scope = permissionCommand.kind.endsWith('_always') ? request.scope || 'session' : undefined
        await actions.approvePermission(request.request_id, permissionCommand.kind, scope)
      },
    })),
    {
      id: 'chat.pending-user-input',
      title: 'Show Pending User Input',
      description: 'Report pending user-input questions rendered below the active conversation.',
      category: 'Chat Actions',
      source: 'chat-action',
      slash: '/user-input',
      slashAliases: ['/reply'],
      usage: '/user-input',
      aliases: ['questions', 'pending reply'],
      run: () => {
        const count = pendingUserInputRequests(state.sessionState.value).length
        actions.setLocalCommandNotice(
          count ? `${count} user-input request(s) are waiting below.` : 'There is no pending user-input request.',
        )
      },
    },
    {
      id: 'chat.queue',
      title: 'Manage Pending Message Queue',
      description: 'Inspect, clear, or move the first pending message back into the composer.',
      category: 'Chat Actions',
      source: 'chat-action',
      slash: '/queue',
      slashAliases: ['/q'],
      usage: '/queue [list|clear|pop]',
      aliases: ['pending messages', 'queued prompts'],
      run: (context) => {
        const action = (context?.args[0] || 'list').toLowerCase()
        if (action === 'clear' || action === 'drop') {
          actions.clearComposerQueue()
          return
        }
        if (action === 'pop' || action === 'edit') {
          actions.popComposerQueue()
          return
        }
        if (action === 'list' || action === 'ls' || action === 'show') {
          const first = state.composerQueue.value[0]
          actions.setLocalCommandNotice(
            first
              ? `${state.composerQueue.value.length} queued message(s); first: ${composerQueuePreview(first)}`
              : 'The message queue is empty.',
          )
          return
        }
        actions.setLocalCommandNotice('Usage: /queue [list|clear|pop]')
      },
    },
    {
      id: 'chat.memory',
      title: 'Manage Durable Memory',
      description: 'Open memory settings, edit a named record, or forget a named record.',
      category: 'Navigation',
      source: 'navigation',
      slash: '/memory',
      slashAliases: ['/mem'],
      usage: '/memory [list|edit [name]|forget <name>]',
      aliases: ['durable context', 'memory records'],
      run: async (context) => {
        const action = (context?.args[0] || 'list').toLowerCase()
        const name = context?.args.slice(1).join(' ').trim() || ''
        if (action === 'list') {
          actions.openMemorySettings()
          return
        }
        if (action === 'edit' || action === 'open') {
          actions.openMemorySettings(name || undefined)
          return
        }
        if (['forget', 'rm', 'remove', 'delete'].includes(action) && name) {
          await actions.forgetMemory(name)
          return
        }
        actions.setLocalCommandNotice('Usage: /memory [list|edit [name]|forget <name>]')
      },
    },
    {
      id: 'chat.permissions',
      title: 'Manage Permissions',
      description: 'Open scoped permission configuration or the persisted runtime-rule manager.',
      category: 'Navigation',
      source: 'navigation',
      slash: '/permissions',
      slashAliases: ['/permission'],
      usage: '/permissions [session|workspace|global|effective|new|list]',
      aliases: ['permission rules', 'allow deny', 'guardrails'],
      run: (context) => {
        const mode = (context?.args[0] || 'session').toLowerCase()
        if (
          ![
            'session',
            'current',
            'workspace',
            'project',
            'global',
            'config',
            'effective',
            'new',
            'list',
            'rules',
            'manage',
          ].includes(mode)
        ) {
          actions.setLocalCommandNotice('Usage: /permissions [session|workspace|global|effective|new|list]')
          return
        }
        actions.openPermissionSettings(mode)
      },
    },
    {
      id: 'chat.review',
      title: 'Review Current Work',
      description: 'Invoke the bundled review runtime command and submit its generated review prompt.',
      category: 'Chat Actions',
      source: 'chat-action',
      slash: '/review',
      usage: '/review [focus]',
      aliases: ['code review', 'review changes'],
      run: (context) => actions.runRuntimeEntry('review', readCommandArgument(context)),
    },
    {
      id: 'chat.snapshot',
      title: 'Manage Session Snapshot',
      description: 'Inspect snapshots or enter, attach, and exit isolated worktrees for the active session.',
      category: 'Workspace Actions',
      source: 'workspace-action',
      slash: '/snapshot',
      usage: '/snapshot [list|enter [name]|attach <path>|exit [keep|remove [force]]]',
      aliases: ['worktree', 'isolated workspace'],
      run: async (context) => {
        const [action = 'list', ...rest] = context?.args || []
        const normalized = action.toLowerCase()
        if (normalized === 'list') {
          actions.openSnapshotInspector()
          return
        }
        if (!state.selectedSessionId.value) {
          actions.setLocalCommandNotice('Select a session before managing snapshots.')
          return
        }
        if (normalized === 'enter') {
          const name = rest.join(' ').trim()
          return await actions.invokeRuntimeTool('enter_snapshot', name ? { name } : {})
        }
        if (normalized === 'attach') {
          const path = rest.join(' ').trim()
          if (!path) {
            actions.setLocalCommandNotice('Usage: /snapshot attach <path>')
            return
          }
          return await actions.invokeRuntimeTool('enter_snapshot', { path })
        }
        if (normalized === 'exit' || normalized === 'leave') {
          const mode = (rest[0] || 'keep').toLowerCase()
          if (!['keep', 'remove'].includes(mode)) {
            actions.setLocalCommandNotice('Usage: /snapshot exit [keep|remove [force]]')
            return
          }
          if (mode === 'remove' && typeof window !== 'undefined' && !window.confirm('Remove this session snapshot?')) {
            actions.setLocalCommandNotice('Snapshot removal cancelled.')
            return
          }
          return await actions.invokeRuntimeTool('exit_snapshot', {
            action: mode,
            discard_changes: mode === 'remove' && ['force', 'discard'].includes((rest[1] || '').toLowerCase()),
          })
        }
        actions.setLocalCommandNotice('Usage: /snapshot [list|enter [name]|attach <path>|exit [keep|remove [force]]]')
      },
    },
    {
      id: 'chat.editor',
      title: 'Focus Composer Editor',
      description: 'Move keyboard focus to the browser composer editor.',
      category: 'Chat Actions',
      source: 'chat-action',
      slash: '/editor',
      slashAliases: ['/edit'],
      usage: '/editor',
      aliases: ['composer', 'prompt editor'],
      run: () => actions.focusComposer(),
    },
    {
      id: 'chat.pager',
      title: 'Focus Conversation',
      description: 'Move keyboard focus to the scrollable browser transcript.',
      category: 'Chat Actions',
      source: 'chat-action',
      slash: '/pager',
      slashAliases: ['/view', '/less'],
      usage: '/pager',
      aliases: ['transcript', 'conversation view'],
      run: () => actions.focusTranscript(),
    },
    ...sectionTabNavigationItems
      .filter((item) => Boolean(item.shortcutSlash))
      .filter((item) => item.shortcutSlash !== '/permissions')
      .map((item) => ({
        id: item.id.replace('nav.', 'chat.'),
        title: item.title,
        description: item.description,
        category: 'Navigation',
        source: 'navigation' as const,
        slash: item.shortcutSlash,
        slashAliases: item.shortcutSlashAliases,
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
