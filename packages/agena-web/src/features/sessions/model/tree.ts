import type { JsonValue as JsonLike } from '@/types/json'

export type SessionLike = {
  id: string
  title?: string
  slug?: string
  directory?: string
  time?: { updated?: number | null } | null
  [k: string]: JsonLike
}

export type FlatTreeRow = {
  id: string
  session: SessionLike
  depth: number
  isParent: boolean
  isExpanded: boolean
  rootId: string
}
