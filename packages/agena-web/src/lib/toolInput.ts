import type { JsonObject } from '../types/json'

/** Decode the accepted tool-input wire forms without changing nested values. */
export function toolInputRecord(value: unknown): JsonObject {
  let input = value
  if (typeof input === 'string') {
    try {
      input = JSON.parse(input)
    } catch {
      return {}
    }
  }
  return typeof input === 'object' && input !== null && !Array.isArray(input) ? (input as JsonObject) : {}
}
