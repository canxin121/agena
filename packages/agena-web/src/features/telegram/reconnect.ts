import type { CreateIntegration, IntegrationBinding } from '@/generated/agenaState'
import type { integrationApi } from '@/lib/integrationApi'

export type TelegramReconnectAttempt = {
  source: number
  originalId: string
  originalRevision: number
  notBeforeMs: number
  request: CreateIntegration
}
export const telegramReconnectKey = (id: string) => `telegram-reconnect:${id}`
export function telegramBindingNeedsReview(binding: IntegrationBinding): boolean {
  // Access edits can clear last_error while leaving a faulted connection.
  // Review is an explicit, server-validated recovery of the existing history.
  return binding.state === 'faulted' && binding.spec.mode === 'channel'
}
export function telegramReconnectAttempt(binding: IntegrationBinding, now = Date.now()): TelegramReconnectAttempt {
  if (binding.spec.provider !== 'agena.telegram' || binding.spec.mode !== 'channel')
    throw new Error('Only a Telegram channel binding can be rebuilt here.')
  return {
    source: binding.source_session_id,
    originalId: binding.id,
    originalRevision: binding.revision,
    notBeforeMs: now,
    request: {
      request_key: telegramReconnectKey(binding.id),
      spec: JSON.parse(JSON.stringify(binding.spec)) as IntegrationBinding['spec'],
    },
  }
}
export async function reconnectTelegramBinding(
  api: Pick<typeof integrationApi, 'create' | 'detail' | 'control'>,
  attempt: TelegramReconnectAttempt,
): Promise<IntegrationBinding> {
  const original = (await api.detail(attempt.source, attempt.originalId)).binding
  if (
    JSON.stringify(original.spec) !== JSON.stringify(attempt.request.spec) ||
    (original.revision !== attempt.originalRevision && original.state !== 'paused')
  )
    throw new Error('This connection changed. Review its latest settings before rebuilding it.')
  // The replacement starts disabled. Its stable request key allows retries
  // after any network failure without creating multiple replacement routes.
  let replacement = await api.create(attempt.source, attempt.request)
  if (JSON.stringify(replacement.spec) !== JSON.stringify(attempt.request.spec))
    throw new Error('A replacement with different settings already exists. Review that connection first.')
  if (original.state !== 'paused')
    await api.control(attempt.source, original.id, { action: 'pause', expected_revision: original.revision })
  if (replacement.state !== 'enabled')
    replacement = await api.control(attempt.source, replacement.id, {
      action: 'enable',
      expected_revision: replacement.revision,
      not_before_ms: attempt.notBeforeMs,
    })
  return replacement
}
