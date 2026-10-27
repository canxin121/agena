import type { SkillInstallPreview, SkillInstallRequest, SkillInstallSource, SkillSummary } from './skillsApi'

export function skillWorkspaceUrl(path: string, workspaceId?: number) {
  if (workspaceId === undefined) return path
  if (!Number.isSafeInteger(workspaceId) || workspaceId <= 0) throw new Error('Invalid workspace ID')
  const separator = path.includes('?') ? '&' : '?'
  return path + separator + 'workspace_id=' + encodeURIComponent(String(workspaceId))
}

export type SkillStatusFilter = 'all' | 'enabled' | 'disabled' | 'invalid' | 'shadowed'

export function filterSkills(skills: readonly SkillSummary[], query: string, scope: string, status: string) {
  const terms = query.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean)
  return skills
    .filter((skill) => {
      if (scope !== 'all' && skill.scope !== scope) return false
      if (status === 'enabled' && (!skill.enabled || !skill.valid || skill.shadowed)) return false
      if (status === 'disabled' && skill.enabled) return false
      if (status === 'invalid' && skill.valid) return false
      if (status === 'shadowed' && !skill.shadowed) return false
      const text = [skill.name, skill.description, skill.path, skill.source?.location || '']
        .join('\n')
        .toLocaleLowerCase()
      return terms.every((term) => text.includes(term))
    })
    .sort((a, b) => a.name.localeCompare(b.name) || a.scope.localeCompare(b.scope) || a.path.localeCompare(b.path))
}

export function validSkillName(name: string) {
  const length = Array.from(name).length
  return length >= 1 && length <= 64 && /^[\p{Lowercase}\p{Number}]+(?:-[\p{Lowercase}\p{Number}]+)*$/u.test(name)
}

export function newSkillDocument(name: string, description: string, instructions: string) {
  // JSON strings are YAML-compatible scalars and preserve punctuation and newlines.
  return (
    '---\nname: ' +
    JSON.stringify(name) +
    '\ndescription: ' +
    JSON.stringify(description) +
    '\n---\n\n' +
    instructions +
    (instructions.endsWith('\n') ? '' : '\n')
  )
}

export function skillMarkdownBody(document: string) {
  return document.replace(/^\uFEFF?---\r?\n[\s\S]*?\r?\n---(?:\r?\n|$)/, '')
}

export function safeSkillResourcePath(path: string) {
  return (
    Boolean(path) &&
    !path.includes('\0') &&
    !path.includes('\\') &&
    !/^(\/|[A-Za-z]:)/.test(path) &&
    path.split('/').every((part) => part !== '' && part !== '.' && part !== '..')
  )
}

export function buildSelectedSkillInstall(
  request: Pick<SkillInstallRequest, 'scope' | 'source'>,
  preview: SkillInstallPreview,
  selected: readonly string[],
  replace: boolean,
): SkillInstallRequest | null {
  const chosen = [...new Set(selected)]
  if (!chosen.length || !preview.revision) return null
  const candidates = new Map(preview.candidates.map((candidate) => [candidate.path, candidate]))
  if (
    chosen.some((path) => {
      const candidate = candidates.get(path)
      return !candidate?.valid || (candidate.conflict && !replace)
    })
  )
    return null
  return { ...request, selected: chosen, replace, expected_source_revision: preview.revision }
}

export const MAX_SKILL_UPLOAD_BYTES = 32 * 1024 * 1024
export const MAX_SKILL_UPLOAD_FILES = 2000

type UploadFile = Pick<File, 'name' | 'size' | 'arrayBuffer'> & { webkitRelativePath?: string }
export type SkillUploadProblem = 'empty' | 'too_large' | 'too_many_files' | 'unsafe_path' | 'duplicate_path'

export function skillUploadProblem(files: readonly UploadFile[], folder: boolean): SkillUploadProblem | null {
  if (!files.length) return 'empty'
  if (files.length > MAX_SKILL_UPLOAD_FILES) return 'too_many_files'
  if (files.reduce((total, file) => total + file.size, 0) > MAX_SKILL_UPLOAD_BYTES) return 'too_large'
  const paths = files.map((file) => (folder ? file.webkitRelativePath || file.name : file.name))
  if (paths.some((path) => !safeSkillResourcePath(path))) return 'unsafe_path'
  if (new Set(paths).size !== paths.length) return 'duplicate_path'
  return null
}

function encodeBytes(buffer: ArrayBuffer) {
  const bytes = new Uint8Array(buffer)
  const parts: string[] = []
  for (let i = 0; i < bytes.length; i += 32768) {
    parts.push(String.fromCharCode(...bytes.subarray(i, i + 32768)))
  }
  return btoa(parts.join(''))
}

export async function skillUploadSource(files: readonly UploadFile[], folder: boolean): Promise<SkillInstallSource> {
  const problem = skillUploadProblem(files, folder)
  if (problem) throw new Error(problem)
  if (!folder) {
    if (files.length !== 1) throw new Error('too_many_files')
    const file = files[0]!
    return { kind: 'archive', filename: file.name, data_base64: encodeBytes(await file.arrayBuffer()) }
  }
  const uploaded: Array<{ path: string; content_base64: string }> = []
  // Read sequentially so a folder of large assets does not duplicate all buffers in memory.
  for (const file of files) {
    uploaded.push({
      path: file.webkitRelativePath || file.name,
      content_base64: encodeBytes(await file.arrayBuffer()),
    })
  }
  return { kind: 'files', files: uploaded }
}
