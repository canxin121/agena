import { describe, expect, it } from 'bun:test'

import type { NativeSessionStatus } from '@/features/sessions/api/nativeAgentSessions'
import {
  acpCanForkMessage,
  acpCanForkSession,
  acpCanRewindMessage,
  acpForkPointIds,
  acpIsReadOnlyHistory,
  acpRewindPointIds,
  acpSupportsAdditionalDirectories,
  acpSupportsSystemPrompt,
  externalAgentSuggestionLabel,
  MAX_ACP_ADDITIONAL_DIRECTORIES,
  normalizeAdditionalDirectories,
} from '@/pages/chat/acpSessionControls'

function makeStatus(overrides: Partial<NativeSessionStatus> = {}): NativeSessionStatus {
  return {
    kind: 'status',
    session: {
      native_id: 'native-1',
      cwd: '/repo',
      title: 'Session',
      updated_at: '2026-11-28T00:00:00Z',
    },
    native_status: 'open',
    additional_directories: ['/extra'],
    fork_points: [
      { message_id: 101, label: 'First prompt' },
      { message_id: 205, label: 'Second prompt' },
    ],
    rewind_points: [{ message_id: 205, label: 'Second prompt' }],
    capabilities: {
      status: true,
      title: true,
      close: true,
      delete: true,
      fork: true,
      fork_before_message: true,
      rewind_before_message: true,
      continue_conversation: true,
      additional_directories: true,
      system_prompt: true,
    },
    ...overrides,
  }
}

describe('acpSessionControls', () => {
  it('filters fork and rewind points strictly by advertised ACP capabilities', () => {
    const enabled = makeStatus()
    expect([...acpForkPointIds(enabled)]).toEqual(['101', '205'])
    expect([...acpRewindPointIds(enabled)]).toEqual(['205'])
    expect(acpCanForkMessage(enabled, '101')).toBe(true)
    expect(acpCanForkMessage(enabled, '999')).toBe(false)
    expect(acpCanRewindMessage(enabled, '205')).toBe(true)
    expect(acpCanRewindMessage(enabled, '101')).toBe(false)

    const disabled = makeStatus({
      capabilities: {
        ...makeStatus().capabilities,
        fork_before_message: false,
        rewind_before_message: false,
      },
    })
    expect(acpForkPointIds(disabled).size).toBe(0)
    expect(acpRewindPointIds(disabled).size).toBe(0)
    expect(acpCanForkMessage(disabled, '101')).toBe(false)
    expect(acpCanRewindMessage(disabled, '205')).toBe(false)
    expect(acpForkPointIds(null).size).toBe(0)
  })

  it('detects read-only history only when continue_conversation is explicitly false', () => {
    expect(acpIsReadOnlyHistory(null)).toBe(false)
    expect(acpIsReadOnlyHistory(makeStatus())).toBe(false)
    expect(
      acpIsReadOnlyHistory(
        makeStatus({
          capabilities: {
            ...makeStatus().capabilities,
            continue_conversation: undefined,
          },
        }),
      ),
    ).toBe(false)
    expect(
      acpIsReadOnlyHistory(
        makeStatus({
          capabilities: {
            ...makeStatus().capabilities,
            continue_conversation: false,
          },
        }),
      ),
    ).toBe(true)
  })

  it('checks session-level fork, directories, and system prompt capabilities', () => {
    const full = makeStatus()
    expect(acpCanForkSession(full)).toBe(true)
    expect(acpSupportsAdditionalDirectories(full)).toBe(true)
    expect(acpSupportsSystemPrompt(full)).toBe(true)

    const unstarted = makeStatus({ session: null })
    expect(acpCanForkSession(unstarted)).toBe(false)

    const limited = makeStatus({
      capabilities: {
        ...full.capabilities,
        fork: false,
        additional_directories: false,
        system_prompt: false,
      },
    })
    expect(acpCanForkSession(limited)).toBe(false)
    expect(acpSupportsAdditionalDirectories(limited)).toBe(false)
    expect(acpSupportsSystemPrompt(limited)).toBe(false)
  })

  it('normalizes, deduplicates, and caps additional workspace directories', () => {
    expect(normalizeAdditionalDirectories(['  /a  ', '', '  ', '/b', '/a', '/c '])).toEqual(['/a', '/b', '/c'])

    const oversized = Array.from({ length: 24 }, (_, idx) => `/dir-${idx}`)
    const capped = normalizeAdditionalDirectories(oversized)
    expect(capped.length).toBe(MAX_ACP_ADDITIONAL_DIRECTORIES)
    expect(capped[0]).toBe('/dir-0')
    expect(capped.at(-1)).toBe('/dir-15')
  })

  it('formats compact single-line labels from ACP prompt suggestions', () => {
    expect(
      externalAgentSuggestionLabel({
        prompt: [
          { type: 'text', text: 'Run\n  unit   tests  ' },
          { type: 'resource_link', uri: 'file:///repo/tests', name: 'tests/' },
        ],
      }),
    ).toBe('Run unit tests tests/')

    expect(
      externalAgentSuggestionLabel({
        prompt: [{ type: 'resource_link', uri: 'file:///repo/README.md', name: '' }],
      }),
    ).toBe('file:///repo/README.md')
  })
})
