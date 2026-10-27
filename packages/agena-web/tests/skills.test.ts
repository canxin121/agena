import { describe, expect, test } from 'bun:test'
import {
  buildSelectedSkillInstall,
  filterSkills,
  MAX_SKILL_UPLOAD_BYTES,
  MAX_SKILL_UPLOAD_FILES,
  newSkillDocument,
  safeSkillResourcePath,
  skillMarkdownBody,
  skillUploadProblem,
  skillUploadSource,
  skillWorkspaceUrl,
  validSkillName,
} from '../src/lib/skills'
import type { SkillInstallPreview, SkillSummary } from '../src/lib/skillsApi'

function skill(overrides: Partial<SkillSummary> = {}): SkillSummary {
  return {
    id: 'one',
    name: 'example',
    description: 'Analyze source files',
    scope: 'workspace',
    path: '/project/.agents/skills/example',
    editable: true,
    enabled: true,
    valid: true,
    shadowed: false,
    revision: 'v1',
    source: null,
    diagnostics: [],
    ...overrides,
  }
}
const preview: SkillInstallPreview = {
  revision: 'source-revision',
  candidates: [
    { path: 'new', name: 'new', description: '', valid: true, diagnostics: [], conflict: false },
    { path: 'existing', name: 'existing', description: '', valid: true, diagnostics: [], conflict: true },
    {
      path: 'broken',
      name: 'broken',
      description: '',
      valid: false,
      diagnostics: ['Invalid metadata'],
      conflict: false,
    },
  ],
}
const request = { scope: 'workspace' as const, source: { kind: 'folder' as const, path: '/source' } }

describe('skill management helpers', () => {
  test('binds requests to the selected workspace without dropping revision or resource queries', () => {
    expect(skillWorkspaceUrl('/api/v1/skills', 23)).toBe('/api/v1/skills?workspace_id=23')
    expect(skillWorkspaceUrl('/api/v1/skills/id/resource?path=references%2Fa.md&expected_revision=v1', 23)).toBe(
      '/api/v1/skills/id/resource?path=references%2Fa.md&expected_revision=v1&workspace_id=23',
    )
    expect(skillWorkspaceUrl('/api/v1/skills')).toBe('/api/v1/skills')
    expect(() => skillWorkspaceUrl('/api/v1/skills', 0)).toThrow()
    expect(() => skillWorkspaceUrl('/api/v1/skills', Number.NaN)).toThrow()
  })

  test('validates Unicode names by codepoints and safely quotes frontmatter', () => {
    for (const name of ['my-skill', 'résumé-2', 'αβγ-٢']) expect(validSkillName(name)).toBe(true)
    for (const name of ['', '-start', 'end-', 'two--hyphens', 'Uppercase', '../outside', 'a'.repeat(65)])
      expect(validSkillName(name)).toBe(false)
    const description = 'Quotes: "x"\n---\nname: injected'
    const document = newSkillDocument('my-skill', description, '# Instructions\n\nExact body.')
    expect(document).toContain('description: ' + JSON.stringify(description) + '\n---\n')
    expect(skillMarkdownBody(document)).toBe('\n# Instructions\n\nExact body.\n')
    expect(skillMarkdownBody('\uFEFF---\r\nname: example\r\n---\r\nBody')).toBe('Body')
  })

  test('separates active skills from disabled, invalid, and overridden entries', () => {
    const skills = [
      skill(),
      skill({ id: 'disabled', enabled: false }),
      skill({ id: 'invalid', valid: false }),
      skill({ id: 'shadowed', shadowed: true, scope: 'global' }),
    ]
    expect(filterSkills(skills, 'source example', 'all', 'enabled').map((item) => item.id)).toEqual(['one'])
    expect(filterSkills(skills, '', 'global', 'all').map((item) => item.id)).toEqual(['shadowed'])
    expect(filterSkills(skills, '', 'all', 'invalid').map((item) => item.id)).toEqual(['invalid'])
    expect(filterSkills(skills, 'missing', 'all', 'all')).toEqual([])
  })

  test('never submits an empty or unreviewed installation selection', () => {
    expect(buildSelectedSkillInstall(request, preview, [], false)).toBeNull()
    expect(buildSelectedSkillInstall(request, preview, ['unknown'], false)).toBeNull()
    expect(buildSelectedSkillInstall(request, preview, ['broken'], true)).toBeNull()
    expect(buildSelectedSkillInstall(request, preview, ['existing'], false)).toBeNull()
    expect(buildSelectedSkillInstall(request, { ...preview, revision: '' }, ['new'], false)).toBeNull()
    expect(buildSelectedSkillInstall(request, preview, ['new', 'existing', 'new'], true)).toEqual({
      ...request,
      selected: ['new', 'existing'],
      replace: true,
      expected_source_revision: 'source-revision',
    })
  })

  test('rejects traversal, ambiguous paths, and duplicates before reading file bytes', () => {
    for (const path of ['/absolute', '../outside', 'a/../b', 'a//b', 'a\\b', 'C:/outside', 'a/\0b'])
      expect(safeSkillResourcePath(path)).toBe(false)
    expect(safeSkillResourcePath('skills/résumé/references/guide.md')).toBe(true)
    const file = new File(['instructions'], 'SKILL.md')
    expect(skillUploadProblem([file, file], true)).toBe('duplicate_path')
    const fake = {
      name: 'large.zip',
      size: MAX_SKILL_UPLOAD_BYTES + 1,
      arrayBuffer: () => {
        throw new Error('Must not read')
      },
    }
    expect(skillUploadProblem([fake], false)).toBe('too_large')
    const many = Array.from({ length: MAX_SKILL_UPLOAD_FILES + 1 }, (_, i) => ({ ...fake, name: String(i), size: 0 }))
    expect(skillUploadProblem(many, true)).toBe('too_many_files')
  })

  test('preserves binary archive bytes and folder-relative paths', async () => {
    const archive = new File([new Uint8Array([0, 1, 2, 255])], 'package.zip')
    expect(await skillUploadSource([archive], false)).toEqual({
      kind: 'archive',
      filename: 'package.zip',
      data_base64: 'AAEC/w==',
    })
    const resource = new File(['exact\ntext\n'], 'SKILL.md')
    Object.defineProperty(resource, 'webkitRelativePath', { value: 'bundle/example/SKILL.md' })
    expect(await skillUploadSource([resource], true)).toEqual({
      kind: 'files',
      files: [{ path: 'bundle/example/SKILL.md', content_base64: btoa('exact\ntext\n') }],
    })
  })
})
