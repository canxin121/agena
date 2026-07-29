import { describe, expect, test } from 'bun:test'
import { computed } from 'vue'

import {
  createChatCommandCatalog,
  parsePullRequestCommandArgs,
  type ChatCommandCatalogActions,
  type ChatCommandCatalogState,
} from './chatCommandCatalog'
import { commandMatchesSlash } from './commandPalette'
import { summarizeChatUsage } from '../pages/chatUsageModel'

function catalogFixture() {
  let renamedTo = ''
  let paletteOpened = false
  let skillPickerOpened = false
  let notice = ''
  const state: ChatCommandCatalogState = {
    selectedWorkspaceId: computed(() => 1),
    selectedSessionId: computed(() => 7),
    sessions: computed(() => []),
    messages: computed(() => []),
    composerQueue: computed(() => []),
    timelineEvents: computed(() => []),
    workspaces: computed(() => []),
    sessionImportJsonl: computed(() => ''),
    sessionTreeRows: computed(() => []),
    rewindCheckpoints: computed(() => []),
    ancestorSessions: computed(() => []),
    childSessions: computed(() => []),
    parentSession: computed(() => null),
    sessionState: computed(() => null),
    sessionUsageSummary: computed(() => summarizeChatUsage([])),
  }
  const noop = () => {}
  const actions: ChatCommandCatalogActions = {
    approvePermission: noop,
    askAside: noop,
    clearSessionGoalAction: noop,
    clearComposerQueue: noop,
    compactCurrentSession: noop,
    completeSessionGoalAction: noop,
    continueCurrentSession: noop,
    copyText: noop,
    createCommit: noop,
    createPullRequest: noop,
    createSessionAction: noop,
    downloadWorkspaceFile: noop,
    exportCurrentSession: noop,
    forgetMemory: noop,
    focusComposer: noop,
    focusTranscript: noop,
    focusRunOptions: noop,
    forkCurrentSession: noop,
    importSessionFromJsonl: noop,
    loadRewindCheckpoints: noop,
    loadSessionTimeline: noop,
    loadSessionTree: noop,
    openCommandPalette: () => {
      paletteOpened = true
    },
    openAttachmentPicker: noop,
    openSkillPicker: () => {
      skillPickerOpened = true
    },
    openMemorySettings: noop,
    openPermissionSettings: noop,
    openRuntimeSection: noop,
    openSessionById: async () => false,
    openWorkspaceBrowser: noop,
    openSnapshotInspector: noop,
    popComposerQueue: noop,
    runRuntimeEntry: noop,
    invokeRuntimeTool: noop,
    refreshConversation: noop,
    renameCurrentSession: (title) => {
      renamedTo = title || ''
    },
    resolveWorkspaceAction: noop,
    selectSession: noop,
    selectWorkspace: noop,
    setLocalCommandNotice: (value) => {
      notice = value
    },
    setNewSessionTitle: noop,
    setSessionGoalAction: noop,
    setSessionSearch: noop,
    setSessionViewMode: noop,
    setWorkspacePath: noop,
    showSessionGoalAction: noop,
  }

  return {
    actions,
    commands: createChatCommandCatalog(state, actions),
    paletteOpened: () => paletteOpened,
    skillPickerOpened: () => skillPickerOpened,
    renamedTo: () => renamedTo,
    notice: () => notice,
  }
}

describe('createChatCommandCatalog', () => {
  test('covers the supported TUI command spellings and aliases', () => {
    const fixture = catalogFixture()
    const expected = [
      '/help',
      '/?',
      '/commands',
      '/palette',
      '/new',
      '/clear',
      '/sessions',
      '/lineage',
      '/branch-history',
      '/branches',
      '/rewind',
      '/backtrack',
      '/rename',
      '/title',
      '/timeline',
      '/events',
      '/model',
      '/export',
      '/save',
      '/compact',
      '/compress',
      '/summarize',
      '/continue',
      '/resume-run',
      '/fork',
      '/branch',
      '/parent',
      '/children',
      '/child',
      '/status',
      '/allow',
      '/allow-always',
      '/deny',
      '/deny-always',
      '/user-input',
      '/reply',
      '/btw',
      '/aside',
      '/side',
      '/copy',
      '/yank',
      '/copy-message',
      '/copy-last',
      '/copy-assistant',
      '/copy-visible',
      '/skill',
      '/attach',
      '/file',
      '/image',
      '/paste-image',
      '/queue',
      '/q',
      '/memory',
      '/mem',
      '/review',
      '/snapshot',
      '/editor',
      '/edit',
      '/pager',
      '/view',
      '/less',
      '/commit',
      '/pr',
      '/diagnostics',
      '/feedback',
      '/permissions',
      '/permission',
      '/config',
      '/download',
      '/dl',
    ]

    for (const slash of expected) {
      expect(fixture.commands.some((command) => commandMatchesSlash(command, slash))).toBe(true)
    }
  })

  test('passes command arguments to rename and opens help for command aliases', async () => {
    const fixture = catalogFixture()
    const rename = fixture.commands.find((command) => commandMatchesSlash(command, '/rename'))
    const commands = fixture.commands.find((command) => commandMatchesSlash(command, '/commands'))

    await rename?.run({ input: '/rename Focused work', args: ['Focused', 'work'] })
    await commands?.run({ input: '/commands', args: [] })

    expect(fixture.renamedTo()).toBe('Focused work')
    expect(fixture.paletteOpened()).toBe(true)
    expect(fixture.notice()).toBe('')
  })

  test('opens the message-scoped Skill picker from the singular slash command', async () => {
    const fixture = catalogFixture()
    const skill = fixture.commands.find((command) => commandMatchesSlash(command, '/skill'))

    await skill?.run({ input: '/skill', args: [] })

    expect(fixture.skillPickerOpened()).toBe(true)
  })

  test('parses pull request options with multi-word values', () => {
    expect(
      parsePullRequestCommandArgs([
        'Finish',
        'web',
        'parity',
        '--body',
        'Adds',
        'settings',
        'and',
        'commands',
        '--base',
        'master',
        '--head',
        'feat/web',
      ]),
    ).toEqual({
      kind: 'create',
      title: 'Finish web parity',
      body: 'Adds settings and commands',
      base: 'master',
      head: 'feat/web',
    })
    expect(parsePullRequestCommandArgs(['Title', '--unknown', 'value'])).toEqual({
      kind: 'error',
      message: 'Unknown /pr option: --unknown',
    })
  })
})
