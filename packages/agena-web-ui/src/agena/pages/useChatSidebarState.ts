import type { ComputedRef, Ref } from 'vue'

import type { SessionResource, WorkspaceResource } from '../lib/agenaApi'

export type ChatSidebarStateInput = {
  createSessionAction: () => void | Promise<void>
  loadSessionsForWorkspace: (workspaceId: number, preserveSelection?: boolean) => void | Promise<void>
  loading: Ref<boolean>
  newSessionTitle: Ref<string>
  resolveWorkspaceAction: (createIfMissing: boolean) => void | Promise<void>
  selectedSession: ComputedRef<SessionResource | null>
  selectedSessionId: Ref<number | null>
  selectedWorkspace: ComputedRef<WorkspaceResource | null>
  selectedWorkspaceId: Ref<number | null>
  selectSession: (sessionId: number) => void | Promise<void>
  selectWorkspace: (workspaceId: number) => void | Promise<void>
  sessionSearch: Ref<string>
  sessionViewMode: Ref<'all' | 'roots' | 'subtree'>
  setSessionViewMode: (mode: 'all' | 'roots' | 'subtree', query?: string) => void | Promise<void>
  sessions: Ref<SessionResource[]>
  workspacePath: Ref<string>
  workspaces: Ref<WorkspaceResource[]>
}

export function useChatSidebarState(input: ChatSidebarStateInput) {
  return {
    createSessionAction: input.createSessionAction,
    loadSessionsForWorkspace: input.loadSessionsForWorkspace,
    loading: input.loading,
    newSessionTitle: input.newSessionTitle,
    resolveWorkspaceAction: input.resolveWorkspaceAction,
    selectedSession: input.selectedSession,
    selectedSessionId: input.selectedSessionId,
    selectedWorkspace: input.selectedWorkspace,
    selectedWorkspaceId: input.selectedWorkspaceId,
    selectSession: input.selectSession,
    selectWorkspace: input.selectWorkspace,
    sessionSearch: input.sessionSearch,
    sessionViewMode: input.sessionViewMode,
    setSessionViewMode: input.setSessionViewMode,
    sessions: input.sessions,
    workspacePath: input.workspacePath,
    workspaces: input.workspaces,
  }
}

export type ChatSidebarState = ReturnType<typeof useChatSidebarState>
