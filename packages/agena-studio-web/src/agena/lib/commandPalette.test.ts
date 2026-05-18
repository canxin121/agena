import { describe, expect, test } from 'bun:test'
import { computed } from 'vue'

import { createCommandPalette } from './commandPalette'

function createRouterStub() {
  const pushes: string[] = []
  return {
    pushes,
    router: {
      push: (value: string) => {
        pushes.push(value)
        return Promise.resolve()
      },
    },
  }
}

describe('commandPalette', () => {
  test('includes navigation and runtime entries', () => {
    const { router } = createRouterStub()
    const palette = createCommandPalette({
      router: router as never,
      runtimeSkills: computed(() => [
        {
          name: 'review',
          description: 'Review code',
          aliases: ['audit'],
          source_path: '.agena/skills/review.md',
        },
      ]),
      runtimeCommands: computed(() => [
        {
          name: 'deploy',
          description: 'Deploy app',
          aliases: ['ship'],
          source_path: '.agena/commands/deploy.ts',
        },
      ]),
    })

    expect(palette.items.value.some((item) => item.id === 'nav.chat')).toBe(true)
    expect(palette.items.value.some((item) => item.id === 'runtime-command.deploy')).toBe(true)
    expect(palette.items.value.some((item) => item.id === 'runtime-skill.review')).toBe(true)
  })

  test('runs exact slash command match', async () => {
    const { pushes, router } = createRouterStub()
    const palette = createCommandPalette({
      router: router as never,
      runtimeSkills: computed(() => []),
      runtimeCommands: computed(() => []),
    })

    const result = await palette.runSlashCommand('/runtime')

    expect(result.matched).toBe(true)
    expect(result.command?.id).toBe('nav.runtime')
    expect(pushes).toEqual(['/runtime'])
  })

  test('runs section tab slash command match', async () => {
    const { pushes, router } = createRouterStub()
    const palette = createCommandPalette({
      router: router as never,
      runtimeSkills: computed(() => []),
      runtimeCommands: computed(() => []),
    })

    const result = await palette.runSlashCommand('/settings-desktop')

    expect(result.matched).toBe(true)
    expect(result.command?.id).toBe('nav.settings.desktop')
    expect(pushes).toEqual(['/settings/desktop'])
  })

  test('filters by aliases and descriptions', () => {
    const { router } = createRouterStub()
    const palette = createCommandPalette({
      router: router as never,
      runtimeSkills: computed(() => []),
      runtimeCommands: computed(() => []),
    })

    palette.query.value = 'marketplace'
    const ids = palette.filteredItems.value.map((item) => item.id)
    expect(ids.includes('nav.plugins')).toBe(true)
    expect(ids.includes('nav.plugins.marketplace')).toBe(true)
  })

  test('filters slash queries by prefix before exact args are present', () => {
    const { router } = createRouterStub()
    const palette = createCommandPalette({
      router: router as never,
      runtimeSkills: computed(() => []),
      runtimeCommands: computed(() => []),
      localCommands: computed(() => [
        {
          id: 'chat.new-session',
          title: 'New Session',
          description: 'Create a new session.',
          category: 'Chat Actions',
          source: 'chat-action',
          slash: '/new',
          usage: '/new [title]',
          run: () => {},
        },
      ]),
    })

    palette.query.value = '/ne'

    expect(palette.filteredItems.value.some((item) => item.id === 'chat.new-session')).toBe(true)
  })

  test('passes parsed slash args to local commands', async () => {
    const { router } = createRouterStub()
    const calls: string[] = []
    const palette = createCommandPalette({
      router: router as never,
      runtimeSkills: computed(() => []),
      runtimeCommands: computed(() => []),
      localCommands: computed(() => [
        {
          id: 'workspace.open-path',
          title: 'Open Workspace Path',
          description: 'Jump to a path.',
          category: 'Workspace Actions',
          source: 'workspace-action',
          slash: '/open-path',
          usage: '/open-path <relative-path>',
          run: async (context) => {
            calls.push(context?.args.join('/') || '')
          },
        },
      ]),
    })

    const result = await palette.runLocalSlashCommand('/open-path src agena pages')

    expect(result.matched).toBe(true)
    expect(result.command?.id).toBe('workspace.open-path')
    expect(calls).toEqual(['src/agena/pages'])
  })

  test('runs highlighted command with parsed slash args from palette query', async () => {
    const { router } = createRouterStub()
    const calls: string[] = []
    const palette = createCommandPalette({
      router: router as never,
      runtimeSkills: computed(() => []),
      runtimeCommands: computed(() => []),
      localCommands: computed(() => [
        {
          id: 'chat.open-session',
          title: 'Open Session by ID',
          description: 'Switch active session.',
          category: 'Chat Actions',
          source: 'chat-action',
          slash: '/open-session',
          usage: '/open-session <session-id>',
          run: async (context) => {
            calls.push(context?.args.join(',') || '')
          },
        },
      ]),
    })

    palette.openPalette()
    palette.query.value = '/open-session 8'

    const ran = await palette.runHighlighted()

    expect(ran).toBe(true)
    expect(calls).toEqual(['8'])
    expect(palette.open.value).toBe(false)
    expect(palette.query.value).toBe('')
  })

  test('routes runtime slash through the shared dispatcher', async () => {
    const { router } = createRouterStub()
    const selected: string[] = []
    const palette = createCommandPalette({
      router: router as never,
      runtimeSkills: computed(() => [
        {
          name: 'review',
          description: 'Review code',
          aliases: ['audit'],
          source_path: '.agena/skills/review.md',
        },
      ]),
      runtimeCommands: computed(() => []),
      localCommands: computed(() => []),
      onSelectRuntimeEntry: async ({ kind, item }) => {
        selected.push(`${kind}:${item.name}`)
      },
    })

    const result = await palette.runSlashCommand('/review src/app.ts')

    expect(result.matched).toBe(true)
    expect(result.command?.id).toBe('runtime-skill.review')
    expect(selected).toEqual(['skill:review'])
  })

  test('routes plugin studio slash through plugin action dispatcher', async () => {
    const { router } = createRouterStub()
    const selected: string[] = []
    const palette = createCommandPalette({
      router: router as never,
      runtimeSkills: computed(() => []),
      runtimeCommands: computed(() => []),
      pluginCommands: computed(() => [
        {
          plugin_id: 'project-helper',
          id: 'summarize-workspace',
          title: 'Summarize workspace',
          description: 'Run the project helper summary tool.',
          category: 'Project',
          slash: '/project-summary',
          aliases: ['workspace summary'],
          usage: '/project-summary [scope]',
          location: 'command_palette',
          action: {
            kind: 'invoke_tool',
            tool: 'summarize',
            submit_output_as_prompt: true,
          },
        },
      ]),
      localCommands: computed(() => []),
      onRunPluginAction: async ({ command, context }) => {
        selected.push(`${command.plugin_id}:${command.id}:${context?.args.join('/') || ''}`)
        return { submitText: 'summary prompt' }
      },
    })

    const result = await palette.runSlashCommand('/project-summary src agena')

    expect(result.matched).toBe(true)
    expect(result.command?.id).toBe('plugin-studio.project-helper.summarize-workspace')
    expect(result.result?.submitText).toBe('summary prompt')
    expect(selected).toEqual(['project-helper:summarize-workspace:src/agena'])
  })

  test('does not intercept runtime slash in local dispatcher', async () => {
    const { router } = createRouterStub()
    const palette = createCommandPalette({
      router: router as never,
      runtimeSkills: computed(() => [
        {
          name: 'review',
          description: 'Review code',
          aliases: ['audit'],
          source_path: '.agena/skills/review.md',
        },
      ]),
      runtimeCommands: computed(() => []),
      localCommands: computed(() => []),
    })

    const result = await palette.runLocalSlashCommand('/review src/app.ts')

    expect(result.matched).toBe(false)
  })
})
