import { describe, expect, test } from 'bun:test'

import { renderVueSsr } from './test/renderVueSsr'

const panels = {
  providers: {
    authProviders: [
      {
        provider_id: 'openai',
        configured: true,
        credential_present: true,
        credential_type: 'api_key',
        key_preview: 'sk-***',
        expires_at: null,
        expired: false,
        account_id: 'acct_1',
        enterprise_url: null,
      },
    ],
    browserAuthCodeDrafts: { openai: '' },
    browserAuthInstanceDrafts: { openai: '' },
    browserAuthStartState: { openai: null },
    deviceAuthEnterpriseDrafts: { openai: '' },
    deviceAuthStartState: { openai: null },
    drafts: { openai: '' },
    finishBrowserAuth: () => {},
    pollDeviceAuth: () => {},
    saveApiKey: () => {},
    refreshCredential: () => {},
    clearCredential: () => {},
    startBrowserAuth: () => {},
    startDeviceAuth: () => {},
  },
  permissions: {
    search: 'bash',
    statusFilter: 'active',
    scopeFilter: 'workspace',
    modeFilter: 'allow',
    subjectFilter: 'tool',
    draft: {
      subjectKind: 'tool',
      toolName: 'bash',
      qualifier: 'git status *',
      pathAccessKind: 'read',
      workspaceRoot: '',
      targetPath: '',
      networkTarget: '',
      networkPort: '',
      scope: 'workspace',
      sessionId: '',
      mode: 'allow',
    },
    editingRuleId: null,
    filteredRules: [
      {
        id: 5,
        mode: 'allow',
        revoked_at: null,
        updated_at: '2026-05-11T00:00:00Z',
      },
    ],
    saveRule: () => {},
    resetDraft: () => {},
    editRule: () => {},
    revokeRuleAction: () => {},
    deleteRuleAction: () => {},
    ruleLabel: () => 'bash :: git status *',
    rulePreview: () => 'Allow git status in workspace',
    ruleFacts: () => ['workspace', 'tool'],
  },
  desktop: {
    enabled: true,
    saving: false,
    updateRunning: false,
    backendUrl: 'http://127.0.0.1:3210',
    notice: 'Desktop connected',
    config: { ok: true },
    runtimeFacts: [{ label: 'Installer Version', value: '1.2.3' }],
    statusFacts: [{ label: 'Running', value: 'yes' }],
    backendErrorFacts: [],
    updateFacts: [{ label: 'Phase', value: 'idle' }],
    configFacts: [{ label: 'Workspace Root', value: '/workspace' }],
    updateProgressPercent: '50%',
    serviceUpdateUrl: 'https://example.com/service.tgz',
    installerUpdateUrl: 'https://example.com/installer.AppImage',
    installerAssetName: 'installer.AppImage',
    form: {
      autostart_on_boot: true,
      host: '127.0.0.1',
      port: '3210',
      workspace_root: '/workspace',
      agena_config_path: '/workspace/.agena/config.toml',
      agena_mode: 'default',
      database_path: '/workspace/agena.db',
      database_url: 'sqlite:///workspace/agena.db',
      backend_log_level: 'info',
      ui_cookie_samesite: 'lax',
    },
    loadPanel: () => {},
    restartBackendAction: () => {},
    openBackendUrlAction: () => {},
    openConfigAction: () => {},
    refreshUpdateProgressAction: () => {},
    runServiceUpdateAction: () => {},
    runInstallerUpdateAction: () => {},
    saveConfigAction: () => {},
  },
}

describe('SettingsSectionPanelRenderer', () => {
  test('renders provider settings content', async () => {
    const html = await renderVueSsr('/src/agena/pages/SettingsSectionPanelRenderer.vue', {
      activeTab: 'providers',
      loading: false,
      load: async () => {},
      panels,
    })

    expect(html.includes('Provider Auth')).toBe(true)
    expect(html.includes('openai')).toBe(true)
    expect(html.includes('Save Key')).toBe(true)
  })

  test('renders permission settings content', async () => {
    const html = await renderVueSsr('/src/agena/pages/SettingsSectionPanelRenderer.vue', {
      activeTab: 'permissions',
      loading: false,
      load: async () => {},
      panels,
    })

    expect(html.includes('Guardrails')).toBe(true)
    expect(html.includes('Create Rule')).toBe(true)
    expect(html.includes('Allow git status in workspace')).toBe(true)
  })

  test('falls back to desktop settings content', async () => {
    const html = await renderVueSsr('/src/agena/pages/SettingsSectionPanelRenderer.vue', {
      activeTab: 'desktop',
      loading: false,
      load: async () => {},
      panels,
    })

    expect(html.includes('Runtime Control')).toBe(true)
    expect(html.includes('Service and Installer')).toBe(true)
    expect(html.includes('Save Config')).toBe(true)
  })
})
