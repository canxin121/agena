export const DEFAULT_TRANSCRIPT_PART_PAGE_SIZE = 5
export const MIN_TRANSCRIPT_PART_PAGE_SIZE = 1
export const MAX_TRANSCRIPT_PART_PAGE_SIZE = 50

export const TRANSCRIPT_PART_PAGE_SIZE_OPTIONS = [5, 10, 20, 50] as const

export function normalizeTranscriptPartPageSize(value: unknown): number {
  const parsed = Number(value)
  if (!Number.isFinite(parsed)) return DEFAULT_TRANSCRIPT_PART_PAGE_SIZE
  return Math.max(MIN_TRANSCRIPT_PART_PAGE_SIZE, Math.min(MAX_TRANSCRIPT_PART_PAGE_SIZE, Math.floor(parsed)))
}
