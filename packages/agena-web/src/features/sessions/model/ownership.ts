/** Workspace membership does not change a Bot conversation's owner. */
export function isBotOwnedSession(session: unknown): boolean {
  if (!session || typeof session !== 'object' || !('owner_bot_id' in session)) return false
  return typeof session.owner_bot_id === 'string' && session.owner_bot_id.trim().length > 0
}
