import type { JsonValue } from '../types/json'

type JsonRecord = Record<string, JsonValue>

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : {}
}

function text(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

/** Match the public UserProblem contract, never Failure's model feedback or
 * diagnostic extensions. Older servers can still send the Failure envelope. */
function publicProblem(value: unknown): JsonRecord {
  const problem = record(value)
  const result: JsonRecord = {}
  for (const key of ['id', 'code', 'category', 'responsibility', 'retry', 'recovery', 'impact']) {
    const value = text(problem[key])
    if (value) result[key] = value
  }
  const user: JsonRecord = {}
  for (const key of ['key', 'fallback', 'detail_key']) {
    const value = text(record(problem.user)[key])
    if (value) user[key] = value
  }
  if (Object.keys(user).length) result.user = user
  return result
}

export function toolFailurePresentation(value: unknown): { message: string; details: JsonRecord | null } {
  const error = record(value)
  const candidates = [error.failure, error.problem, error]
  const details = candidates.map(publicProblem).find((candidate) => Object.keys(candidate).length > 0) ?? null
  const message =
    text(record(details?.user).fallback) ||
    // Legacy public error envelopes used message/detail instead of UserProblem.
    candidates.map((candidate) => text(record(candidate).message)).find(Boolean) ||
    text(error.detail)
  if (message && !text(record(details?.user).fallback)) return { message, details: { ...details, message } }
  return { message, details }
}
