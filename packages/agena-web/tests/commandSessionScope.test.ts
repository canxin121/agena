import { expect, test } from 'bun:test'
import {
  clientCommandsFromCatalog,
  commandAppliesToSession,
} from '../src/pages/chat/chatCommandsCatalog'
import type { PluginCommandCatalogItem } from '../src/lib/pluginOperations'

function command(
  id: string,
  applies_to?: PluginCommandCatalogItem['applies_to'],
): PluginCommandCatalogItem {
  return {
    plugin_id: 'agena.commands',
    accepts_empty_input: true,
    default_input: {},
    id,
    title: id,
    group: 'Built-in',
    slash: `/${id}`,
    aliases: [],
    applies_to,
    docs: {},
    input: {} as PluginCommandCatalogItem['input'],
    target: { kind: 'client', action: 'help' },
  }
}

test('an undeclared command serves every conversation kind', () => {
  expect(commandAppliesToSession(command('help'), false)).toBe(true)
  expect(commandAppliesToSession(command('help'), true)).toBe(true)
  expect(commandAppliesToSession({ applies_to: 'all' }, true)).toBe(true)
})

test('a declared kind is the only one that matches', () => {
  const newCommand = command('new', 'project')
  expect(commandAppliesToSession(newCommand, false)).toBe(true)
  expect(commandAppliesToSession(newCommand, true)).toBe(false)

  const botOnly = command('sessions', 'bot')
  expect(commandAppliesToSession(botOnly, true)).toBe(true)
  expect(commandAppliesToSession(botOnly, false)).toBe(false)

  const both = command('help', ['project', 'bot'])
  expect([commandAppliesToSession(both, false), commandAppliesToSession(both, true)]).toEqual([
    true,
    true,
  ])
})

test('the palette drops declarations that do not name this conversation', () => {
  const commands = [command('help'), command('new', 'project'), command('sessions', 'bot')]
  expect(clientCommandsFromCatalog(commands, true).map((entry) => entry.name)).toEqual([
    'help',
    'sessions',
  ])
  expect(clientCommandsFromCatalog(commands, false).map((entry) => entry.name)).toEqual([
    'help',
    'new',
  ])
})
