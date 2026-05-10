import { describe, expect, test } from 'bun:test'

import { useRuntimePageStore } from './useRuntimePageStore'

describe('useRuntimePageStore', () => {
  test('creates grouped runtime state with expected defaults', () => {
    const state = useRuntimePageStore()

    expect(state.activeTab.value).toBe('overview')
    expect(state.activeSettingsTab.value).toBe('providers')
    expect(state.activePluginsTab.value).toBe('installed')
    expect(state.runtime.value).toBe(null)
    expect(state.providers.value).toEqual([])
    expect(state.providerModels).toEqual({})
    expect(state.permissionRules.value).toEqual([])
    expect(state.sessions.value).toEqual([])
    expect(state.selectedWorkspaceId.value).toBe(null)
    expect(state.selectedSessionId.value).toBe(null)
    expect(state.selectedPluginId.value).toBe('')
    expect(state.loading.value).toBe(false)
    expect(state.pluginLoading.value).toBe(false)
    expect(state.workflowLoading.value).toBe(false)
    expect(state.desktopSaving.value).toBe(false)
    expect(state.actionError.value).toBe('')
    expect(state.actionMessage.value).toBe('')
    expect(state.desktopNotice.value).toBe('')
    expect(state.permissionSearch.value).toBe('')
    expect(state.permissionModeFilter.value).toBe('all')
    expect(state.permissionScopeFilter.value).toBe('all')
    expect(state.permissionSubjectFilter.value).toBe('all')
    expect(state.permissionStatusFilter.value).toBe('active')
    expect(state.marketplaceRegistryId.value).toBe('default')
    expect(state.marketplaceAllowUnverified.value).toBe(false)
    expect(state.marketplaceRequireSignature.value).toBe(false)
    expect(state.marketplaceRefreshIndex.value).toBe(false)
    expect(state.marketplaceCascadeUninstall.value).toBe(false)
    expect(state.marketplaceLoading.value).toBe(false)
    expect(state.desktopUpdateRunning.value).toBe(false)
    expect(state.permissionDraft).toEqual({
      subjectKind: 'builtin_tool',
      toolName: '',
      qualifier: '',
      pathAccessKind: 'read',
      workspaceRoot: '',
      targetPath: '',
      scope: 'workspace',
      sessionId: '',
      mode: 'ask',
    })
    expect(state.desktopForm).toEqual({
      autostart_on_boot: false,
      host: '',
      port: '',
      workspace_root: '',
      agena_config_path: '',
      agena_mode: '',
      database_path: '',
      database_url: '',
      backend_log_level: '',
      ui_cookie_samesite: '',
    })
  })
})
