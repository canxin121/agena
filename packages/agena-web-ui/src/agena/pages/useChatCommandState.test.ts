import { describe, expect, test } from 'bun:test'

import type { CommandItem } from '../lib/commandPalette'
import { buildSlashSuggestions } from './useChatCommandState'

function command(id: string, title: string, slash: string): CommandItem {
  return {
    id,
    title,
    description: title,
    category: 'Chat Actions',
    source: 'chat-action',
    slash,
    usage: slash,
    run: () => {},
  }
}

describe('buildSlashSuggestions', () => {
  test('keeps the Skill picker visible and first in the bare slash panel', () => {
    const items = Array.from({ length: 20 }, (_, index) =>
      command(`chat.command-${index}`, `Command ${String(index).padStart(2, '0')}`, `/command-${index}`),
    )
    items.push(command('chat.attach-skill', 'Attach Skill', '/skill'))

    const suggestions = buildSlashSuggestions(items, '/', 10)

    expect(suggestions.length).toBe(10)
    expect(suggestions[0]?.id).toBe('chat.attach-skill')
    expect(suggestions.some((item) => item.slash === '/skill')).toBe(true)
  })

  test('ranks an exact typed slash ahead of the featured bare-slash entry', () => {
    const suggestions = buildSlashSuggestions(
      [command('chat.attach-skill', 'Attach Skill', '/skill'), command('chat.status', 'Status', '/status')],
      '/status',
    )

    expect(suggestions.map((item) => item.id)).toEqual(['chat.status'])
  })
})
