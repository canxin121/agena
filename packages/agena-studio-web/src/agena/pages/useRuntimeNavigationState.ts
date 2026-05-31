import { computed, type Ref } from 'vue'
import type { Router } from 'vue-router'

import { workspaceShortcuts } from '../lib/runtimeWorkspaceShortcuts'
import type { RuntimeSkill, WorkspaceResource } from '../lib/agenaApi'
import { buildRuntimeSectionPath } from './runtimePageStateModel'

export type RuntimeNavigationStateInput = {
  selectedSessionId: Ref<number | null>
  selectedWorkspaceId: Ref<number | null>
  selectedPluginManifest: Ref<unknown>
  workspaces: Ref<WorkspaceResource[]>
}

export function useRuntimeNavigationState(input: RuntimeNavigationStateInput, deps: { router: Pick<Router, 'push'> }) {
  const selectedWorkspace = computed(
    () => input.workspaces.value.find((workspace) => workspace.id === input.selectedWorkspaceId.value) || null,
  )

  function openSelectedSessionInChat() {
    if (!input.selectedSessionId.value) return
    void deps.router.push(`/chat?session=${input.selectedSessionId.value}`)
  }

  function openWorkspacePath(relativePath?: string | null) {
    if (!selectedWorkspace.value) return
    const query: Record<string, string> = {
      workspace: String(selectedWorkspace.value.id),
    }
    const normalizedPath = String(relativePath || '')
      .trim()
      .replace(/^\/+/, '')
    if (normalizedPath) {
      query.path = normalizedPath
    }
    void deps.router.push({ path: '/workspace', query })
  }

  function openWorkspaceShortcut(shortcutId: string) {
    const shortcut = workspaceShortcuts.find((item) => item.id === shortcutId)
    if (!shortcut) return
    openWorkspacePath(shortcut.relativePath)
  }

  function openRuntimeConfigRoot() {
    void deps.router.push(buildRuntimeSectionPath('settings', 'plugins'))
  }

  function openPluginManifestInWorkspace() {
    if (!input.selectedPluginManifest.value) return
    void deps.router.push(buildRuntimeSectionPath('settings', 'plugins'))
  }

  function openPluginLogsWorkspacePath() {
    void deps.router.push(buildRuntimeSectionPath('settings', 'plugins'))
  }

  function openRuntimeEntrySource(entry: RuntimeSkill) {
    if (!entry.source_path) return
    openWorkspacePath(entry.source_path)
  }

  function openRuntimeEntryInChat(entry: RuntimeSkill) {
    const query: Record<string, string> = {
      slash: `/${entry.name}`,
    }
    if (input.selectedSessionId.value) {
      query.session = String(input.selectedSessionId.value)
    }
    void deps.router.push({ path: '/chat', query })
  }

  return {
    openPluginLogsWorkspacePath,
    openPluginManifestInWorkspace,
    openRuntimeConfigRoot,
    openRuntimeEntryInChat,
    openRuntimeEntrySource,
    openSelectedSessionInChat,
    openWorkspacePath,
    openWorkspaceShortcut,
    selectedWorkspace,
  }
}
