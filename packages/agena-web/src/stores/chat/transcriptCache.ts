/** Recoverable transcript budgets; composer and attachment drafts are stored
 * separately and must never be removed by transcript eviction.
 */
export class TranscriptCacheIndex {
  private sessions = new Map<string, Map<string, number>>()
  private memberships = new Map<string, Set<string>>()
  private sizes = new Map<string, number>()
  private total = 0

  hasSession(id: string) {
    return this.sessions.has(id)
  }
  members(part: string): Iterable<string> {
    return this.memberships.get(part) ?? []
  }
  touch(id: string) {
    const entries = this.sessions.get(id)
    if (entries) {
      this.sessions.delete(id)
      this.sessions.set(id, entries)
    }
  }
  set(id: string, entries: Iterable<[string, unknown]>) {
    this.remove(id)
    for (const [part, value] of entries) this.update(id, part, value)
  }
  update(id: string, part: string, value: unknown) {
    let entries = this.sessions.get(id)
    if (!entries) this.sessions.set(id, (entries = new Map()))
    const previous = entries.get(part) ?? 0
    const size = estimateBytes(value)
    entries.set(part, size)
    this.sizes.set(id, (this.sizes.get(id) ?? 0) + size - previous)
    this.total += size - previous
    let members = this.memberships.get(part)
    if (!members) this.memberships.set(part, (members = new Set()))
    members.add(id)
  }
  removePart(id: string, part: string) {
    const entries = this.sessions.get(id)
    const size = entries?.get(part) ?? 0
    entries?.delete(part)
    this.total -= size
    this.sizes.set(id, (this.sizes.get(id) ?? 0) - size)
    const members = this.memberships.get(part)
    members?.delete(id)
    if (!members?.size) this.memberships.delete(part)
  }
  remove(id: string) {
    for (const part of this.sessions.get(id)?.keys() ?? []) this.removePart(id, part)
    this.sessions.delete(id)
    this.sizes.delete(id)
  }
  clear() {
    this.sessions.clear()
    this.memberships.clear()
    this.sizes.clear()
    this.total = 0
  }
  evictions(protectedIds: ReadonlySet<string>, maxSessions = 24, maxBytes = 32 * 1024 * 1024): string[] {
    let count = this.sessions.size
    let bytes = this.total
    const victims: string[] = []
    for (const id of this.sessions.keys()) {
      if (count <= maxSessions && bytes <= maxBytes) break
      if (protectedIds.has(id)) continue
      victims.push(id)
      count--
      bytes -= this.sizes.get(id) ?? 0
    }
    return victims
  }
}

function estimateBytes(value: unknown, seen = new WeakSet<object>()): number {
  if (typeof value === 'string') return value.length * 2 + 24
  if (value === null || typeof value !== 'object') return 8
  if (seen.has(value)) return 0
  seen.add(value)
  let size = 32
  for (const [key, item] of Object.entries(value)) size += key.length * 2 + 16 + estimateBytes(item, seen)
  return size
}
