import { describe, expect, test } from 'bun:test'

import {
  applyAcpAgentDraft,
  blankCustomAcpAgentDraft,
  draftFromAcpAgent,
  formatAcpCommandPreview,
  isAbsoluteAcpPath,
  isValidAcpAgentId,
  isValidAcpEnvKey,
  listAcpAgentIds,
  parseAcpConfigDocument,
  removeAcpAgentFromConfig,
  resolveAcpAgentSummary,
  serializeAcpConfigDocument,
  validateAcpAgentDraft,
} from '../src/lib/acpAgentsConfig'
import { buildSettingsNavigationCategories } from '../src/components/settings/settingsInformationArchitecture'
import { buildSettingsSidebarGroups } from '../src/components/settings/sidebar/settingsSidebarNavigation'

describe('acpAgentsConfig helpers', () => {
  test('validates agent IDs, env keys, and absolute writable paths', () => {
    expect(isValidAcpAgentId('codex')).toBe(true)
    expect(isValidAcpAgentId('gemini-cli_2')).toBe(true)
    expect(isValidAcpAgentId('0agent')).toBe(true)
    expect(isValidAcpAgentId('')).toBe(false)
    expect(isValidAcpAgentId('Codex')).toBe(false)
    expect(isValidAcpAgentId('-leading')).toBe(false)
    expect(isValidAcpAgentId('bad name')).toBe(false)
    expect(isValidAcpAgentId('a'.repeat(65))).toBe(false)

    expect(isValidAcpEnvKey('CUSTOM_API_TOKEN')).toBe(true)
    expect(isValidAcpEnvKey('_PRIVATE')).toBe(true)
    expect(isValidAcpEnvKey('AGENA_TOKEN')).toBe(false)
    expect(isValidAcpEnvKey('agena_secret')).toBe(false)
    expect(isValidAcpEnvKey('BAD-KEY')).toBe(false)
    expect(isValidAcpEnvKey('1BAD')).toBe(false)

    expect(isAbsoluteAcpPath('/home/user/.codex')).toBe(true)
    expect(isAbsoluteAcpPath('C:\\Users\\dev\\.codex')).toBe(true)
    expect(isAbsoluteAcpPath('relative/dir')).toBe(false)
    expect(isAbsoluteAcpPath('')).toBe(false)
  })

  test('round-trips custom agents and built-in overrides while omitting redundant defaults', () => {
    const initial = parseAcpConfigDocument({
      agents: {
        codex: {
          name: 'Codex CLI',
          command: 'codex-acp',
          args: ['--stdio'],
          env: { CODEX_PROFILE: 'work' },
          client_tools: false,
          writable_paths: ['/tmp/codex'],
        },
        antigravity: {
          disabled: true,
        },
      },
    })

    expect(listAcpAgentIds(initial)).toEqual(['claude-code', 'antigravity', 'codex'])

    const claudeSummary = resolveAcpAgentSummary('claude-code', initial.agents['claude-code'])
    expect(claudeSummary.status).toBe('builtin')
    expect(claudeSummary.command).toBe('claude-agent-acp')

    const agySummary = resolveAcpAgentSummary('antigravity', initial.agents.antigravity)
    expect(agySummary.status).toBe('disabled')
    expect(agySummary.args).toEqual(['acp', '--process-per-session'])

    const codexSummary = resolveAcpAgentSummary('codex', initial.agents.codex)
    expect(codexSummary.status).toBe('custom')
    expect(formatAcpCommandPreview(codexSummary.command, codexSummary.args)).toBe('codex-acp --stdio')

    // Re-enabling built-in Antigravity without overrides removes the entry cleanly.
    const agyDraft = draftFromAcpAgent('antigravity', initial.agents.antigravity)
    agyDraft.disabled = false
    const afterRestore = applyAcpAgentDraft(initial, agyDraft)
    expect(afterRestore.agents.antigravity).toBeUndefined()

    // Removing the last custom agent serializes to null (delete global acp key).
    const emptyDoc = removeAcpAgentFromConfig(afterRestore, 'codex')
    expect(serializeAcpConfigDocument(emptyDoc)).toBeNull()
  })

  test('validates custom agent drafts before saving', () => {
    const draft = blankCustomAcpAgentDraft()
    expect(validateAcpAgentDraft(draft, [], true).map((e) => e.code)).toEqual([
      'invalid_id',
      'missing_command',
    ])

    draft.id = 'claude-code'
    draft.command = 'custom-claude'
    expect(validateAcpAgentDraft(draft, [], true).map((e) => e.code)).toEqual(['duplicate_id'])

    draft.id = 'my-acp'
    draft.envRows = [
      { key: 'AGENA_SECRET', value: 'x' },
      { key: 'TOKEN', value: '1' },
      { key: 'TOKEN', value: '2' },
    ]
    draft.writablePathsText = 'relative/path'
    expect(validateAcpAgentDraft(draft, [], true).map((e) => e.code)).toEqual([
      'invalid_env_key',
      'duplicate_env_key',
      'invalid_writable_path',
    ])

    draft.envRows = [{ key: 'TOKEN', value: '1' }]
    draft.writablePathsText = '/var/lib/my-acp'
    expect(validateAcpAgentDraft(draft, [], true)).toEqual([])
  })

  test('settings navigation exposes ACP agents under Extensions & connections', () => {
    const categories = buildSettingsNavigationCategories()
    const extensions = categories.find((cat) => cat.id === 'extensions')
    expect(extensions?.children?.some((task) => task.id === 'acp-agents' && task.view === 'acp-agents')).toBe(true)

    const rows = buildSettingsSidebarGroups({
      query: 'codex',
      tabs: categories,
      activeTab: 'interface',
      activeView: 'web-appearance',
    }).flatMap((group) => group.items)
    expect(rows.some((row) => row.key === 'extensions/acp-agents' && row.view === 'acp-agents')).toBe(true)
  })
})
