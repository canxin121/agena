<script setup>
// Production UI components and state; all business actions stay in this isolated page.
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { useUiStore } from '../../src/stores/ui'
import { useSettingsStore } from '../../src/stores/settings'
import { useChatStore } from '../../src/stores/chat'
import { useWorkspacePreviewStore } from '../../src/stores/workspacePreview'
import { useDirectoryStore } from '../../src/stores/directory'
import { router } from '../../src/router'
import { installKeyboardShortcuts } from '../../src/app/runtime/installKeyboardShortcuts'
import { vKeyboardCollection } from '../../src/lib/keyboardCollection'
import { deferKeyboardFocus } from '../../src/lib/deferredKeyboardFocus'
import FilesExplorerPane from '../../src/pages/files/components/FilesExplorerPane.vue'
import FilesMoveDialog from '../../src/pages/files/components/FilesMoveDialog.vue'
import SettingsSidebarNavList from '../../src/components/settings/sidebar/SettingsSidebarNavList.vue'
import DirectoryRow from '../../src/layout/chatSidebar/components/DirectoryRow.vue'
import SessionRow from '../../src/layout/chatSidebar/components/SessionRow.vue'
import GitChangesSection from '../../src/components/git/GitChangesSection.vue'
import GitHistorySection from '../../src/components/git/GitHistorySection.vue'
import AttachmentsPanel from '../../src/components/chat/AttachmentsPanel.vue'
import ImageViewerDialog from '../../src/components/ImageViewerDialog.vue'
import TranscriptKeyboardToolbar from '../../src/components/chat/TranscriptKeyboardToolbar.vue'
import MessageItem from '../../src/components/chat/MessageItem.vue'
import WorkspaceEditorGroupPane from '../../src/layout/WorkspaceEditorGroupPane.vue'
import WorkspacePreviewDockPanel from '../../src/features/workspacePreview/components/WorkspacePreviewDockPanel.vue'
import HelpDialog from '../../src/components/HelpDialog.vue'
import FormDialog from '../../src/components/ui/FormDialog.vue'
import ConfirmPopover from '../../src/components/ui/ConfirmPopover.vue'
import CommandPalette from '../../src/components/chat/CommandPalette.vue'
import PromptHistoryPalette from '../../src/components/chat/PromptHistoryPalette.vue'
import Composer from '../../src/components/chat/Composer.vue'
import { RiTerminalBoxLine } from '@remixicon/vue'

const ui = useUiStore(),
  settings = useSettingsStore(),
  chat = useChatStore()
const preview = useWorkspacePreviewStore(),
  directory = useDirectoryStore()
settings.data = { chatTranscriptVim: false }
settings.save = async (patch) => Object.assign(settings.data, patch)
chat.createSession = async () => {
  throw new Error('Business writes are not part of this fixture')
}
preview.refreshSessions = async () => {}
preview.retainLiveSessions = () => () => {}
directory.currentDirectory = '/workspace'
preview.sessions = [
  {
    id: 'fixture',
    state: 'running',
    directory: '/workspace',
    runDirectory: '/workspace',
    proxyBasePath: '/api/v1/workbench/preview/s/fixture/',
    targetUrl: 'http://fixture.invalid',
    command: 'fixture',
    args: ['fixture'],
    logsPath: '/fixture.log',
  },
]
preview.selectSession('fixture')
preview.setViewportSize({ width: 390, height: 300 })
ui.closeAllWorkspaceWindows()
ui.openWorkspaceWindow('files', { title: 'Files alpha', reuseExisting: false })
ui.openWorkspaceWindow('git', { title: 'Git beta', reuseExisting: false })
const state = reactive({
  expanded: false,
  fileSelected: '',
  fileSelection: [],
  multi: false,
  renameFail: false,
  renamed: '',
  moved: [],
  moveOpen: false,
  moveInput: 'folder',
  moveError: '',
  moveFails: false,
  compact: false,
  directoryCollapsed: false,
  threadExpanded: false,
  sessionSelected: '',
  sessionAction: '',
  settingsExpanded: false,
  setting: '',
  gitSelected: [],
  gitAnchor: '',
  gitExpanded: true,
  gitStaged: [],
  openedFile: '',
  selectedCommit: '',
  attachmentsOpen: false,
  attachProjectOpen: false,
  attached: [
    {
      id: 'image',
      filename: 'corner-test.svg',
      mime: 'image/svg+xml',
      size: 300,
      url:
        'data:image/svg+xml,' +
        encodeURIComponent(
          '<svg xmlns="http://www.w3.org/2000/svg" width="1400" height="1000"><rect width="1400" height="1000" fill="#174c65"/><path d="M0 0L1400 1000M1400 0L0 1000" stroke="#ffcd40" stroke-width="40"/><text x="80" y="140" fill="white" font-size="80">KEYBOARD IMAGE</text></svg>',
        ),
    },
  ],
  copied: '',
  searched: 0,
  older: false,
  plan: false,
  viewer: 'responsive',
  palette: false,
  commandIndex: 0,
  historyOpen: false,
  historyIndex: 0,
  historyQuery: '',
  historySelected: '',
  draft: '',
  deferredReady: false,
  deferredMounted: false,
  routeSource: true,
  routeReady: false,
  deletionRows: ['alpha', 'beta', 'gamma'],
  collectionDialog: false,
  deleteTarget: '',
  composerHidden: false,
  composerMounted: false,
})
const alpha = reactive({ name: 'alpha.txt', path: '/workspace/alpha.txt', type: 'file', extension: 'txt' })
const folder = { name: 'folder', path: '/workspace/folder', type: 'directory' }
const child = { name: 'child.txt', path: '/workspace/folder/child.txt', type: 'file', extension: 'txt' }
const zeta = { name: 'zeta.txt', path: '/workspace/zeta.txt', type: 'file', extension: 'txt' }
const tree = computed(() => [
  { node: folder, depth: 0, isExpanded: state.expanded, isLoading: false },
  ...(state.expanded ? [{ node: child, depth: 1, isExpanded: false, isLoading: false }] : []),
  ...[alpha, zeta].map((node) => ({ node, depth: 0, isExpanded: false, isLoading: false })),
])
const noop = () => {},
  success = async () => true
