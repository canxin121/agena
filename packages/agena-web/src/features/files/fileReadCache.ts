import { readFileChunk } from './api/filesApi'
import type { FsReadChunkResponse } from './api/filesApi'

type FileReadRequest = {
  directory: string
  path: string
  offset?: number
  limit?: number
}

type FileReadCacheOptions = {
  force?: boolean
}

type CacheEntry = {
  directory: string
  path: string
  value: FsReadChunkResponse
  bytes: number
}

// Keep content in memory while the app is alive so switching a workspace
// pane, or recreating its route component, does not download the same file
// again.  The bound prevents opening many large files from growing memory
// without limit; an evicted entry is simply fetched again when needed.
const MAX_CACHE_ENTRIES = 96
const MAX_CACHE_BYTES = 24 * 1024 * 1024

const cache = new Map<string, CacheEntry>()
const inFlight = new Map<string, { generation: number; promise: Promise<FsReadChunkResponse> }>()
let cacheBytes = 0
let cacheGeneration = 0

function normalizeNumber(value: number | undefined, fallback: number): number {
  return typeof value === 'number' && Number.isFinite(value) ? Math.max(0, Math.floor(value)) : fallback
}

function requestKey(input: FileReadRequest): string {
  return JSON.stringify([
    String(input.directory || '').trim(),
    String(input.path || '').trim(),
    normalizeNumber(input.offset, 0),
    normalizeNumber(input.limit, -1),
  ])
}

function touch(key: string, entry: CacheEntry) {
  cache.delete(key)
  cache.set(key, entry)
}

function setCache(key: string, value: FsReadChunkResponse, input: FileReadRequest) {
  const previous = cache.get(key)
  if (previous) cacheBytes -= previous.bytes

  const bytes = typeof value.content === 'string' ? value.content.length * 2 : 0
  cache.set(key, {
    directory: String(input.directory || '').trim(),
    path: String(input.path || '').trim(),
    value,
    bytes,
  })
  cacheBytes += bytes

  while (cache.size > MAX_CACHE_ENTRIES || cacheBytes > MAX_CACHE_BYTES) {
    const oldestKey = cache.keys().next().value
    if (typeof oldestKey !== 'string') break
    const oldest = cache.get(oldestKey)
    cache.delete(oldestKey)
    if (oldest) cacheBytes -= oldest.bytes
  }
}

export async function readFileChunkCached(
  input: FileReadRequest,
  opts?: FileReadCacheOptions,
): Promise<FsReadChunkResponse> {
  const key = requestKey(input)
  if (!opts?.force) {
    const cached = cache.get(key)
    if (cached) {
      touch(key, cached)
      return cached.value
    }
  }

  const existing = inFlight.get(key)
  if (existing && existing.generation === cacheGeneration) return existing.promise

  const generation = cacheGeneration
  const request = readFileChunk(input)
    .then((value) => {
      // A write/fs event may invalidate the request while it is in flight.
      // Do not let that old response repopulate the cache after invalidation.
      if (generation === cacheGeneration) setCache(key, value, input)
      return value
    })
    .finally(() => {
      if (inFlight.get(key)?.promise === request) inFlight.delete(key)
    })
  inFlight.set(key, { generation, promise: request })
  return request
}

export function invalidateFileReadCache(opts?: { directory?: string; paths?: string[] }) {
  cacheGeneration += 1
  const directory = String(opts?.directory || '').trim()
  const paths = new Set((opts?.paths || []).map((path) => String(path || '').trim()).filter(Boolean))

  for (const [key, entry] of cache) {
    if (directory && entry.directory !== directory) continue
    if (paths.size > 0 && !paths.has(entry.path)) continue
    cache.delete(key)
    cacheBytes -= entry.bytes
  }
}

export function clearFileReadCache() {
  cacheGeneration += 1
  cache.clear()
  inFlight.clear()
  cacheBytes = 0
}
