/** Literal persisted tool paths and operation diffs; never current Git state. */
export interface SessionFileEdit {
  part_id: number
  tool: string
  kind: string
  from_path: string | null
  before_sha256: string | null
  after_sha256: string | null
  diff: string | null
  diff_truncated: boolean
  diff_scope: 'file' | 'operation'
  diff_unavailable_reason: string | null
}
export interface SessionFileChange {
  path: string
  operation_count: number
  operation_history: boolean
  operations: SessionFileEdit[]
}
export interface SessionFileChanges {
  files: SessionFileChange[]
  total_files: number
  offset: number
  has_more: boolean
  recording_incomplete: boolean
}
