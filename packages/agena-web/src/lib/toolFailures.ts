import type { JsonValue } from '../types/json'

type JsonRecord = Record<string, JsonValue>

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : {}
}

function text(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

/** Project only the public UserProblem fields from a current failure.
 * Model feedback and diagnostic extensions never belong in user details. */
export function publicUserProblem(value: unknown): JsonRecord {
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
  const problem = publicUserProblem(record(value).failure)
  const details = Object.keys(problem).length ? problem : null
  return { message: text(record(problem.user).fallback), details }
}
