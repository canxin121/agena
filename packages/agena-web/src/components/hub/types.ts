/**
 * Session hub wire types.
 *
 * SessionResource is the projection returned by GET /api/v1/sessions/overview
 * (and GET /api/v1/sessions). Types live in a .ts module — the repo's import
 * rules forbid exporting types from .vue files.
 */

export type SessionResource = {
  id: number
  parent_id?: number | null
  depth: number
  root_id: number
  workspace_id: number
  title?: string | null
  version: number
  relation_kind: 'root' | 'child' | 'fork' | 'rewind' | 'subagent'
  lifecycle_state: string
  state: 'creating' | 'ready' | 'running' | 'awaiting_user' | 'interrupted' | 'failed'
  is_subagent: boolean
  message_count: number
  child_session_count: number
  last_message_at?: string | null
  created_at: string
  updated_at: string
}

export type HubRowKind = 'attention' | 'running' | 'recent'
