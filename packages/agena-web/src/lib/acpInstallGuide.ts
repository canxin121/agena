import { findBuiltinAcpAgent } from './acpAgentsConfig'

export type AcpInstallCommandLabelKey =
  | 'chat.nativeAgent.installGuide.commands.claudeAdapterNpm'
  | 'chat.nativeAgent.installGuide.commands.claudeAdapterBun'
  | 'chat.nativeAgent.installGuide.commands.claudeSignIn'
  | 'chat.nativeAgent.installGuide.commands.antigravityCheck'
  | 'chat.nativeAgent.installGuide.commands.antigravityLogin'
  | 'chat.nativeAgent.installGuide.commands.customCheck'

export type AcpInstallCommand = {
  id: string
  labelKey: AcpInstallCommandLabelKey
  command: string
}

export type AcpInstallGuide = {
  kind: 'claude-code' | 'antigravity' | 'custom'
  agentId: string
  expectedCommand: string
  commands: AcpInstallCommand[]
  npxPreset?: {
    command: string
    args: readonly string[]
  }
}

export const CLAUDE_CODE_NPX_PRESET = {
  command: 'npx',
  args: ['-y', '@agentclientprotocol/claude-agent-acp'] as const,
} as const

const MISSING_COMMAND_ERROR_RE =
  /Could not start .*(?:No such file or directory|os error 2\b|os error 3\b|ENOENT|cannot find the file|program not found|not found)/i

const KNOWN_INSTALL_HINT_RE = /Install Claude Code and|Install the official Antigravity CLI/i

export function isMissingAcpCommandError(error: string | null | undefined): boolean {
  if (!error) return false
  return MISSING_COMMAND_ERROR_RE.test(error) || KNOWN_INSTALL_HINT_RE.test(error)
}

export function inferAcpAgentIdFromError(error: string | null | undefined): string | undefined {
  if (!error) return undefined
  const configMatch = error.match(/acp\.agents\.([a-z0-9][a-z0-9_-]{0,63})/i)
  if (configMatch?.[1]) return configMatch[1].toLowerCase()
  if (/Could not start Claude Code\b/i.test(error)) return 'claude-code'
  if (/Could not start Antigravity\b/i.test(error)) return 'antigravity'
  return undefined
}

export function getAcpInstallGuide(agentId: string, configuredCommand?: string): AcpInstallGuide {
  const normalizedId = agentId.trim() || 'custom'
  const builtin = findBuiltinAcpAgent(normalizedId)
  const expectedCommand = configuredCommand?.trim() || builtin?.defaultCommand || normalizedId

  if (normalizedId === 'claude-code') {
    return {
      kind: 'claude-code',
      agentId: normalizedId,
      expectedCommand,
      commands: [
        {
          id: 'claude-npm',
          labelKey: 'chat.nativeAgent.installGuide.commands.claudeAdapterNpm',
          command: 'npm install -g @agentclientprotocol/claude-agent-acp',
        },
        {
          id: 'claude-bun',
          labelKey: 'chat.nativeAgent.installGuide.commands.claudeAdapterBun',
          command: 'bun add -g @agentclientprotocol/claude-agent-acp',
        },
        {
          id: 'claude-login',
          labelKey: 'chat.nativeAgent.installGuide.commands.claudeSignIn',
          command: 'claude',
        },
      ],
      npxPreset: CLAUDE_CODE_NPX_PRESET,
    }
  }

  if (normalizedId === 'antigravity') {
    return {
      kind: 'antigravity',
      agentId: normalizedId,
      expectedCommand,
      commands: [
        {
          id: 'agy-check',
          labelKey: 'chat.nativeAgent.installGuide.commands.antigravityCheck',
          command: `which ${expectedCommand} && ${expectedCommand} --version`,
        },
        {
          id: 'agy-login',
          labelKey: 'chat.nativeAgent.installGuide.commands.antigravityLogin',
          command: `${expectedCommand} acp --login`,
        },
      ],
    }
  }

  return {
    kind: 'custom',
    agentId: normalizedId,
    expectedCommand,
    commands: expectedCommand
      ? [
          {
            id: 'custom-check',
            labelKey: 'chat.nativeAgent.installGuide.commands.customCheck',
            command: `which ${expectedCommand}`,
          },
        ]
      : [],
  }
}
