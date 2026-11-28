import type { NativeSessionStatus } from '@/features/sessions/api/nativeAgentSessions'
import type { ExternalAgentSuggestion } from '@/types/chat'

export const MAX_ACP_ADDITIONAL_DIRECTORIES = 16

export function acpForkPointIds(status: NativeSessionStatus | null): Set<string> {
  if (!status?.capabilities.fork_before_message) return new Set()
  return new Set((status.fork_points ?? []).map((point) => String(point.message_id)))
}

export function acpRewindPointIds(status: NativeSessionStatus | null): Set<string> {
  if (!status?.capabilities.rewind_before_message) return new Set()
  return new Set((status.rewind_points ?? []).map((point) => String(point.message_id)))
}

export function acpCanForkMessage(status: NativeSessionStatus | null, messageId: string): boolean {
  const id = String(messageId || '').trim()
  if (!id || !status?.capabilities.fork_before_message) return false
  return (status.fork_points ?? []).some((point) => String(point.message_id) === id)
}

export function acpCanRewindMessage(status: NativeSessionStatus | null, messageId: string): boolean {
  const id = String(messageId || '').trim()
  if (!id || !status?.capabilities.rewind_before_message) return false
  return (status.rewind_points ?? []).some((point) => String(point.message_id) === id)
}

export function acpIsReadOnlyHistory(status: NativeSessionStatus | null): boolean {
  return status?.capabilities.continue_conversation === false
}

export function acpCanForkSession(status: NativeSessionStatus | null): boolean {
  return Boolean(status?.session && status.capabilities.fork)
}

export function acpSupportsAdditionalDirectories(status: NativeSessionStatus | null): boolean {
  return Boolean(status?.capabilities.additional_directories)
}

export function acpSupportsSystemPrompt(status: NativeSessionStatus | null): boolean {
  return Boolean(status?.capabilities.system_prompt)
}

export function normalizeAdditionalDirectories(directories: readonly string[]): string[] {
  const unique: string[] = []
  const seen = new Set<string>()
  for (const raw of directories) {
    const trimmed = String(raw || '').trim()
    if (!trimmed || seen.has(trimmed)) continue
    seen.add(trimmed)
    unique.push(trimmed)
    if (unique.length >= MAX_ACP_ADDITIONAL_DIRECTORIES) break
  }
  return unique
}

export function externalAgentSuggestionLabel(suggestion: ExternalAgentSuggestion): string {
  const parts: string[] = []
  for (const block of suggestion.prompt ?? []) {
    if (block.type === 'text') {
      const normalized = block.text.replace(/\s+/g, ' ').trim()
      if (normalized) parts.push(normalized)
    } else if (block.name || block.uri) {
      parts.push(block.name || block.uri)
    }
  }
  return parts.join(' ').trim()
}
