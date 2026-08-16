import type { JsonValue } from '@/types/json'

export type MessagePartLike = {
  id?: string
  type?: string
  tool?: string
  state?: JsonValue
  text?: string
  content?: string
  url?: string
  serverPath?: string
  filename?: string
  mime?: string
  synthetic?: boolean
  ignored?: boolean
  partState?: string
  agenaKind?: string
  agenaRole?: string
  agenaSummary?: string | null
  agenaContent?: JsonValue
  runId?: number | null
  parentPartId?: number | null
  time?: { start?: number; end?: number }
  [k: string]: JsonValue
}

export type MessageLike = {
  info: {
    id?: string
    role?: string
    time?: { created?: number; completed?: number }
    finish?: string
    error?: JsonValue
    modelID?: string
    providerID?: string
    adapterID?: string
    runId?: number
    runState?: string
    runContent?: JsonValue
    [k: string]: JsonValue
  }
  parts: MessagePartLike[]
}

/**
 * Presentation kinds mirror agena-tui-transcript's render model. `answer`
 * and `text_segment` are projections of ordinary durable `text` parts: the
 * last assistant text with no later operation is the Answer, while earlier
 * working prose remains a collapsible TextSegment.
 */
export type TranscriptPartKind =
  | 'text'
  | 'answer'
  | 'text_segment'
  | 'reasoning'
  | 'operation'
  | 'resource'
  | 'skill'
  | 'notice'
  | 'compaction'
  | 'interaction'
  | 'lifecycle'
  | 'error'
  | 'unknown'

export type AttentionLike = {
  kind: 'permission' | 'question'
  payload: JsonValue
} | null

export type TranscriptDisplayPart = {
  key: string
  id: string
  kind: TranscriptPartKind
  status: string
  role: string
  source: MessagePartLike
  title: string
  summary: string
  copyText: string
  toggleable: boolean
  defaultExpanded: boolean
}

export type RevertLike = {
  messageID: string
  revertedUserCount: number
  diffFiles: Array<{ filename: string; additions: number; deletions: number }>
}

export type RetryStatusLike = {
  next?: number
  attempt?: number
  message?: string
  [k: string]: JsonValue
} | null

export type SessionErrorLike = {
  at: number
  error: {
    message: string
    rendered?: string
    code?: string
    classification?: string
  }
} | null

export type MessageRenderBlock = {
  kind: 'message'
  key: string
  message: MessageLike
  displayParts: TranscriptDisplayPart[]
  runIds: string[]
  hasActivity: boolean
}

export type RenderBlock = MessageRenderBlock | { kind: 'revert'; key: string; revert: RevertLike }