function clickNode(node, modifiers) {
  if (state.multi) {
    if (modifiers.range)
      state.fileSelection = tree.value
        .map((row) => row.node.path)
        .slice(0, tree.value.findIndex((row) => row.node.path === node.path) + 1)
    else
      state.fileSelection = state.fileSelection.includes(node.path)
        ? state.fileSelection.filter((path) => path !== node.path)
        : [...state.fileSelection, node.path]
  } else if (node.type === 'directory') state.expanded = !state.expanded
  else state.fileSelected = node.path
}
async function rename(node, name) {
  if (state.renameFail) return false
  node.name = name
  node.path = '/workspace/' + name
  state.renamed = name
  return true
}
const filesProps = computed(() => ({
  root: '/workspace',
  isCompactLayout: state.compact,
  isCompactTouch: state.compact,
  selectedFilePath: state.fileSelected,
  activeCreateDir: '/workspace',
  hasRootChildren: true,
  flattenedTree: tree.value,
  deletingPaths: new Set(),
  selectedPaths: new Set(state.fileSelection),
  multiSelectEnabled: state.multi,
  uploading: false,
  toggleMultiSelectMode: () => {
    state.multi = !state.multi
  },
  selectAllVisibleNodes: noop,
  invertVisibleNodeSelection: noop,
  handleNodeClick: clickNode,
  handleNodeLongPress: noop,
  refreshRoot: noop,
  collapseAll: () => {
    state.expanded = false
  },
  expandDirectory: () => {
    state.expanded = true
  },
  toggleDirectory: () => {
    state.expanded = !state.expanded
  },
  createNode: success,
  renameNode: rename,
  moveNodeByDrag: success,
  runFileAction: noop,
  openDialog: (kind) => {
    state.sessionAction = kind
  },
  deleteNode: noop,
  deleteSelectedNodes: noop,
  openMoveSelectedDialog: (paths) => {
    state.moved = paths
    state.moveOpen = true
  },
  clearSelectedPaths: noop,
  uploadFiles: noop,
}))
const settingGroups = computed(() => [
  {
    id: 'application',
    labelKey: 'settings.groups.application',
    items: [
      {
        key: 'interface',
        section: 'interface',
        label: 'Appearance',
        depth: 0,
        active: false,
        hasChildren: true,
        expanded: state.settingsExpanded,
      },
      ...(state.settingsExpanded
        ? [
            {
              key: 'theme',
              section: 'interface',
              label: 'Theme',
              depth: 1,
              active: state.setting === 'theme',
              hasChildren: false,
              expanded: false,
            },
          ]
        : []),
      {
        key: 'diagnostics',
        section: 'diagnostics',
        label: 'Diagnostics',
        depth: 0,
        active: state.setting === 'diagnostics',
        hasChildren: false,
        expanded: false,
      },
    ],
  },
])
const sampleDirectory = { id: 'directory', path: '/workspace', label: 'Workspace' }
const gitFiles = ['alpha.ts', 'beta.ts', 'gamma.ts'].map((path) => ({
  path,
  index: ' ',
  workingTree: 'M',
  status: ' M',
  code: ' M',
}))
function toggleGit(path, event) {
  if (event.shiftKey && state.gitAnchor) {
    const a = gitFiles.findIndex((f) => f.path === state.gitAnchor),
      b = gitFiles.findIndex((f) => f.path === path)
    state.gitSelected = gitFiles.slice(Math.min(a, b), Math.max(a, b) + 1).map((f) => f.path)
  } else {
    state.gitAnchor = path
    state.gitSelected = state.gitSelected.includes(path)
      ? state.gitSelected.filter((item) => item !== path)
      : [...state.gitSelected, path]
  }
}
const commits = [
  { hash: 'aaa111', subject: 'Alpha commit', authorName: 'Fixture', authorDate: '' },
  { hash: 'bbb222', subject: 'Beta commit', authorName: 'Fixture', authorDate: '' },
]
const attachmentTrigger = ref(null),
  transcriptRoot = ref(null),
  composer = ref(null)
