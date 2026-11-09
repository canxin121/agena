import { describe, expect, test } from 'bun:test'
import { reconcileBots, searchBots } from '../src/features/bots/profiles'
import type { BotProfile } from '../src/lib/bots'
const bot = (id: string, name: string, revision = 1, archived = false): BotProfile => ({ id, name, revision, archived, persona: { instructions: '' }, session_id: 1 } as BotProfile)
describe('Bot sidebar profiles', () => {
  test('search normalizes whitespace and case and sorts names', () => {
    expect(searchBots([bot('2', 'Zed'), bot('1', 'Alpha  Bot')], ' ALPHA bot ').map(p => p.id)).toEqual(['1'])
    expect(searchBots([bot('2', 'Zed'), bot('1', 'Alpha')], '').map(p => p.id)).toEqual(['1', '2'])
  })
  test('deduplicates IDs and protects writes from stale lists', () => {
    const updated = bot('A', 'New', 3)
    expect(reconcileBots([bot('a', 'Old', 1), bot('A', 'Older', 2)], new Map([['a', updated]]))).toEqual([updated])
    expect(reconcileBots([bot('a', 'Old', 1)], new Map([['a', bot('A', 'New', 3, true)] ]))).toEqual([])
    expect(reconcileBots([bot('a', 'Newer', 4)], new Map([['a', updated]])).map(p => p.name)).toEqual(['Newer'])
  })
})

test('search includes instructions', () => {
  const profile = bot('1', 'Atlas')
  profile.persona.instructions = 'Translate carefully'
  expect(searchBots([profile], 'carefully')).toEqual([profile])
})
