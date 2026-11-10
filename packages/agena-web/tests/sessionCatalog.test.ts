import { describe, expect, test } from 'bun:test'
import {
  groupSessionsByProject,
  sessionGroupsForPicker,
  sessionProjectLabel,
} from '../src/components/settings/sessionCatalog'
import { directoriesForPage, PROJECTS_PER_PAGE } from '../src/components/settings/projectCatalog'
import type { Session } from '../src/types/chat'
import type { DirectoryEntry } from '../src/features/sessions/model/types'

function session(id: string, workspaceId: number | undefined, title: string): Session {
  return { id, title, workspace_id: workspaceId }
}

const paths: Record<string, string> = { '1': '/home/me/alpha', '2': '/home/me/beta' }

describe('session grouping by project', () => {
  test('groups conversations under their project and keeps global ones last', () => {
    const groups = groupSessionsByProject(
      [
        session('10', 2, 'beta one'),
        session('11', undefined, 'bot chat'),
        session('12', 1, 'alpha one'),
        session('13', 2, 'beta two'),
      ],
      (item) => (item.workspace_id ? paths[String(item.workspace_id)] : null),
    )
    expect(groups.slice(0, 2).map((group) => group.label)).toEqual(['/home/me/beta', '/home/me/alpha'])
    expect(groups[0].sessions.map((item) => item.id)).toEqual(['10', '13'])
    expect(groups.at(-1)?.workspaceId).toBeNull()
  })

  test('a session without a known path still gets a stable project label', () => {
    expect(sessionProjectLabel(session('10', 7, 'x'))).toBe('#7')
    // A global conversation has no project; the label is the localized default.
    expect(sessionProjectLabel(session('11', undefined, 'x')).trim().length).toBeGreaterThan(0)
  })

  test('picker groups are collapsible, counted, and mark the selected row', () => {
    const groups = groupSessionsByProject(
      [session('10', 1, 'alpha one'), session('11', 1, 'alpha two')],
      () => '/home/me/alpha',
    )
    const picker = sessionGroupsForPicker(groups, { selectedId: '11' })
    expect(picker).toHaveLength(1)
    expect(picker[0].title).toBe('/home/me/alpha')
    expect(picker[0].subtitle).toContain('2')
    expect(picker[0].items.map((item) => item.id)).toEqual(['10', '11'])
    expect(picker[0].items.find((item) => item.id === '11')?.checked).toBe(true)
    expect(picker[0].items.find((item) => item.id === '10')?.checked).toBe(false)
  })
})

describe('project catalog paging', () => {
  test('resolves page rows through the directory map without inventing entries', () => {
    const byId: Record<string, DirectoryEntry> = {
      '1': { id: '1', path: '/home/me/alpha' },
      '2': { id: '2', path: '/home/me/beta' },
    }
    expect(directoriesForPage([{ id: '1', path: '' }, { id: 'gone', path: '' }], byId).map((e) => e.id)).toEqual(['1'])
    expect(PROJECTS_PER_PAGE).toBeGreaterThan(0)
  })
})