const messages = computed(() =>
  [...(state.older ? ['older'] : []), 'one', 'two', 'three'].map((id, index) => ({
    info: { id, role: index % 2 ? 'assistant' : 'user' },
    parts: [{ id: 'p-' + id, type: 'text', text: 'Message body ' + id }],
  })),
)
const partsFor = (message) =>
  message.parts.map((source) => ({
    key: source.id,
    id: source.id,
    kind: message.info.role === 'user' ? 'text' : 'answer',
    role: message.info.role,
    source,
    title: '',
    summary: '',
    copyText: source.text,
    status: '',
    toggleable: false,
    defaultExpanded: true,
  }))
const commands = ['alpha', 'beta'].map((name) => ({ name, description: name + ' command', kind: 'builtin' }))
function paletteKey(event) {
  if (['ArrowUp', 'ArrowDown'].includes(event.key)) {
    event.preventDefault()
    state.commandIndex = state.commandIndex ? 0 : 1
  }
  if (event.key === 'Escape') {
    event.preventDefault()
    state.palette = false
  }
}
const historyEntries = computed(() =>
  Array.from({ length: 40 }, (_, i) => `Prompt ${i + 1}`).filter((text) =>
    text.toLowerCase().includes(state.historyQuery.toLowerCase()),
  ),
)
function updateHistoryQuery(value) {
  state.historyQuery = value
  state.historyIndex = 0
}
function openCollectionForm(id) {
  state.deleteTarget = id
  state.collectionDialog = true
}
function removeSourceRow() {
  deleteRow(state.deleteTarget)
  state.collectionDialog = false
}
function historyKey(event) {
  if (event.isComposing || event.defaultPrevented) return
  if (['ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) {
    event.preventDefault()
    const last = historyEntries.value.length - 1
    state.historyIndex =
      event.key === 'Home'
        ? 0
        : event.key === 'End'
          ? last
          : Math.max(0, Math.min(last, state.historyIndex + (event.key === 'ArrowDown' ? 1 : -1)))
  } else if (event.key === 'Escape') {
    event.preventDefault()
    state.historyOpen = false
  } else if (event.key === 'Enter') {
    event.preventDefault()
    state.historySelected = historyEntries.value[state.historyIndex]
    state.historyOpen = false
  }
}
function focusTranscriptInput() {
  document.getElementById('transcript-input')?.focus()
}
function deferred() {
  state.deferredReady = false
  state.deferredMounted = true
  deferKeyboardFocus(() => (state.deferredReady ? document.querySelector('#deferred-target') : null))
  setTimeout(() => {
    state.deferredReady = true
  }, 120)
}
async function routeNavigate() {
  await router.push('/settings/interface')
  state.routeSource = false
  setTimeout(() => {
    state.routeReady = true
  }, 120)
}
function deleteRow(id) {
  state.deletionRows = state.deletionRows.filter((item) => item !== id)
}
let dispose
onMounted(() => {
  dispose = installKeyboardShortcuts()
})
onBeforeUnmount(() => dispose?.())
window.keyboardWorkflows = { state, ui, preview, directory, tree, router, deferred, alpha }
</script>

<template>
  <main class="min-h-screen space-y-5 overflow-auto bg-background p-4 text-foreground">
    <nav data-keyboard-region tabindex="-1">
      <button id="help-workflows" @click="ui.isHelpDialogOpen = true">Help</button
      ><button id="outside">Outside control</button>
    </nav>
    <section id="workspace-fixture" class="flex h-36 gap-2">
      <WorkspaceEditorGroupPane
        v-for="group in ui.workspaceGroups"
        :key="group.id"
        :group-id="group.id"
        class="min-w-0 flex-1"
      />
    </section>
    <section id="files-fixture" class="h-80 w-[420px] max-w-full border border-border">
      <FilesExplorerPane v-bind="filesProps" />
    </section>
    <FilesMoveDialog
      v-model:open="state.moveOpen"
      v-model="state.moveInput"
      :count="state.moved.length"
      :error="state.moveError"
      :submitting="false"
      @submit="state.moveFails ? (state.moveError = 'Destination already exists') : (state.moveOpen = false)"
    />
    <section id="settings-fixture" class="w-96">
      <SettingsSidebarNavList
        :groups="settingGroups"
        @activate-row="
          (row) => (row.hasChildren ? (state.settingsExpanded = !state.settingsExpanded) : (state.setting = row.key))
        "
      />
    </section>
    <section id="session-fixture" v-keyboard-collection="{ tree: true }" aria-label="Sessions" class="w-[420px]">
      <DirectoryRow
        :directory="sampleDirectory"
        :ui-is-compact-layout="false"
        :collapsed="state.directoryCollapsed"
        @toggle-collapse="state.directoryCollapsed = !state.directoryCollapsed"
        @row-click="state.sessionAction = 'directory'"
        @open-context-menu="state.sessionAction = 'directory-menu'"
      />
      <template v-if="!state.directoryCollapsed">
        <SessionRow
          session-id="parent"
          :session="{ id: 'parent', title: 'Alpha session' }"
          :directory="sampleDirectory"
          :ui-is-compact-layout="false"
          :indent-px="2"
          :is-parent="true"
          :is-expanded="state.threadExpanded"
          @toggle-thread="state.threadExpanded = !state.threadExpanded"
          @open="state.sessionSelected = 'parent'"
          @open-actions="state.sessionAction = 'parent-menu'"
          @open-action-menu="state.sessionAction = 'parent-menu'"
        />
        <SessionRow
          v-if="state.threadExpanded"
          session-id="child"
          :session="{ id: 'child', title: 'Child session' }"
          :directory="sampleDirectory"
          :ui-is-compact-layout="false"
          :indent-px="12"
          @open="state.sessionSelected = 'child'"
        />
        <SessionRow
          session-id="last"
          :session="{ id: 'last', title: 'Zeta session' }"
          :directory="sampleDirectory"
          :ui-is-compact-layout="false"
          :indent-px="2"
          @open="state.sessionSelected = 'last'"
        />
      </template>
    </section>
    <section id="git-fixture" class="w-[420px]">
      <GitChangesSection
        :expanded="state.gitExpanded"
        :count="gitFiles.length"
        :files="gitFiles"
        :selected-file="state.openedFile"
        diff-source="working"
        :has-more="false"
        :loading="false"
        :multi-select-mode="true"
        :selected-paths="state.gitSelected"
        @update:expanded="state.gitExpanded = $event"
        @toggle-select="toggleGit"
        @select="state.openedFile = $event"
        @stage="state.gitStaged.push($event)"
      />
    </section>
    <section id="history-fixture" class="w-[420px]">
      <GitHistorySection
        :expanded="true"
        :commits="commits"
        :loading="false"
        :error="null"
        :has-more="false"
        :selected-hash="state.selectedCommit"
        @select="state.selectedCommit = $event.hash"
      />
    </section>
    <section>
      <button id="attachment-trigger" ref="attachmentTrigger" @click="state.attachmentsOpen = true">Attachments</button>
    </section>
    <AttachmentsPanel
      :open="state.attachmentsOpen"
      :attached-files="state.attached"
      :format-bytes="(n) => n + ' bytes'"
      :desktop-anchor-el="attachmentTrigger"
      :is-compact-touch="state.compact"
      @update:open="state.attachmentsOpen = $event"
      @attach-project="state.attachProjectOpen = true"
      @remove="state.attached = state.attached.filter((file) => file.id !== $event)"
    />
    <FormDialog
      :open="state.attachProjectOpen"
      title="Project attachment"
      @update:open="state.attachProjectOpen = $event"
      ><input aria-label="Project file"
    /></FormDialog>
    <ImageViewerDialog />
    <section id="transcript-fixture" ref="transcriptRoot" class="w-[700px] max-w-full">
      <TranscriptKeyboardToolbar
        :root="transcriptRoot"
        :has-older="!state.older"
        :load-older="
          async () => {
            state.older = true
            return true
          }
        "
        :plan-available="true"
        @copy="state.copied = $event"
        @search="state.searched++"
        @input="focusTranscriptInput"
        @plan="state.plan = true"
      />
      <div class="h-64 overflow-auto">
        <MessageItem
          v-for="message in messages"
          :key="message.info.id"
          :message="message"
          :display-parts="partsFor(message)"
          :show-timestamps="false"
          :format-time="() => ''"
          copied-message-id=""
          revert-busy-message-id=""
          :is-streaming="false"
          :collapse-signal="0"
          :activity-page-size="12"
          :is-compact-touch="false"
          :is-part-expanded="() => true"
        />
      </div>
      <textarea id="transcript-input" aria-label="Transcript input" />
    </section>
    <section id="preview-fixture" class="h-[530px] border border-border">
      <WorkspacePreviewDockPanel :viewer-mode="state.viewer" controls-variant="viewport" />
    </section>
    <section id="commands-fixture" class="relative w-[500px] max-w-full pt-80">
      <button @click="state.palette = true">Commands</button>
      <Composer
        ref="composer"
        v-model:draft="state.draft"
        :fullscreen="false"
        :attached-files="[]"
        :pending-attachments="[]"
        :command-open="state.palette"
        command-list-id="fixture-command-list"
        :command-active-id="'fixture-command-list-' + state.commandIndex"
        @draft-keydown="paletteKey"
      >
        <template #overlay
          ><CommandPalette
            :open="state.palette"
            :auto-focus="false"
            :loading="false"
            query=""
            :commands="commands"
            :active-index="state.commandIndex"
            :command-icon="() => RiTerminalBoxLine"
            list-id="fixture-command-list"
            @keydown="paletteKey"
            @dismiss="state.palette = false"
        /></template>
      </Composer>
    </section>
    <section id="history-palette-fixture" class="composer-shell relative w-[500px] max-w-full pt-80">
      <textarea id="history-editor" data-chat-input="true" aria-label="History composer" />
      <button id="history-trigger" @click="state.historyOpen = true">Prompt history</button>
      <PromptHistoryPalette
        :open="state.historyOpen"
        :auto-focus="true"
        :query="state.historyQuery"
        :entries="historyEntries"
        :active-index="state.historyIndex"
        @update:query="updateHistoryQuery"
        @update:active-index="state.historyIndex = $event"
        @keydown="historyKey"
        @dismiss="state.historyOpen = false"
      />
      <button id="history-after">After history</button>
    </section>
    <section id="deferred-fixture">
      <button id="deferred-trigger" @click="deferred">Load page</button
      ><button id="deferred-other">Other destination</button
      ><input
        v-if="state.deferredMounted"
        v-show="state.deferredReady"
        id="deferred-target"
        aria-label="Loaded page input"
      />
    </section>
    <section id="route-fixture">
      <button v-if="state.routeSource" id="route-source" @click="routeNavigate">Navigate to settings</button>
      <section
        v-else
        id="route-destination"
        data-workspace-pane-window="route-fixture-pane"
        data-keyboard-region
        tabindex="-1"
        data-keyboard-route="/settings/interface"
        :aria-busy="!state.routeReady"
      >
        <input v-if="state.routeReady" aria-label="Route destination" />
      </section>
    </section>
    <section
      id="deletion-fixture"
      v-keyboard-collection
      aria-label="Deletion list"
      class="min-h-20 border border-border"
    >
      <div
        v-for="id in state.deletionRows"
        :key="id"
        :data-keyboard-item="id"
        :data-keyboard-label="id"
        class="flex gap-2"
      >
        <button data-keyboard-primary @click="openCollectionForm(id)">
          {{ id }}
        </button>
        <ConfirmPopover
          title="Delete row?"
          confirm-text="Confirm delete"
          cancel-text="Cancel delete"
          @confirm="deleteRow(id)"
          ><button>Delete {{ id }}</button></ConfirmPopover
        >
      </div>
    </section>
    <FormDialog :open="state.collectionDialog" title="Collection form" @update:open="state.collectionDialog = $event"
      ><button @click="removeSourceRow">Remove source row</button></FormDialog
    >
    <aside data-keyboard-region data-keyboard-sidebar tabindex="-1">
      <button id="choose-session">Choose a session</button>
    </aside>
    <section
      :data-workspace-pane-window="ui.workspaceWindows.find((window) => window.mainTab === 'chat')?.id"
      v-show="state.composerMounted && ui.activeMainTab === 'chat'"
    >
      <textarea id="fallback-composer" data-chat-input="true" aria-label="Fallback composer" />
    </section>
    <HelpDialog />
  </main>
</template>
