import { i18n } from '@/i18n'
import { apiResponse } from '../../lib/api'

export type BtwAnswer = { text: string; done: boolean; error?: string | null }

export function parseBtwFrame(frame: string): BtwAnswer | null {
  let event = ''
  const data: string[] = []
  for (const line of frame.split(/\r?\n/)) {
    if (line.startsWith('event:')) event = line.slice(6).trim()
    if (line.startsWith('data:')) data.push(line.slice(5).replace(/^ /, ''))
  }
  if (!data.length) return null
  if (event !== 'btw') throw new Error(i18n.global.t('chat.errors.btwUnexpectedEvent'))
  const answer: unknown = JSON.parse(data.join('\n'))
  if (
    !answer ||
    typeof answer !== 'object' ||
    !('text' in answer) ||
    typeof answer.text !== 'string' ||
    !('done' in answer) ||
    typeof answer.done !== 'boolean' ||
    ('error' in answer && answer.error !== null && typeof answer.error !== 'string')
  ) {
    throw new Error(i18n.global.t('chat.errors.btwInvalidAnswer'))
  }
  return answer as BtwAnswer
}

/** One POST, no automatic replay. Dropping its stream cancels the server run. */
export async function askBtw(
  sessionId: string,
  question: string,
  signal: AbortSignal,
  onAnswer: (answer: BtwAnswer) => void,
) {
  const response = await apiResponse(`/api/v1/sessions/${encodeURIComponent(sessionId)}/btw`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', accept: 'text/event-stream' },
    body: JSON.stringify({ question }),
    signal,
  })
  if (!response.body) throw new Error(i18n.global.t('chat.errors.btwNoStream'))
  const reader = response.body.getReader()
  const decoder = new TextDecoder()
  let pending = ''
  try {
    while (true) {
      let timer: ReturnType<typeof setTimeout> | undefined
      const chunk = await Promise.race([
        reader.read(),
        new Promise<never>((_, reject) => {
          timer = setTimeout(() => reject(new Error(i18n.global.t('chat.errors.btwStreamStalled'))), 60_000)
        }),
      ]).finally(() => clearTimeout(timer))
      if (chunk.done) throw new Error(i18n.global.t('chat.errors.btwStreamEndedEarly'))
      pending += decoder.decode(chunk.value, { stream: true })
      if (pending.length > 4 * 1024 * 1024) throw new Error(i18n.global.t('chat.errors.btwStreamTooLarge'))
      let boundary: RegExpExecArray | null
      while ((boundary = /\r?\n\r?\n/.exec(pending))) {
        const frame = pending.slice(0, boundary.index)
        pending = pending.slice(boundary.index + boundary[0].length)
        const answer = parseBtwFrame(frame)
        if (!answer) continue
        onAnswer(answer)
        if (answer.done) return
      }
    }
  } finally {
    await reader.cancel().catch(() => {})
    reader.releaseLock()
  }
}
