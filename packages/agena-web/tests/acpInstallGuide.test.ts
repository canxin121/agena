import { describe, expect, it } from 'bun:test'
import { createI18n } from 'vue-i18n'

import enUS from '../src/i18n/messages/en-US'
import zhCN from '../src/i18n/messages/zh-CN'
import esES from '../src/i18n/messages/es-ES'
import frFR from '../src/i18n/messages/fr-FR'
import ptBR from '../src/i18n/messages/pt-BR'
import arSA from '../src/i18n/messages/ar-SA'
import hiIN from '../src/i18n/messages/hi-IN'
import {
  CLAUDE_CODE_NPX_PRESET,
  getAcpInstallGuide,
  inferAcpAgentIdFromError,
  isMissingAcpCommandError,
} from '../src/lib/acpInstallGuide'

describe('acpInstallGuide', () => {
  it('detects missing ACP CLI errors cleanly', () => {
    expect(
      isMissingAcpCommandError(
        'Could not start Claude Code: No such file or directory (os error 2). Install Claude Code and `claude-agent-acp` on the server\'s PATH.',
      ),
    ).toBe(true)
    expect(
      isMissingAcpCommandError(
        'Could not start Antigravity: program not found. Install the official Antigravity CLI with ACP support.',
      ),
    ).toBe(true)
    expect(isMissingAcpCommandError('Could not start Codex: ENOENT')).toBe(true)
    expect(isMissingAcpCommandError('Network timeout while connecting')).toBe(false)
    expect(isMissingAcpCommandError('')).toBe(false)
    expect(isMissingAcpCommandError(null)).toBe(false)
  })

  it('infers agent id from server spawn errors', () => {
    expect(
      inferAcpAgentIdFromError(
        'Could not start Claude Code: No such file or directory (os error 2). Configure acp.agents.claude-code.',
      ),
    ).toBe('claude-code')
    expect(
      inferAcpAgentIdFromError(
        'Could not start Antigravity: No such file or directory (os error 2).',
      ),
    ).toBe('antigravity')
    expect(
      inferAcpAgentIdFromError(
        'Could not start Codex: No such file or directory. Check acp.agents.codex_cli.',
      ),
    ).toBe('codex_cli')
    expect(inferAcpAgentIdFromError('Unrelated failure')).toBeUndefined()
  })

  it('returns actionable install commands for claude-code, antigravity, and custom agents', () => {
    const claude = getAcpInstallGuide('claude-code')
    expect(claude.kind).toBe('claude-code')
    expect(claude.expectedCommand).toBe('claude-agent-acp')
    expect(claude.commands.some((c) => c.command.includes('@agentclientprotocol/claude-agent-acp'))).toBe(true)
    expect(claude.npxPreset).toEqual(CLAUDE_CODE_NPX_PRESET)

    const agy = getAcpInstallGuide('antigravity')
    expect(agy.kind).toBe('antigravity')
    expect(agy.expectedCommand).toBe('agy')
    expect(agy.commands.some((c) => c.command.includes('agy acp --login'))).toBe(true)
    expect(agy.npxPreset).toBeUndefined()

    const custom = getAcpInstallGuide('codex', 'codex-acp')
    expect(custom.kind).toBe('custom')
    expect(custom.expectedCommand).toBe('codex-acp')
    expect(custom.commands[0]?.command).toBe('which codex-acp')
  })

  it('compiles all installGuide i18n messages cleanly without vue-i18n syntax errors across all 7 locales', () => {
    const locales = {
      'en-US': enUS,
      'zh-CN': zhCN,
      'es-ES': esES,
      'fr-FR': frFR,
      'pt-BR': ptBR,
      'ar-SA': arSA,
      'hi-IN': hiIN,
    }
    const i18n = createI18n({
      legacy: false,
      locale: 'en-US',
      messages: locales,
      onWarn: (msg) => {
        throw new Error(`Unexpected vue-i18n warning: ${msg}`)
      },
    })
    for (const locale of Object.keys(locales) as Array<keyof typeof locales>) {
      i18n.global.locale.value = locale
      const tip = i18n.global.t('chat.nativeAgent.installGuide.claudeNpxTip', {
        pkg: '@agentclientprotocol/claude-agent-acp',
      })
      expect(tip).toContain('@agentclientprotocol/claude-agent-acp')
      expect(
        i18n.global.t('chat.nativeAgent.installGuide.title', { agent: 'Claude Code' }),
      ).toContain('Claude Code')
    }
  })
})
