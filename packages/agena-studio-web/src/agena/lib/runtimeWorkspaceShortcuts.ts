export type WorkspaceShortcut = {
  id: string
  title: string
  description: string
  relativePath: string
}

export const workspaceShortcuts: WorkspaceShortcut[] = [
  {
    id: 'commands',
    title: 'Commands',
    description: 'Project-scoped markdown slash commands under .agena/commands/.',
    relativePath: '.agena/commands',
  },
  {
    id: 'skills',
    title: 'Skills',
    description: 'Project-scoped skills under .agena/skills/.',
    relativePath: '.agena/skills',
  },
  {
    id: 'agents',
    title: 'Agents',
    description: 'Custom subagent profiles under .agena/agents/.',
    relativePath: '.agena/agents',
  },
  {
    id: 'hooks',
    title: 'Hooks',
    description: 'Project automation and permission hook configuration.',
    relativePath: '.agena',
  },
  {
    id: 'plans',
    title: 'Plans',
    description: 'Generated plan documents under .agena/plans/.',
    relativePath: '.agena/plans',
  },
  {
    id: 'worktrees',
    title: 'Worktrees',
    description: 'Managed worktree directories under .agena/worktrees/.',
    relativePath: '.agena/worktrees',
  },
]
