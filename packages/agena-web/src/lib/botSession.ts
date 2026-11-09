import { resolveBotSession } from './bots'

/** Bodyless server-default resolution, retained for callers that need only its session. */
export async function getBotSession(signal?: AbortSignal): Promise<string> {
  return String((await resolveBotSession(undefined, signal)).session_id)
}

/** Check ownership on both sides of hydration: either request can outlive navigation. */
export async function bindBotSession(options: {
  resolve: () => Promise<string>
  isCurrent: () => boolean
  hydrate: (sessionId: string, isCurrent: () => boolean) => Promise<void>
  bind: (sessionId: string) => Promise<unknown>
}): Promise<string | null> {
  const sessionId = await options.resolve()
  if (!options.isCurrent()) return null
  await options.hydrate(sessionId, options.isCurrent)
  if (!options.isCurrent()) return null
  await options.bind(sessionId)
  return options.isCurrent() ? sessionId : null
}
