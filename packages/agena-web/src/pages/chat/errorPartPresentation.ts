import { i18n } from '@/i18n'
import { formatNumber, formatTimeHMS } from '@/i18n/intl'
import type { TranscriptDisplayPart } from '@/components/chat/messageList.types'
import { errorPresentation, jsonRecord, partStatusPresentation } from './transcriptPartPresentation'

function positiveNumber(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) && value > 0 ? value : null
}

export function retryDuration(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds))
  const t = i18n.global.t
  if (total < 60) return t('chat.errorPart.seconds', { count: formatNumber(total) })
  if (total < 3600)
    return t('chat.errorPart.minutes', {
      minutes: formatNumber(Math.floor(total / 60)),
      seconds: formatNumber(total % 60),
    })
  return t('chat.errorPart.hours', {
    hours: formatNumber(Math.floor(total / 3600)),
    minutes: formatNumber(Math.floor(total / 60) % 60),
  })
}

/** Only the clock is local. Retry ownership, phase, and outcome come from the Part. */
export function errorPartHeadline(part: TranscriptDisplayPart, nowMs: number) {
  const t = i18n.global.t
  const retry = jsonRecord(jsonRecord(part.source.agenaContent).retry)
  const message = errorPresentation(part).message
  const outcome = retry.outcome
  const attempt = Math.floor(positiveNumber(retry.attempt) ?? 0)
  const max = Math.floor(positiveNumber(retry.max_retries) ?? 0)
  const count = t(max ? 'chat.errorPart.retryCount' : 'chat.errorPart.retryNumber', {
    attempt: formatNumber(attempt),
    max: formatNumber(max),
  })
  const result = {
    title: part.title,
    message,
    progress: '',
    timing: '',
    scheduledAt: '',
    clockActive: false,
    status: partStatusPresentation(part.status),
  }
  if (outcome === 'retrying') {
    const requesting = retry.phase === 'requesting' || retry.phase === 'streaming'
    const nextAt = positiveNumber(retry.next_at_ms)
    const startedAt = positiveNumber(retry.started_at_ms)
    result.title = t(requesting ? 'chat.errorPart.retrying' : 'chat.errorPart.waiting')
    result.status = partStatusPresentation(requesting ? 'in_progress' : 'pending')
    // Backoff is waiting, so it does not use a spinning request indicator.
    if (!requesting) result.status = { ...result.status, icon: '◷', tone: 'warning', spinning: false }
    result.progress = count
    if (requesting) {
      result.timing = t(retry.phase === 'streaming' ? 'chat.errorPart.streaming' : 'chat.errorPart.requesting')
      if (startedAt)
        result.timing += ` · ${t('chat.errorPart.elapsed', { duration: retryDuration(Math.max(0, nowMs - startedAt) / 1000) })}`
      result.clockActive = startedAt !== null
    } else if (nextAt && nextAt > nowMs) {
      result.timing = t('chat.errorPart.inDuration', { duration: retryDuration(Math.ceil((nextAt - nowMs) / 1000)) })
      result.scheduledAt = t('chat.errorPart.scheduledAt', { time: formatTimeHMS(nextAt) })
      result.clockActive = true
    } else {
      // Old Parts have no phase. Never infer a network request from an expired deadline.
      result.timing = t('chat.errorPart.awaitingStart')
    }
  } else if (outcome === 'recovered') {
    result.title = t('chat.errorPart.recovered')
    result.status = partStatusPresentation('completed')
    result.progress = t('chat.errorPart.recoveredAfter', { attempt: formatNumber(attempt) })
  } else if (outcome === 'failed') {
    result.title = t('chat.errorPart.failed')
    result.progress = count
    result.timing = t(max && attempt >= max ? 'chat.errorPart.exhausted' : 'chat.errorPart.stopped', {
      max: formatNumber(max),
    })
  } else if (outcome === 'cancelled') {
    result.title = t('chat.errorPart.cancelled')
    result.status = partStatusPresentation('cancelled')
    result.progress = count
    result.timing = t('chat.errorPart.stopped')
  }
  return result
}
