import { apiJson } from './api'
import { skillWorkspaceUrl } from './skills'

export type SkillScope = 'global' | 'workspace'
export type SkillSource = {
  kind: string
  location: string
  reference: string | null
  subpath: string | null
  revision: string | null
}
export type SkillSummary = {
  id: string
  name: string
  description: string
  scope: SkillScope
  path: string
  editable: boolean
  enabled: boolean
  valid: boolean
  shadowed: boolean
  revision: string
  source: SkillSource | null
  diagnostics: string[]
}
export type SkillMetadata = {
  name: string
  description: string
  license: string | null
  compatibility: string | null
  metadata: Record<string, string>
  allowed_tools: string | null
  aliases: string[]
}
export type SkillCatalog = {
  skills: SkillSummary[]
  diagnostics: Array<{ path: string; message: string }>
  roots: Array<{ path: string; scope: SkillScope; editable: boolean }>
}
export type SkillDetail = SkillSummary & {
  document: string
  metadata: SkillMetadata | null
  files: Array<{ path: string; size: number }>
}
export type SkillResource = { path: string; content: string; revision: string }
export type SkillValidation = {
  valid: boolean
  metadata: SkillMetadata | null
  errors: string[]
  warnings: string[]
}
export type SkillInstallSource =
  | { kind: 'github'; url: string; reference?: string; subpath?: string }
  | { kind: 'folder'; path: string }
  | { kind: 'archive'; path?: string; url?: string; filename?: string; data_base64?: string }
  | { kind: 'files'; files: Array<{ path: string; content_base64: string; executable?: boolean }> }
export type SkillInstallRequest = {
  scope: SkillScope
  source: SkillInstallSource
  selected?: string[]
  replace?: boolean
  expected_source_revision?: string
}
export type SkillInstallPreview = {
  candidates: Array<{
    path: string
    name: string
    description: string
    valid: boolean
    diagnostics: string[]
    conflict: boolean
  }>
  revision: string
}
export type SkillInstallResult = { installed: SkillSummary[] }

export function createSkillsClient(workspaceId?: number) {
  const root = '/api/v1/skills'
  const url = (path: string) => skillWorkspaceUrl(path, workspaceId)
  const skillPath = (id: string) => root + '/' + encodeURIComponent(id)
  const json = <T>(path: string, method: string, body: unknown, signal?: AbortSignal) =>
    apiJson<T>(url(path), {
      method,
      signal,
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(body),
    })
  return {
    listSkills: (signal?: AbortSignal) => apiJson<SkillCatalog>(url(root), { signal }),
    readSkill: (id: string, signal?: AbortSignal) => apiJson<SkillDetail>(url(skillPath(id)), { signal }),
    createSkill: (scope: SkillScope, document: string) => json<SkillDetail>(root, 'POST', { scope, document }),
    updateSkill: (id: string, document: string, expected_revision: string) =>
      json<SkillDetail>(skillPath(id), 'PUT', { document, expected_revision }),
    removeSkill: (id: string, revision: string) =>
      apiJson<{ removed: boolean; id: string }>(
        url(skillPath(id) + '?expected_revision=' + encodeURIComponent(revision)),
        { method: 'DELETE' },
      ),
    setSkillEnabled: (id: string, enabled: boolean, expected_revision: string) =>
      json<SkillDetail>(skillPath(id) + '/enabled', 'PUT', { enabled, expected_revision }),
    readSkillResource: (id: string, path: string, signal?: AbortSignal) =>
      apiJson<SkillResource>(url(skillPath(id) + '/resource?path=' + encodeURIComponent(path)), { signal }),
    writeSkillResource: (id: string, path: string, content: string, expected_revision?: string) =>
      json<SkillResource>(skillPath(id) + '/resource', 'PUT', { path, content, expected_revision }),
    removeSkillResource: (id: string, path: string, revision: string) =>
      apiJson<SkillDetail>(
        url(
          skillPath(id) +
            '/resource?path=' +
            encodeURIComponent(path) +
            '&expected_revision=' +
            encodeURIComponent(revision),
        ),
        { method: 'DELETE' },
      ),
    validateSkill: (document: string, name?: string) =>
      json<SkillValidation>(root + '/validate', 'POST', { document, name }),
    previewSkillInstall: (request: SkillInstallRequest, signal?: AbortSignal) =>
      json<SkillInstallPreview>(root + '/install/preview', 'POST', request, signal),
    installSkills: (request: SkillInstallRequest) => json<SkillInstallResult>(root + '/install', 'POST', request),
  }
}
export type SkillsClient = ReturnType<typeof createSkillsClient>
