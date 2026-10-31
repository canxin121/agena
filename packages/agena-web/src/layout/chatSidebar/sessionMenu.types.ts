import type { ComponentPublicInstance } from 'vue'
import type { DirectoryEntry } from '@/features/sessions/model/types'
import type { JsonValue } from '@/types/json'

/** Session fields used by sidebar rows, independent of the full chat state. */
export type SidebarSession = {
  id: string
  title?: string
  slug?: string
  directory?: string
  time?: { updated?: number | null } | null
  [k: string]: JsonValue
}

export type SessionMenuTarget = {
  directory: DirectoryEntry
  session: { id: string; favorite?: boolean }
}

export type MenuRefEl = Element | ComponentPublicInstance | null

export type SessionToggleSelectionOptions = {
  event?: MouseEvent
  orderedSessionIds?: string[]
}

export type DirectoryToggleSelectionOptions = {
  event?: MouseEvent
  orderedDirectoryIds?: string[]
}
