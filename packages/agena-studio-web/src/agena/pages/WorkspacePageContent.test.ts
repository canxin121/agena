import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'

const filePath = '/home/canxin/Git/ai/agena/packages/agena-studio-web/src/agena/pages/WorkspacePageContent.vue'

describe('WorkspacePageContent', () => {
  test('renders workspace cards, entry points, and file tree from page state', () => {
    const source = readFileSync(filePath, 'utf8')

    expect(source.includes('Resolve Workspace')).toBe(true)
    expect(source.includes('Project Status')).toBe(true)
    expect(source.includes('Current Workspace')).toBe(true)
    expect(source.includes('Project Entry Points')).toBe(true)
    expect(source.includes('Open Worktrees')).toBe(true)
    expect(source.includes('Open Logs')).toBe(true)
    expect(source.includes('File Tree')).toBe(true)
    expect(source.includes('props.workspace.workspaceConfigCards.value')).toBe(true)
    expect(source.includes('props.workspace.rows.value')).toBe(true)
  })
})
