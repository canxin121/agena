import type { BotProfile } from '@/lib/bots'

/** Keep the newest revision for each UUID, including archived tombstones. */
export function reconcileBots(items: BotProfile[], writes: Map<string, BotProfile>): BotProfile[] {
  const merged = new Map<string, BotProfile>()
  for (const profile of items) {
    const id = profile.id.toLowerCase()
    const previous = merged.get(id)
    if (
      !previous ||
      profile.revision > previous.revision ||
      (profile.revision === previous.revision && profile.archived)
    )
      merged.set(id, profile)
  }
  for (const [id, profile] of merged) {
    const written = writes.get(id)
    if (written && (written.revision > profile.revision || (written.revision === profile.revision && written.archived)))
      merged.set(id, written)
  }
  return [...merged.values()].filter((profile) => !profile.archived)
}
export function searchBots(profiles: BotProfile[], query: string): BotProfile[] {
  const normalize = (value: string) => value.trim().toLocaleLowerCase().replace(/\s+/g, ' ')
  const q = normalize(query)
  return profiles
    .filter((p) => normalize(`${p.name} ${p.persona.instructions}`).includes(q))
    .sort((a, b) => a.name.localeCompare(b.name) || a.id.localeCompare(b.id))
}
