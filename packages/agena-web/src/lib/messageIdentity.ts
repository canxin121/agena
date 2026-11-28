import type { MessageAuthor, MessageOrigin } from '@/types/chat'

type MessageIdentity = { author?: MessageAuthor; origin?: MessageOrigin }
type MessageHeader = MessageIdentity & { role?: string }

function record(value: unknown): Record<string, unknown> | undefined {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined
}

export function normalizeMessageIdentity(part: { author?: unknown; origin?: unknown }): MessageIdentity {
  const identity: MessageIdentity = {}
  const author = record(part.author)
  const kind = record(author?.kind)
  if (
    typeof author?.id === 'string' &&
    author.id.trim() &&
    (kind?.kind === 'instance_operator' ||
      (kind?.kind === 'channel_member' &&
        ['owner', 'administrator', 'operator', 'member', 'blocked'].includes(String(kind.role))) ||
      (kind?.kind === 'plugin' && typeof kind.as_user === 'boolean') ||
      (kind?.kind === 'system' &&
        ['automation_report', 'shared_chat', 'subagent', 'integration', 'bot'].includes(String(kind.source))))
  ) {
    identity.author = {
      id: author.id,
      kind: kind as MessageAuthor['kind'],
      ...(typeof author.display_name === 'string' ? { display_name: author.display_name } : {}),
    }
  }
  const origin = record(part.origin)
  const platform = record(origin?.platform)
  const chat = record(platform?.platform)
  if (
    typeof origin?.conversation === 'string' &&
    ['direct', 'group', 'broadcast'].includes(String(origin.kind)) &&
    ((platform?.transport === 'local' && ['web', 'tui', 'cli', 'api'].includes(String(platform.surface))) ||
      (platform?.transport === 'chat' && typeof chat?.id === 'string' && typeof chat.account === 'string'))
  ) {
    identity.origin = origin as MessageOrigin
  }
  return identity
}

export function messageSenderLabel(info: MessageHeader): string {
  const role = info.role || 'assistant'
  if (role !== 'user' || !info.author) return role
  const name = info.author.display_name?.trim()
  if (name) return name
  return info.author.kind.kind === 'instance_operator' ? role : info.author.id || role
}

export function messageSenderDetails(info: MessageHeader): string {
  if (info.role !== 'user') return ''
  const details: string[] = []
  if (info.author?.id) details.push(info.author.id)
  const origin = info.origin
  if (origin) {
    details.push(
      origin.platform.transport === 'chat'
        ? `${origin.platform.platform.id}/${origin.platform.platform.account}`
        : origin.platform.surface,
      origin.conversation,
    )
    if (origin.thread) details.push(origin.thread)
  }
  return details.join(' · ')
}
